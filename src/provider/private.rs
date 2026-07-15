//! The commit-reveal private-transaction flow — [`RootProvider::send_private`].
//!
//! Pyde's front-running protection is **commit-reveal**: a
//! transaction's ordering position is fixed *before* its contents are
//! visible, with no decryption key anywhere. The sender publishes a
//! salted hash (the *commitment*) whose ordering wave reserves the
//! slot, then opens it with a `Reveal` once the order is finalised.
//!
//! [`RootProvider::send_private`] hides the two round-trips behind a
//! single call that auto-reveals, so it feels like one send. Under the
//! hood it:
//!
//! 1. signs the hidden inner tx (nonce `base + 2`),
//! 2. draws a fresh 32-byte CSPRNG salt and computes the commitment,
//! 3. submits a `Commit` (nonce `base`) and waits for it to finalise,
//! 4. submits the matching `Reveal` (nonce `base + 1`).
//!
//! The inner tx then executes in the reveal wave's resolution pass,
//! **in commit order**, and its receipt is keyed by the inner tx's own
//! hash. [`PrivateSendHandle::await_receipt`] resolves on that receipt.
//!
//! ## What this protects — and what it does not
//!
//! Content-targeted front-running is prevented: no actor can read the
//! transaction before its slot is locked. It is **not** a total
//! ordering lock — the reveal necessarily exposes the contents before
//! the inner tx executes, and an unrelated tx that arrives in the
//! reveal→execute window can still be ordered around it. The
//! resolution pass runs revealed txs in commit order to keep that
//! window minimal.
//!
//! ## Advanced / relay use
//!
//! For splitting the two phases, or revealing on another account's
//! behalf, drop to [`crate::tx::TxBuilder::commit`] /
//! [`crate::tx::TxBuilder::reveal`] plus [`crate::tx::commitment_hash`]
//! and [`crate::tx::required_bond`].

use std::sync::Arc;

use rand::RngCore;

use crate::error::SdkError;
use crate::signer::Signer;
use crate::tx::{commitment_hash, encode, tx_hash, TxBuilder};
use crate::types::{Receipt, Tx, TxHash};

use super::pending::PendingTx;
use super::provider_trait::{Provider, RootProvider};
use super::transport::Transport;

/// Gas limit for the `Commit` tx. The engine charges `COMMIT_GAS`
/// (== `MIN_GAS_LIMIT`, 21,000); this leaves comfortable headroom.
const COMMIT_GAS_LIMIT: u64 = 100_000;

/// Base gas the engine charges a `Reveal` before the per-byte term
/// (mirrors the engine's `REVEAL_BASE_GAS`).
const REVEAL_BASE_GAS: u64 = 21_000;

/// Per-byte gas the engine charges on the revealed inner-tx payload
/// (mirrors the engine's `REVEAL_GAS_PER_BYTE`).
const REVEAL_GAS_PER_BYTE: u64 = 16;

/// Gas limit for a `Reveal` carrying `inner_len` payload bytes, with
/// a flat safety margin over the engine's exact charge.
fn reveal_gas_limit(inner_len: usize) -> u64 {
    let charged =
        REVEAL_BASE_GAS.saturating_add(REVEAL_GAS_PER_BYTE.saturating_mul(inner_len as u64));
    charged.saturating_add(REVEAL_BASE_GAS) // +21,000 margin
}

/// Handle returned by [`RootProvider::send_private`].
///
/// Carries all three transaction hashes from the commit-reveal dance.
/// Most callers only need [`Self::await_receipt`], which resolves on
/// the **inner** tx's receipt — the real outcome of the private send.
#[derive(Clone)]
pub struct PrivateSendHandle {
    commit_hash: TxHash,
    reveal_hash: TxHash,
    inner_hash: TxHash,
    provider: Arc<dyn Provider>,
}

impl PrivateSendHandle {
    /// Hash of the `Commit` tx (phase 1 — reserves the ordered slot).
    #[must_use]
    pub fn commit_hash(&self) -> TxHash {
        self.commit_hash
    }

    /// Hash of the `Reveal` tx (phase 2 — opens the slot).
    #[must_use]
    pub fn reveal_hash(&self) -> TxHash {
        self.reveal_hash
    }

    /// Hash of the hidden **inner** tx — the real transaction. Its
    /// receipt is the one that carries the execution outcome.
    #[must_use]
    pub fn inner_hash(&self) -> TxHash {
        self.inner_hash
    }

    /// Block until the inner tx's receipt is available.
    ///
    /// The inner tx executes in the reveal wave's resolution pass, so
    /// this returns once that pass commits. Uses the default
    /// [`PendingTx`] poll interval + timeout.
    ///
    /// # Errors
    /// - [`SdkError::Timeout`] if the inner tx never commits in time.
    /// - Any error surfaced by the underlying receipt poll.
    pub async fn await_receipt(&self) -> Result<Receipt, SdkError> {
        PendingTx::new(self.inner_hash, self.provider.clone())
            .wait_for_receipt()
            .await
    }
}

impl std::fmt::Debug for PrivateSendHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateSendHandle")
            .field("commit_hash", &self.commit_hash)
            .field("reveal_hash", &self.reveal_hash)
            .field("inner_hash", &self.inner_hash)
            .finish_non_exhaustive()
    }
}

impl<T: Transport + 'static> RootProvider<T> {
    /// Send a transaction privately via the commit-reveal flow, in one
    /// call. The transaction's ordering position is fixed before its
    /// contents are visible — front-running protection with no
    /// decryption key anywhere.
    ///
    /// `inner` is the **unsigned** transaction you want to send (built
    /// with [`crate::tx::TxBuilder`], `from` == `signer.address()`).
    /// This method assigns its nonce, signs it, and drives the full
    /// commit → wait → reveal dance. The returned
    /// [`PrivateSendHandle`] exposes the three tx hashes; call
    /// [`PrivateSendHandle::await_receipt`] for the inner tx's outcome.
    ///
    /// The `signer` commits, reveals, and (for the common single-wallet
    /// case) owns the inner tx. Nonces are assigned from the signer's
    /// account: `commit = base`, `reveal = base + 1`, `inner = base + 2`
    /// — all inside the 16-slot nonce window, consumed in that order.
    ///
    /// The `Commit` posts a refundable bond
    /// ([`crate::tx::required_bond`]` (inner.value)`); the signer must
    /// have that balance free. It is refunded when the reveal is
    /// accepted.
    ///
    /// For relaying (inner tx from a different account, or splitting
    /// the two phases), use the lower-level
    /// [`crate::tx::TxBuilder::commit`] / [`crate::tx::TxBuilder::reveal`]
    /// helpers instead.
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] if `inner.from != signer.address()`
    ///   (use the low-level builders for the relay case).
    /// - Whatever [`Provider::send_raw_transaction`] /
    ///   [`Provider::get_nonce`] surface.
    /// - [`SdkError::Rpc`] if the commit does not commit successfully
    ///   (e.g. insufficient balance for the bond) — the reveal is not
    ///   sent in that case.
    pub async fn send_private<S: Signer + ?Sized>(
        self: &Arc<Self>,
        signer: &S,
        mut inner: Tx,
    ) -> Result<PrivateSendHandle, SdkError> {
        let committer = signer.address();
        if inner.from != committer {
            return Err(SdkError::InvalidArgument(format!(
                "send_private: inner.from ({}) must equal signer address ({}); \
                 use TxBuilder::commit / TxBuilder::reveal for the relay case",
                inner.from, committer
            )));
        }

        // Nonce plan: commit=base, reveal=base+1, inner=base+2. All
        // inside the 16-slot window; consumed commit → reveal → inner.
        let base = self.get_nonce(&committer).await?;
        let chain_id = inner.chain_id;

        // 1. Sign the inner tx at nonce base+2, then encode ONCE and
        //    reuse those exact bytes for both the commitment and the
        //    reveal payload (re-encoding risks a byte drift that would
        //    make the engine's recomputed hash mismatch).
        inner.nonce = base.saturating_add(2);
        signer.sign_tx(&mut inner).await?;
        let inner_bytes = encode(&inner)?;
        let inner_hash = tx_hash(&inner);

        // 2. Fresh CSPRNG salt → commitment.
        let mut salt = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut salt);
        let commitment = commitment_hash(&inner_bytes, &salt);
        let value_ceiling = inner.value;

        // 3. Build + sign + submit the Commit (nonce base).
        let mut commit_tx = TxBuilder::new()
            .from(committer)
            .chain_id(chain_id)
            .nonce(base)
            .commit(commitment, value_ceiling)?
            .gas_limit(COMMIT_GAS_LIMIT)
            .build()?;
        signer.sign_tx(&mut commit_tx).await?;
        let commit_hash = self.send_raw_transaction(&commit_tx).await?;

        // 4. Wait for the commit to finalise before revealing. If it
        //    reverted (e.g. couldn't afford the bond), bail without
        //    revealing.
        let provider: Arc<dyn Provider> = self.clone();
        let commit_receipt = PendingTx::new(commit_hash, provider.clone())
            .wait_for_receipt()
            .await?;
        if !commit_receipt.is_success() {
            return Err(SdkError::Rpc(format!(
                "commit {} did not succeed (status {:?}); reveal not sent",
                commit_receipt.tx_hash, commit_receipt.status
            )));
        }

        // 5. Build + sign + submit the Reveal (nonce base+1) carrying
        //    the exact inner bytes. base+1 accounts for the in-flight
        //    commit that getNonce may not yet reflect.
        let mut reveal_tx = TxBuilder::new()
            .from(committer)
            .chain_id(chain_id)
            .nonce(base.saturating_add(1))
            .reveal(commitment, salt, inner_bytes.clone())?
            .gas_limit(reveal_gas_limit(inner_bytes.len()))
            .build()?;
        signer.sign_tx(&mut reveal_tx).await?;
        let reveal_hash = self.send_raw_transaction(&reveal_tx).await?;

        Ok(PrivateSendHandle {
            commit_hash,
            reveal_hash,
            inner_hash,
            provider,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn reveal_gas_scales_with_payload() {
        // Base + margin for an empty payload.
        assert_eq!(reveal_gas_limit(0), REVEAL_BASE_GAS + REVEAL_BASE_GAS);
        // Grows 16 gas per byte.
        assert_eq!(
            reveal_gas_limit(1000),
            REVEAL_BASE_GAS + REVEAL_GAS_PER_BYTE * 1000 + REVEAL_BASE_GAS
        );
    }
}
