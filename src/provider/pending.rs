//! `PendingTx` — handle returned by `send_raw_transaction`.
//!
//! Wraps the tx hash with a `wait_for_receipt` helper that polls
//! `pyde_getReceipt` until the tx commits (or the deadline elapses).
//! Modeled on alloy's `PendingTransactionBuilder` — single struct,
//! one `await` to reach a settled state.

use std::sync::Arc;
use std::time::Duration;

use crate::error::SdkError;
use crate::types::{Receipt, TxHash};

use super::provider_trait::Provider;

/// Default poll interval when waiting for a receipt — 250 ms.
///
/// At Pyde's ~400 ms wave cadence (Ch 3), 250 ms gives a tight
/// observation loop without flooding the RPC.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Default deadline for [`PendingTx::wait_for_receipt`] — 60 s.
///
/// Generous on Pyde's ~400 ms cadence; cap-and-bail if the tx
/// never lands so the caller can surface a timeout cleanly.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Handle returned by [`Provider::send_raw_transaction`].
///
/// Holds the committed tx hash + a borrow of the provider needed to
/// poll for the receipt. Drop the handle to "fire and forget"; call
/// [`Self::wait_for_receipt`] to block until the chain confirms.
#[derive(Clone)]
pub struct PendingTx {
    hash: TxHash,
    provider: Arc<dyn Provider>,
    poll_interval: Duration,
    timeout: Duration,
}

impl PendingTx {
    /// Construct a pending-tx handle from a known hash and a
    /// provider to poll against.
    #[must_use]
    pub fn new(hash: TxHash, provider: Arc<dyn Provider>) -> Self {
        Self {
            hash,
            provider,
            poll_interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the poll interval.
    #[must_use]
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Override the timeout.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The hash of the submitted tx.
    #[must_use]
    pub fn hash(&self) -> TxHash {
        self.hash
    }

    /// Block until the receipt is available or the deadline elapses.
    ///
    /// Polls `pyde_getTransactionReceipt` at [`Self::with_poll_interval`]
    /// until it returns `Some`. The mempool→commit path on a healthy
    /// devnet is typically 1-3 waves (<1.5 s); the default 60-second
    /// timeout is a generous backstop for slow / saturated networks.
    ///
    /// Note: we poll `pyde_getTransactionReceipt` (the hot state's
    /// receipt map) rather than `pyde_getReceipt` (the consensus
    /// store's long-lived archive). The hot map populates the moment
    /// a tx commits; the consensus store may lag — especially on
    /// devnet, where it can stay empty until a snapshot rolls.
    ///
    /// # Errors
    /// - [`SdkError::Timeout`] when the deadline elapses without a
    ///   receipt.
    /// - Any error surfaced by the underlying
    ///   `get_transaction_receipt` call.
    pub async fn wait_for_receipt(&self) -> Result<Receipt, SdkError> {
        let deadline = std::time::Instant::now() + self.timeout;
        let expected_hex = format!("0x{}", hex::encode(self.hash.as_bytes()));
        loop {
            if let Some(receipt) = self.provider.get_transaction_receipt(&self.hash).await? {
                // Cross-check the receipt's tx_hash matches the
                // hash we polled for. Without this a malicious /
                // racing node could serve a receipt for a different
                // tx and the SDK would happily report success.
                if !receipt
                    .tx_hash
                    .trim_start_matches("0x")
                    .eq_ignore_ascii_case(expected_hex.trim_start_matches("0x"))
                {
                    return Err(SdkError::InvalidResponse(format!(
                        "receipt tx_hash {} doesn't match polled hash {expected_hex}",
                        receipt.tx_hash
                    )));
                }
                return Ok(receipt);
            }
            if std::time::Instant::now() >= deadline {
                return Err(SdkError::Timeout(format!(
                    "tx {expected_hex} did not commit within {:?}",
                    self.timeout
                )));
            }
            tokio::time::sleep(self.poll_interval).await;
        }
    }
}

impl std::fmt::Debug for PendingTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTx")
            .field("hash", &self.hash)
            .field("poll_interval", &self.poll_interval)
            .field("timeout", &self.timeout)
            .finish()
    }
}
