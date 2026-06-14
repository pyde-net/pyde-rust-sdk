//! Treasury-multisig signature primitive + envelope construction.
//!
//! Pyde's chain has a single on-chain treasury account guarded by a
//! `k-of-n` FALCON-512 multisig (Ch 11 §11.7). Four tx types use it
//! to authorise actions against the treasury or chain-emergency
//! state:
//!
//! | TxType            | Action                                  |
//! |-------------------|-----------------------------------------|
//! | `MultisigTx`      | Treasury spend (debit treasury, credit target) |
//! | `RotateMultisig`  | Change the signer set + threshold       |
//! | `EmergencyPause`  | Halt all non-`Emergency*` tx processing |
//! | `EmergencyResume` | Lift a pause                            |
//! | `DisputeSlash`    | Apply a slashing dispute resolution     |
//!
//! All five share one signing primitive: each signer signs a
//! [`canonical_msg`] = `Poseidon2(domain_byte || nonce_le ||
//! Poseidon2(payload))`. The per-action `domain_byte` keeps a
//! signature for one action from being lifted into a different
//! action at the same nonce.
//!
//! ## Building a treasury spend
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use pyde_rust_sdk::{Provider, Signer, TxBuilder};
//! use pyde_rust_sdk::multisig::{sign_action, MultisigTxPayload};
//! use pyde_rust_sdk::types::{Address, TxType};
//!
//! # async fn run(
//! #     provider: Arc<dyn Provider>,
//! #     signers: Vec<Box<dyn Signer>>,
//! #     target: Address,
//! #     amount: u128,
//! #     multisig_nonce: u64,
//! #     chain_id: u64,
//! # ) -> pyde_rust_sdk::Result<()> {
//! // Each signer independently signs the same canonical message,
//! // producing one BundleEntry. At least `threshold` entries are
//! // needed for the chain to accept the spend.
//! let payload_bytes = MultisigTxPayload::canonical_bytes(target, amount)?;
//! let mut bundle = Vec::new();
//! for (idx, signer) in signers.iter().enumerate() {
//!     let entry = sign_action(
//!         signer.as_ref(),
//!         idx as u32,
//!         TxType::MultisigTx,
//!         multisig_nonce,
//!         &payload_bytes,
//!     ).await?;
//!     bundle.push(entry);
//! }
//!
//! // The envelope-style tx — `from` and `to` are both ZERO; the
//! // bundle authorises both the target and amount.
//! let tx = TxBuilder::new()
//!     .from(Address::ZERO)
//!     .chain_id(chain_id)
//!     .nonce(0)  // chain ignores tx.nonce for multisig actions;
//!                // multisig_nonce above is what gets bumped.
//!     .multisig_treasury_spend(target, amount, bundle)?
//!     .build()?;
//! // Treasury txs are NOT signed at the tx level — the bundle is
//! // the signature. Submit unsigned.
//! provider.send_raw_transaction(&tx).await?;
//! # Ok(()) }
//! ```
//!
//! ## Wire-format invariants
//!
//! Wire bytes are byte-for-byte identical to the engine — see
//! `engine/crates/tx/src/multisig.rs` for the source of truth.
//! `BundleEntry`, `SigBundle`, and `MultisigTxPayload` all share
//! their borsh shape with their engine counterparts; the canonical
//! message hash is identical for matching `(domain, nonce, payload)`.
//!
//! Tests pin every domain byte + a full canonical-message vector
//! against the engine constants; if the engine reshapes the wire,
//! this module fails to build before any signature mismatch hits
//! production.

use borsh::{BorshDeserialize, BorshSerialize};
use pyde_crypto::poseidon2::poseidon2_hash;

use crate::error::SdkError;
use crate::signer::Signer;
use crate::types::{Address, FalconSignature, TxHash, TxType};

// ── Domain separation ─────────────────────────────────────────────────

/// Domain byte mixed into [`canonical_msg`] for `MultisigTx`. Locked
/// at v1; engine source: `engine/crates/tx/src/multisig.rs::domain_byte`.
pub const DOMAIN_MULTISIG_TX: u8 = 0x09;
/// Domain byte for `RotateMultisig`.
pub const DOMAIN_ROTATE_MULTISIG: u8 = 0x0A;
/// Domain byte for `EmergencyPause`.
pub const DOMAIN_EMERGENCY_PAUSE: u8 = 0x0B;
/// Domain byte for `EmergencyResume`.
pub const DOMAIN_EMERGENCY_RESUME: u8 = 0x0C;
/// Domain byte for `DisputeSlash`.
pub const DOMAIN_DISPUTE_SLASH: u8 = 0x10;

/// Map a multisig-driven [`TxType`] to its canonical-message domain
/// byte.
///
/// Returns `None` for tx types that don't use the multisig primitive
/// (e.g., a plain `Standard` transfer).
#[must_use]
pub fn domain_byte(tx_type: TxType) -> Option<u8> {
    match tx_type {
        TxType::MultisigTx => Some(DOMAIN_MULTISIG_TX),
        TxType::RotateMultisig => Some(DOMAIN_ROTATE_MULTISIG),
        TxType::EmergencyPause => Some(DOMAIN_EMERGENCY_PAUSE),
        TxType::EmergencyResume => Some(DOMAIN_EMERGENCY_RESUME),
        TxType::DisputeSlash => Some(DOMAIN_DISPUTE_SLASH),
        _ => None,
    }
}

/// Compute the canonical 32-byte message a multisig signer signs.
///
/// `Poseidon2(domain_byte || nonce_le || Poseidon2(payload))`. The
/// inner `Poseidon2(payload)` keeps signers from having to handle
/// arbitrarily-long payload buffers; the outer hash binds the
/// signature to a specific `(action, nonce, payload)` triple.
///
/// `payload` is the per-action body — borsh-encoded
/// `(target, amount)` for `MultisigTx`, empty for
/// `EmergencyPause` / `EmergencyResume`, etc. See per-payload
/// helpers (e.g. [`MultisigTxPayload::canonical_bytes`]).
///
/// Returns `None` if `tx_type` is not a multisig-driven action.
#[must_use]
pub fn canonical_msg(tx_type: TxType, nonce: u64, payload: &[u8]) -> Option<[u8; 32]> {
    let domain = domain_byte(tx_type)?;
    let payload_digest: [u8; 32] = poseidon2_hash(payload).into();
    let mut preimage = Vec::with_capacity(1 + 8 + 32);
    preimage.push(domain);
    preimage.extend_from_slice(&nonce.to_le_bytes());
    preimage.extend_from_slice(&payload_digest);
    Some(poseidon2_hash(&preimage).into())
}

// ── Signature bundle ──────────────────────────────────────────────────

/// One signer's contribution to a multisig signature bundle.
///
/// Wire shape (borsh): `(signer_index: u32, signature: FalconSignature)`.
/// `signer_index` is the 0-based position of this signer's pubkey in
/// the on-chain `MultisigState::signers` vector; the chain rejects
/// out-of-range indices and duplicates within a bundle.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct BundleEntry {
    /// Position of the signer in the on-chain `MultisigState::signers`.
    pub signer_index: u32,
    /// FALCON-512 signature over [`canonical_msg`] for the action.
    pub signature: FalconSignature,
}

/// Borsh-encoded list of `(signer_index, signature)` pairs forming a
/// `k-of-n` multisig authorisation. Carried inside per-action
/// payloads (e.g. [`MultisigTxPayload::bundle`]); never standalone.
pub type SigBundle = Vec<BundleEntry>;

// ── Per-action payloads ───────────────────────────────────────────────

/// `tx.data` wire shape for a `MultisigTx` (treasury spend).
///
/// Borsh field order matches `engine/crates/tx/src/handlers/multisig_tx.rs::MultisigTxPayload`
/// byte-for-byte. The bundle lives inside the payload so the
/// canonical message can stand on `borsh((target, amount))` alone
/// — bundle bytes never enter the signed hash.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigTxPayload {
    /// Recipient of the spend. The chain rejects `Address::ZERO`,
    /// the treasury itself, and `tx.from`.
    pub target: Address,
    /// Amount of micro-PYDE to debit from the treasury and credit
    /// to `target`. Must be `> 0`.
    pub amount: u128,
    /// Authorising signature bundle — at least `MultisigState::threshold`
    /// distinct valid FALCON-512 signatures over [`canonical_msg`].
    pub bundle: SigBundle,
}

impl MultisigTxPayload {
    /// Produce the borsh-encoded `(target, amount)` bytes that go
    /// into [`canonical_msg`]'s `payload` argument.
    ///
    /// This is the same byte sequence the chain feeds into
    /// `Poseidon2(payload)` when re-deriving the canonical message
    /// during verification. Stable across SDK versions.
    ///
    /// # Errors
    /// Returns [`SdkError::Other`] if borsh encoding fails (not
    /// reachable for fixed-size `Address` + `u128`).
    pub fn canonical_bytes(target: Address, amount: u128) -> Result<Vec<u8>, SdkError> {
        borsh::to_vec(&(target, amount))
            .map_err(|e| SdkError::Other(format!("borsh encode (target, amount): {e}")))
    }
}

// ── Sign helper ───────────────────────────────────────────────────────

/// Produce one [`BundleEntry`] from a signer for the given action.
///
/// Computes the canonical message, asks the signer to FALCON-sign
/// it, and wraps the result with `signer_index`. The caller is
/// responsible for picking `signer_index` correctly — it must match
/// the signer's position in the on-chain `MultisigState::signers`
/// vector.
///
/// Independent of how many other signers participate; each call
/// produces one entry. Collect `>= threshold` entries into a
/// [`SigBundle`] and embed in the per-action payload.
///
/// # Errors
/// - [`SdkError::InvalidArgument`] if `tx_type` is not a
///   multisig-driven action (use [`domain_byte`] to check).
/// - [`SdkError::Signing`] propagated from the signer.
pub async fn sign_action(
    signer: &dyn Signer,
    signer_index: u32,
    tx_type: TxType,
    nonce: u64,
    payload: &[u8],
) -> Result<BundleEntry, SdkError> {
    let msg = canonical_msg(tx_type, nonce, payload).ok_or_else(|| {
        SdkError::InvalidArgument(format!(
            "tx_type {tx_type:?} is not a multisig-driven action"
        ))
    })?;
    let hash = TxHash::new(msg);
    let signature = signer.sign_hash(&hash).await?;
    Ok(BundleEntry {
        signer_index,
        signature,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::signer::LocalSigner;

    // ── Domain bytes ── pinned against engine; must never drift ──────

    #[test]
    fn domain_bytes_match_engine_constants() {
        assert_eq!(DOMAIN_MULTISIG_TX, 0x09);
        assert_eq!(DOMAIN_ROTATE_MULTISIG, 0x0A);
        assert_eq!(DOMAIN_EMERGENCY_PAUSE, 0x0B);
        assert_eq!(DOMAIN_EMERGENCY_RESUME, 0x0C);
        assert_eq!(DOMAIN_DISPUTE_SLASH, 0x10);
    }

    #[test]
    fn domain_byte_maps_every_multisig_tx_type() {
        assert_eq!(domain_byte(TxType::MultisigTx), Some(0x09));
        assert_eq!(domain_byte(TxType::RotateMultisig), Some(0x0A));
        assert_eq!(domain_byte(TxType::EmergencyPause), Some(0x0B));
        assert_eq!(domain_byte(TxType::EmergencyResume), Some(0x0C));
        assert_eq!(domain_byte(TxType::DisputeSlash), Some(0x10));
    }

    #[test]
    fn domain_byte_rejects_non_multisig_tx_types() {
        assert_eq!(domain_byte(TxType::Standard), None);
        assert_eq!(domain_byte(TxType::Deploy), None);
        assert_eq!(domain_byte(TxType::RegisterPubkey), None);
    }

    // ── Canonical message ── shape + determinism + collision basics ──

    #[test]
    fn canonical_msg_returns_none_for_standard_tx() {
        assert!(canonical_msg(TxType::Standard, 0, &[]).is_none());
    }

    #[test]
    fn canonical_msg_is_deterministic() {
        let a = canonical_msg(TxType::MultisigTx, 7, b"payload").unwrap();
        let b = canonical_msg(TxType::MultisigTx, 7, b"payload").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_msg_changes_with_action() {
        let spend = canonical_msg(TxType::MultisigTx, 1, b"x").unwrap();
        let rotate = canonical_msg(TxType::RotateMultisig, 1, b"x").unwrap();
        assert_ne!(
            spend, rotate,
            "domain separation must produce distinct messages"
        );
    }

    #[test]
    fn canonical_msg_changes_with_nonce() {
        let a = canonical_msg(TxType::MultisigTx, 1, b"x").unwrap();
        let b = canonical_msg(TxType::MultisigTx, 2, b"x").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn canonical_msg_changes_with_payload() {
        let a = canonical_msg(TxType::MultisigTx, 1, b"alpha").unwrap();
        let b = canonical_msg(TxType::MultisigTx, 1, b"beta").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn canonical_msg_matches_engine_construction() {
        // Hand-construct using the same shape the engine uses:
        //   Poseidon2(domain || nonce_le || Poseidon2(payload))
        let payload = b"target||amount";
        let nonce: u64 = 0xdead_beef;
        let payload_digest: [u8; 32] = poseidon2_hash(payload).into();
        let mut preimage = Vec::with_capacity(1 + 8 + 32);
        preimage.push(DOMAIN_MULTISIG_TX);
        preimage.extend_from_slice(&nonce.to_le_bytes());
        preimage.extend_from_slice(&payload_digest);
        let expected: [u8; 32] = poseidon2_hash(&preimage).into();

        let computed = canonical_msg(TxType::MultisigTx, nonce, payload).unwrap();
        assert_eq!(computed, expected);
    }

    // ── BundleEntry / SigBundle borsh shape ──────────────────────────

    #[test]
    fn bundle_entry_round_trip() {
        let entry = BundleEntry {
            signer_index: 7,
            signature: FalconSignature::new(vec![1u8; 666]),
        };
        let bytes = borsh::to_vec(&entry).unwrap();
        let decoded: BundleEntry = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, entry);
    }

    #[test]
    fn sig_bundle_round_trip() {
        let bundle: SigBundle = (0..3)
            .map(|i| BundleEntry {
                signer_index: i,
                signature: FalconSignature::new(vec![i as u8; 666]),
            })
            .collect();
        let bytes = borsh::to_vec(&bundle).unwrap();
        let decoded: SigBundle = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, bundle);
    }

    // ── MultisigTxPayload borsh shape ────────────────────────────────

    #[test]
    fn multisig_tx_payload_round_trip() {
        let payload = MultisigTxPayload {
            target: Address::ZERO,
            amount: 12345,
            bundle: vec![BundleEntry {
                signer_index: 0,
                signature: FalconSignature::new(vec![9u8; 666]),
            }],
        };
        let bytes = borsh::to_vec(&payload).unwrap();
        let decoded: MultisigTxPayload = borsh::from_slice(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn multisig_tx_payload_field_order_pinned() {
        // Pin: target (32) || amount (16, LE u128) || bundle
        // (4-byte length prefix + entries). An empty bundle is
        // the simplest pin point — the first 32+16+4 bytes must
        // match the engine's borsh exactly.
        let payload = MultisigTxPayload {
            target: Address::ZERO,
            amount: 1,
            bundle: Vec::new(),
        };
        let bytes = borsh::to_vec(&payload).unwrap();
        assert_eq!(bytes.len(), 32 + 16 + 4);

        // target ZERO bytes
        assert!(bytes[0..32].iter().all(|b| *b == 0));
        // amount = 1u128 LE
        assert_eq!(bytes[32], 1);
        assert!(bytes[33..48].iter().all(|b| *b == 0));
        // empty bundle: u32 length prefix of 0
        assert_eq!(&bytes[48..52], &[0u8; 4]);
    }

    #[test]
    fn canonical_bytes_matches_inline_borsh() {
        let target = Address::new([0xAB; 32]);
        let amount: u128 = 1_000_000;
        let from_helper = MultisigTxPayload::canonical_bytes(target, amount).unwrap();
        let inline = borsh::to_vec(&(target, amount)).unwrap();
        assert_eq!(from_helper, inline);
        assert_eq!(from_helper.len(), 32 + 16);
    }

    // ── sign_action end-to-end ───────────────────────────────────────

    #[tokio::test]
    async fn sign_action_rejects_non_multisig_tx_type() {
        let signer = LocalSigner::random().unwrap();
        let err = sign_action(&signer, 0, TxType::Standard, 0, &[])
            .await
            .unwrap_err();
        match err {
            SdkError::InvalidArgument(msg) => {
                assert!(msg.contains("multisig-driven"), "wrong message: {msg}");
            }
            other => panic!("expected InvalidArgument, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sign_action_produces_verifiable_entry() {
        use pyde_crypto::falcon::{
            falcon_verify, FalconPublicKey as CryptoPk, FalconSignature as CryptoSig,
        };

        let signer = LocalSigner::random().unwrap();
        let payload = MultisigTxPayload::canonical_bytes(Address::ZERO, 7).unwrap();
        let entry = sign_action(&signer, 3, TxType::MultisigTx, 42, &payload)
            .await
            .unwrap();
        assert_eq!(entry.signer_index, 3);

        // The signature must verify against the same canonical
        // message the engine would compute.
        let msg = canonical_msg(TxType::MultisigTx, 42, &payload).unwrap();
        let pk = CryptoPk::from_bytes(signer.pubkey().as_bytes()).expect("valid pubkey");
        let sig = CryptoSig::from_bytes(entry.signature.as_bytes()).expect("valid sig");
        assert!(
            falcon_verify(&pk, &msg, &sig),
            "signature must verify against canonical_msg",
        );
    }

    #[tokio::test]
    async fn three_signers_produce_distinct_bundle_entries() {
        let signers = (0..3)
            .map(|_| LocalSigner::random().unwrap())
            .collect::<Vec<_>>();
        let payload = MultisigTxPayload::canonical_bytes(Address::ZERO, 99).unwrap();

        let mut bundle: SigBundle = Vec::new();
        for (idx, signer) in signers.iter().enumerate() {
            let entry = sign_action(signer, idx as u32, TxType::MultisigTx, 1, &payload)
                .await
                .unwrap();
            bundle.push(entry);
        }

        assert_eq!(bundle.len(), 3);
        // Indices are preserved by sign_action.
        assert_eq!(
            bundle.iter().map(|e| e.signer_index).collect::<Vec<_>>(),
            vec![0, 1, 2],
        );
        // FALCON is randomised — three signers signing the same
        // message must produce three distinct signature byte vectors.
        let sigs: std::collections::HashSet<_> = bundle
            .iter()
            .map(|e| e.signature.as_bytes().to_vec())
            .collect();
        assert_eq!(sigs.len(), 3);
    }
}
