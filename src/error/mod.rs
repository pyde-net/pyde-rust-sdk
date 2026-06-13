//! SDK error taxonomy.
//!
//! Every fallible operation in the SDK returns [`Result<T>`] — an alias for
//! `std::result::Result<T, SdkError>`. The variants cluster by *layer*:
//! transport (`Rpc`, `Connection`, `Timeout`), authorization (`Signing`),
//! input validation (`InvalidAddress`, `InvalidArgument`), and contract
//! execution (`Reverted`, `InsufficientBalance`).
//!
//! Future work (T8): align error codes with [`HOST_FN_ABI_SPEC §4`].
//! That spec defines 17 negative `i32` codes returned from host functions
//! (`-1 = ERR_INVALID_INPUT`, `-5 = ERR_FORBIDDEN`, `-17 = ERR_SIGNATURE_INVALID`,
//! etc.). When `Provider::call` surfaces one of those codes from a view
//! call, the SDK will map it to a structured variant rather than wrapping
//! the raw number in `SdkError::Other`.

use thiserror::Error;

/// Errors returned from SDK operations.
///
/// Each variant carries enough context to surface a useful message to a
/// dapp's user. See [`SdkError::code`] for stable, programmatic codes
/// suitable for client-side error branching.
#[derive(Debug, Error)]
pub enum SdkError {
    /// JSON-RPC error returned by the node (method-level failure).
    #[error("RPC error: {0}")]
    Rpc(String),

    /// Transport-layer failure — TCP/TLS handshake, DNS, connection reset.
    #[error("connection error: {0}")]
    Connection(String),

    /// Local signing failure (FALCON keygen, signature emission).
    #[error("signing error: {0}")]
    Signing(String),

    /// A bounded wait elapsed without the expected event (typically used
    /// by `wait_for_receipt` and `await_finalized`).
    #[error("timeout: {0}")]
    Timeout(String),

    /// Transaction reverted on-chain. State changes rolled back; gas is
    /// still charged per [`ReceiptStatus::Reverted`].
    #[error("{}", format_revert(.gas_used, .data))]
    Reverted {
        /// Gas charged before the revert.
        gas_used: u64,
        /// Return data emitted by `pyde::revert(...)` (UTF-8 if possible).
        data: Vec<u8>,
    },

    /// Sender's balance is too low to cover `value + gas_limit × base_fee`.
    /// Surfaced both at mempool admission and at execute-time.
    #[error("insufficient balance: need {required}, have {available}")]
    InsufficientBalance {
        /// Minimum required (value + gas reservation).
        required: u128,
        /// Sender's actual balance.
        available: u128,
    },

    /// Caller passed a malformed address (wrong length, bad hex, or
    /// reserved sentinel).
    #[error("invalid address: {0}")]
    InvalidAddress(String),

    /// Caller passed an argument that failed pre-flight validation.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Node returned a response the SDK could not deserialize (typically
    /// indicates a node-side bug or version skew).
    #[error("invalid response: {0}")]
    InvalidResponse(String),

    /// Catch-all for errors that don't fit a structured variant.
    /// Prefer adding a new variant over leaning on this.
    #[error("{0}")]
    Other(String),
}

impl SdkError {
    /// Stable programmatic code string suitable for client-side branching.
    ///
    /// These mirror ethers-rs / alloy conventions so dapp devs can reuse
    /// existing error-handling habits. See module docs for the planned
    /// alignment with [`HOST_FN_ABI_SPEC §4`].
    pub fn code(&self) -> &'static str {
        match self {
            SdkError::Rpc(_) => "RPC_ERROR",
            SdkError::Connection(_) => "CONNECTION_ERROR",
            SdkError::Signing(_) => "SIGNING_ERROR",
            SdkError::Timeout(_) => "TIMEOUT",
            SdkError::Reverted { .. } => "CALL_EXCEPTION",
            SdkError::InsufficientBalance { .. } => "INSUFFICIENT_FUNDS",
            SdkError::InvalidAddress(_) => "INVALID_ARGUMENT",
            SdkError::InvalidArgument(_) => "INVALID_ARGUMENT",
            SdkError::InvalidResponse(_) => "INVALID_RESPONSE",
            SdkError::Other(_) => "OTHER",
        }
    }

    /// For revert errors, attempt to decode the human-readable reason
    /// the contract author passed to `pyde::revert(...)`. Returns `None`
    /// for non-revert errors or if the revert data isn't a recognizable
    /// length-prefixed UTF-8 string.
    pub fn revert_reason(&self) -> Option<String> {
        if let SdkError::Reverted { data, .. } = self {
            decode_revert_reason(data)
        } else {
            None
        }
    }

    /// Convenience: `true` iff this error is a contract revert.
    pub fn is_revert(&self) -> bool {
        matches!(self, SdkError::Reverted { .. })
    }
}

/// SDK-wide result alias. Every fallible function in the public API
/// returns this.
pub type Result<T> = std::result::Result<T, SdkError>;

/// Attempt to decode a revert reason from raw return data.
///
/// Recognizes two encodings:
///
/// 1. **Length-prefixed**: `[len:u64-LE][utf8 bytes]` (the canonical
///    encoding emitted by `pyde::revert(reason)` after Borsh string
///    encoding).
/// 2. **Raw UTF-8**: fallback for contracts that emit revert reasons
///    without the length prefix.
///
/// Returns `None` if neither encoding produces a clean UTF-8 string.
fn decode_revert_reason(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    // Try length-prefixed string: [len:8 LE][utf8 bytes]
    if data.len() >= 8 {
        let len = u64::from_le_bytes(data[..8].try_into().ok()?) as usize;
        if len > 0 && len <= data.len() - 8 {
            if let Ok(s) = std::str::from_utf8(&data[8..8 + len]) {
                if s.chars().all(|c| !c.is_control() || c == '\n') {
                    return Some(s.to_string());
                }
            }
        }
    }
    // Try raw UTF-8
    if let Ok(s) = std::str::from_utf8(data) {
        if s.len() <= 256 && s.chars().all(|c| !c.is_control() || c == '\n') {
            return Some(s.to_string());
        }
    }
    None
}

/// Format a revert error's display text.
fn format_revert(gas_used: &u64, data: &[u8]) -> String {
    if let Some(reason) = decode_revert_reason(data) {
        format!("transaction reverted: {} (gas={})", reason, gas_used)
    } else {
        format!("transaction reverted (gas={})", gas_used)
    }
}
