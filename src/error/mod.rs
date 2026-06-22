//! SDK error taxonomy.
//!
//! Every fallible operation in the SDK returns [`Result<T>`] — an
//! alias for `std::result::Result<T, SdkError>`. The variants
//! cluster by *layer*: transport ([`SdkError::Rpc`],
//! [`SdkError::Connection`], [`SdkError::Timeout`]), authorization
//! ([`SdkError::Signing`]), input validation
//! ([`SdkError::InvalidAddress`], [`SdkError::InvalidArgument`]),
//! and contract execution ([`SdkError::Reverted`],
//! [`SdkError::InsufficientBalance`], [`SdkError::HostFn`]).
//!
//! [`SdkError::error_code`] scans the revert payload for a
//! [HOST_FN_ABI §4][spec] error code so callers can branch on
//! the typed reason (e.g. `ERR_FORBIDDEN`, `ERR_INSUFFICIENT_BALANCE`)
//! without parsing strings by hand.
//!
//! For revert errors, [`SdkError::revert_category`] surfaces the
//! engine's [`crate::types::RevertCategory`] (`EngineValidation` /
//! `Contract` / `Vm`) so callers can branch on the *layer* that
//! rejected the tx without parsing the message. The convenience
//! predicates [`SdkError::is_engine_validation_revert`],
//! [`SdkError::is_contract_revert`], and [`SdkError::is_vm_trap`]
//! cover the common cases. [`SdkError::from_receipt`] converts a
//! non-success [`crate::types::Receipt`] into the matching
//! `SdkError` variant in one call.
//!
//! [spec]: https://book.pyde.network/companion/HOST_FN_ABI_SPEC#4-error-codes

use thiserror::Error;

use crate::types::{ErrorCode, Receipt, RevertCategory, RevertReason};

/// Errors returned from SDK operations.
///
/// Each variant carries enough context to surface a useful message
/// to a dapp's user. See [`SdkError::code`] for stable, programmatic
/// codes suitable for client-side error branching.
#[derive(Debug, Error)]
pub enum SdkError {
    /// JSON-RPC error returned by the node (method-level failure).
    #[error("RPC error: {0}")]
    Rpc(String),

    /// Transport-layer failure — TCP/TLS handshake, DNS, connection
    /// reset.
    #[error("connection error: {0}")]
    Connection(String),

    /// Local signing failure (FALCON keygen, signature emission).
    #[error("signing error: {0}")]
    Signing(String),

    /// A bounded wait elapsed without the expected event (typically
    /// used by `wait_for_receipt` and `await_finalized`).
    #[error("timeout: {0}")]
    Timeout(String),

    /// Transaction reverted on-chain. State changes rolled back;
    /// gas is still charged per
    /// [`crate::types::ReceiptStatus::Reverted`].
    ///
    /// `reason` carries the engine's structured revert reason
    /// when available — `Some` for any
    /// `status: "reverted"` receipt when the engine emits a structured reason, `None` when the SDK derived the revert
    /// from `return_data` alone (older nodes / direct constructions).
    /// Branch on [`Self::revert_category`] for control-flow
    /// decisions; use `reason.message` for display.
    #[error("{}", format_revert(.gas_used, .data, .reason.as_ref()))]
    Reverted {
        /// Gas charged before the revert.
        gas_used: u64,
        /// Return data emitted by `pyde::revert(...)` (UTF-8 if
        /// possible).
        data: Vec<u8>,
        /// Structured revert reason from the engine when present.
        /// `None` when the SDK only has the raw `data` bytes — call
        /// [`Self::revert_reason`] to decode from `data`.
        reason: Option<RevertReason>,
    },

    /// Sender's balance is too low to cover
    /// `value + gas_limit × base_fee`. Surfaced both at mempool
    /// admission and at execute-time.
    #[error("insufficient balance: need {required}, have {available}")]
    InsufficientBalance {
        /// Minimum required (value + gas reservation).
        required: u128,
        /// Sender's actual balance.
        available: u128,
    },

    /// A typed [HOST_FN_ABI §4][spec] error code surfaced from a
    /// host function or a cross-contract call. Use when the SDK
    /// can extract a specific `ERR_*` code rather than collapsing
    /// to the generic [`SdkError::Reverted`] / [`SdkError::Rpc`].
    ///
    /// [spec]: https://book.pyde.network/companion/HOST_FN_ABI_SPEC#4-error-codes
    #[error("{code}: {message}")]
    HostFn {
        /// The structured §4 code.
        code: ErrorCode,
        /// Caller-visible context the SDK / node attached.
        message: String,
    },

    /// Caller passed a malformed address (wrong length, bad hex, or
    /// reserved sentinel).
    #[error("invalid address: {0}")]
    InvalidAddress(String),

    /// Caller passed an argument that failed pre-flight validation.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// Node returned a response the SDK could not deserialize
    /// (typically indicates a node-side bug or version skew).
    #[error("invalid response: {0}")]
    InvalidResponse(String),

    /// Catch-all for errors that don't fit a structured variant.
    /// Prefer adding a new variant over leaning on this.
    #[error("{0}")]
    Other(String),
}

impl SdkError {
    /// Stable programmatic code string suitable for client-side
    /// branching.
    ///
    /// These mirror ethers-rs / alloy conventions so dapp devs can
    /// reuse existing error-handling habits.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            SdkError::Rpc(_) => "RPC_ERROR",
            SdkError::Connection(_) => "CONNECTION_ERROR",
            SdkError::Signing(_) => "SIGNING_ERROR",
            SdkError::Timeout(_) => "TIMEOUT",
            SdkError::Reverted { .. } => "CALL_EXCEPTION",
            SdkError::InsufficientBalance { .. } => "INSUFFICIENT_FUNDS",
            SdkError::HostFn { code, .. } => code.name(),
            SdkError::InvalidAddress(_) => "INVALID_ARGUMENT",
            SdkError::InvalidArgument(_) => "INVALID_ARGUMENT",
            SdkError::InvalidResponse(_) => "INVALID_RESPONSE",
            SdkError::Other(_) => "OTHER",
        }
    }

    /// For revert errors, attempt to decode the human-readable
    /// reason the contract author passed to `pyde::revert(...)`.
    /// Returns `None` for non-revert errors or if the revert data
    /// isn't a recognizable encoding.
    ///
    /// Recognizes three encodings (probed in order):
    ///
    /// 1. **Borsh-encoded `String`** — `[len:u32_le][utf8 bytes]`
    ///    (what the `#[pyde::entry]` macro emits when an entry
    ///    panics or calls `pyde::revert`).
    /// 2. **Length-prefixed `u64`** — `[len:u64_le][utf8 bytes]`
    ///    (legacy emitter shape some hand-written contracts still
    ///    use).
    /// 3. **Raw UTF-8** — fallback for contracts that emit revert
    ///    reasons without any length prefix.
    #[must_use]
    pub fn revert_reason(&self) -> Option<String> {
        if let SdkError::Reverted { data, .. } = self {
            decode_revert_reason(data)
        } else {
            None
        }
    }

    /// For revert errors, scan the revert payload for a
    /// [HOST_FN_ABI §4](crate::types::error_code) code.
    ///
    /// Two extraction paths:
    ///
    /// 1. **Named token** — the message contains an exact
    ///    `ERR_*` token (e.g. `"ERR_FORBIDDEN"`). The engine's
    ///    `pyde::revert` helpers and the otigen runtime emit this
    ///    shape.
    /// 2. **Negative integer** — the message contains a parseable
    ///    negative integer in the spec table (`"-5"`, `"code: -10"`).
    ///
    /// Returns `None` for non-revert errors or revert payloads
    /// that don't contain a recognizable code.
    #[must_use]
    pub fn error_code(&self) -> Option<ErrorCode> {
        match self {
            SdkError::HostFn { code, .. } => Some(*code),
            SdkError::Reverted { data, .. } => extract_error_code(data),
            _ => None,
        }
    }

    /// Convenience: `true` iff this error is a contract revert.
    #[must_use]
    pub fn is_revert(&self) -> bool {
        matches!(self, SdkError::Reverted { .. })
    }

    /// Structured revert category populated by the engine. Returns
    /// `None` for non-revert errors or revert errors constructed
    /// from a receipt that didn't carry a structured reason.
    ///
    /// Branch on this — not [`Self::revert_reason`] — for
    /// control flow:
    ///
    /// ```ignore
    /// use pyde_rust_sdk::types::RevertCategory;
    ///
    /// match err.revert_category() {
    ///     Some(RevertCategory::EngineValidation) => /* nonce/balance/etc */,
    ///     Some(RevertCategory::Contract)         => /* user's contract said no */,
    ///     Some(RevertCategory::Vm)               => /* trap / OOM / OOG */,
    ///     _                                      => /* not a revert, or older engine */,
    /// }
    /// ```
    #[must_use]
    pub fn revert_category(&self) -> Option<&RevertCategory> {
        match self {
            SdkError::Reverted { reason, .. } => reason.as_ref().map(|r| &r.category),
            _ => None,
        }
    }

    /// `true` iff this revert error carries
    /// `category == EngineValidation` (nonce, balance, fee,
    /// access-list, dispatch decode).
    #[must_use]
    pub fn is_engine_validation_revert(&self) -> bool {
        matches!(
            self.revert_category(),
            Some(RevertCategory::EngineValidation)
        )
    }

    /// `true` iff this revert error carries `category == Contract`
    /// (contract code called `revert(msg)`).
    #[must_use]
    pub fn is_contract_revert(&self) -> bool {
        matches!(self.revert_category(), Some(RevertCategory::Contract))
    }

    /// `true` iff this revert error carries `category == Vm`
    /// (wasmtime trap, OOB memory, executor-side gas exhaustion).
    #[must_use]
    pub fn is_vm_trap(&self) -> bool {
        matches!(self.revert_category(), Some(RevertCategory::Vm))
    }

    /// Construct an [`SdkError`] from a [`Receipt`] whose
    /// `status != Success`. Returns `None` for success
    /// receipts (which aren't errors).
    ///
    /// Out-of-gas → [`SdkError::Reverted`] with synthesised
    /// reason `Some({category: Vm, message: "out of gas"})`.
    /// Reverted with structured reason → carried through.
    /// Reverted without structured reason → bytes-only.
    #[must_use]
    pub fn from_receipt(receipt: &Receipt) -> Option<Self> {
        use crate::types::ReceiptStatus;
        let gas_used = receipt.try_gas().unwrap_or(0);
        let data = receipt.try_return_bytes().unwrap_or_default();
        match receipt.status {
            ReceiptStatus::Success => None,
            ReceiptStatus::OutOfGas => Some(SdkError::Reverted {
                gas_used,
                data,
                reason: Some(RevertReason {
                    category: RevertCategory::Vm,
                    message: "out of gas".to_string(),
                }),
            }),
            ReceiptStatus::Reverted => Some(SdkError::Reverted {
                gas_used,
                data,
                reason: receipt.revert_reason.clone(),
            }),
        }
    }
}

/// SDK-wide result alias. Every fallible function in the public
/// API returns this.
pub type Result<T> = std::result::Result<T, SdkError>;

/// Attempt to decode a revert reason from raw return data.
///
/// See [`SdkError::revert_reason`] for the encoding table.
fn decode_revert_reason(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    // 1. Borsh-encoded String: [len:4 LE][utf8 bytes]
    if data.len() >= 4 {
        let len = u32::from_le_bytes(data[..4].try_into().ok()?) as usize;
        if len > 0 && len <= data.len() - 4 {
            if let Ok(s) = std::str::from_utf8(&data[4..4 + len]) {
                if looks_printable(s) {
                    return Some(s.to_string());
                }
            }
        }
    }
    // 2. Length-prefixed u64: [len:8 LE][utf8 bytes]
    if data.len() >= 8 {
        let len = u64::from_le_bytes(data[..8].try_into().ok()?) as usize;
        if len > 0 && len <= data.len() - 8 {
            if let Ok(s) = std::str::from_utf8(&data[8..8 + len]) {
                if looks_printable(s) {
                    return Some(s.to_string());
                }
            }
        }
    }
    // 3. Raw UTF-8 fallback.
    if let Ok(s) = std::str::from_utf8(data) {
        if s.len() <= 256 && looks_printable(s) {
            return Some(s.to_string());
        }
    }
    None
}

/// `true` if every character is printable (no control codes except
/// newline). Filters out random binary that happens to decode as
/// UTF-8 but is almost certainly not a human revert message.
fn looks_printable(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| !c.is_control() || c == '\n')
}

/// Scan a revert payload for a HOST_FN_ABI §4 error code.
///
/// Two parses tried in order: named token (`ERR_FORBIDDEN`) and
/// negative-integer parse.
fn extract_error_code(data: &[u8]) -> Option<ErrorCode> {
    let reason = decode_revert_reason(data)?;
    extract_error_code_from_str(&reason)
}

fn extract_error_code_from_str(reason: &str) -> Option<ErrorCode> {
    // Named-token scan. We check the longest names first to avoid
    // accidentally matching a prefix (e.g. `ERR_CROSS_CALL_FAILED`
    // vs `ERR_CROSS_CALL_OUT_OF_GAS`).
    let codes = [
        ErrorCode::AccessListViolation,
        ErrorCode::CiphertextInvalid,
        ErrorCode::CrossCallFailed,
        ErrorCode::CrossCallOutOfGas,
        ErrorCode::Forbidden,
        ErrorCode::InsufficientBalance,
        ErrorCode::Internal,
        ErrorCode::InvalidAddress,
        ErrorCode::InvalidFunctionName,
        ErrorCode::InvalidInput,
        ErrorCode::NotFound,
        ErrorCode::OutOfGas,
        ErrorCode::OutputBufferTooSmall,
        ErrorCode::ParachainOnly,
        ErrorCode::ReentrancyBlocked,
        ErrorCode::SignatureInvalid,
        ErrorCode::ValueTransferNotPayable,
        ErrorCode::XCallRateLimited,
    ];
    // Sort by name length descending so longer matches win.
    let mut sorted = codes;
    sorted.sort_by_key(|c| std::cmp::Reverse(c.name().len()));
    for code in sorted {
        if reason.contains(code.name()) {
            return Some(code);
        }
    }
    // Integer-parse fallback: scan whitespace-separated tokens for
    // a parseable `-<n>` that resolves under ErrorCode::from_i32.
    for token in reason
        .split(|c: char| !c.is_ascii_digit() && c != '-')
        .filter(|t| !t.is_empty())
    {
        if let Ok(n) = token.parse::<i32>() {
            if let Some(code) = ErrorCode::from_i32(n) {
                return Some(code);
            }
        }
    }
    None
}

/// Format a revert error's display text. Prefers the engine's
/// structured `reason` when present; falls back to decoding `data` bytes otherwise.
fn format_revert(gas_used: &u64, data: &[u8], reason: Option<&RevertReason>) -> String {
    if let Some(r) = reason {
        return format!(
            "transaction reverted [{:?}]: {} (gas={gas_used})",
            r.category, r.message
        );
    }
    if let Some(reason) = decode_revert_reason(data) {
        format!("transaction reverted: {reason} (gas={gas_used})")
    } else {
        format!("transaction reverted (gas={gas_used})")
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // ── decode_revert_reason ──────────────────────────────────

    #[test]
    fn revert_reason_empty_returns_none() {
        assert!(decode_revert_reason(&[]).is_none());
    }

    #[test]
    fn revert_reason_borsh_u32_string() {
        // Borsh string: [len:u32_le][utf8]
        let msg = "insufficient balance";
        let mut payload = (msg.len() as u32).to_le_bytes().to_vec();
        payload.extend_from_slice(msg.as_bytes());
        assert_eq!(decode_revert_reason(&payload).as_deref(), Some(msg));
    }

    #[test]
    fn revert_reason_legacy_u64_string() {
        // Legacy: [len:u64_le][utf8]. To force the u64 path (not
        // u32), use a length that wouldn't fit the u32 interpretation.
        let msg = "legacy revert";
        let bogus_u32 = u32::MAX.to_le_bytes(); // makes Borsh u32 reject
        let mut payload = bogus_u32.to_vec();
        payload.extend_from_slice(&(msg.len() as u32).to_le_bytes());
        payload.extend_from_slice(msg.as_bytes());
        // The u32 path bails on length > data.len()-4 → falls
        // through to u64 which reads the full 8-byte prefix.
        let len_u64 = msg.len() as u64;
        let mut alt = len_u64.to_le_bytes().to_vec();
        alt.extend_from_slice(msg.as_bytes());
        assert_eq!(decode_revert_reason(&alt).as_deref(), Some(msg));
    }

    #[test]
    fn revert_reason_raw_utf8_fallback() {
        let msg = "bare utf8 message";
        assert_eq!(decode_revert_reason(msg.as_bytes()).as_deref(), Some(msg));
    }

    #[test]
    fn revert_reason_rejects_binary_garbage() {
        // High-bit bytes that aren't valid UTF-8 → None.
        let garbage = [0xFFu8, 0xFE, 0xFD, 0xFC];
        assert!(decode_revert_reason(&garbage).is_none());
    }

    #[test]
    fn revert_reason_rejects_long_random_utf8() {
        // > 256 bytes of valid UTF-8 still rejected (likely binary
        // that happens to decode).
        let long = "x".repeat(300);
        assert!(decode_revert_reason(long.as_bytes()).is_none());
    }

    #[test]
    fn revert_reason_rejects_control_chars() {
        let with_null = b"abc\x00def";
        assert!(decode_revert_reason(with_null).is_none());
    }

    // ── extract_error_code ────────────────────────────────────

    #[test]
    fn error_code_from_named_token() {
        let payload = b"erc20: ERR_FORBIDDEN -- sstore not allowed in view";
        assert_eq!(extract_error_code(payload), Some(ErrorCode::Forbidden));
    }

    #[test]
    fn error_code_from_longer_name_wins() {
        // ERR_CROSS_CALL_OUT_OF_GAS contains ERR_CROSS_CALL_FAILED
        // as a prefix substring of the prefix — we sort by length
        // so the longer match wins.
        let payload = b"sub-call: ERR_CROSS_CALL_OUT_OF_GAS";
        assert_eq!(
            extract_error_code(payload),
            Some(ErrorCode::CrossCallOutOfGas)
        );
    }

    #[test]
    fn error_code_from_integer() {
        let payload = b"reverted with code -5";
        assert_eq!(extract_error_code(payload), Some(ErrorCode::Forbidden));
    }

    #[test]
    fn error_code_from_negative_unknown_returns_none() {
        let payload = b"random number -42 here";
        assert!(extract_error_code(payload).is_none());
    }

    #[test]
    fn error_code_no_match_returns_none() {
        let payload = b"nothing matches here at all";
        assert!(extract_error_code(payload).is_none());
    }

    // ── SdkError integration ──────────────────────────────────

    #[test]
    fn reverted_error_code_extracts_named_token() {
        let err = SdkError::Reverted {
            gas_used: 1000,
            data: b"erc721: ERR_INSUFFICIENT_BALANCE".to_vec(),
            reason: None,
        };
        assert_eq!(err.error_code(), Some(ErrorCode::InsufficientBalance));
        assert!(err.is_revert());
    }

    #[test]
    fn host_fn_variant_carries_typed_code() {
        let err = SdkError::HostFn {
            code: ErrorCode::ReentrancyBlocked,
            message: "buy() reentered".into(),
        };
        assert_eq!(err.error_code(), Some(ErrorCode::ReentrancyBlocked));
        assert_eq!(err.code(), "ERR_REENTRANCY_BLOCKED");
        assert!(!err.is_revert());
    }

    #[test]
    fn non_revert_variants_have_no_error_code() {
        let err = SdkError::Rpc("-32602: bad params".into());
        assert!(err.error_code().is_none());
    }

    #[test]
    fn code_string_for_each_variant() {
        // Spot-check every variant produces a stable label.
        assert_eq!(SdkError::Rpc(String::new()).code(), "RPC_ERROR");
        assert_eq!(
            SdkError::Connection(String::new()).code(),
            "CONNECTION_ERROR"
        );
        assert_eq!(SdkError::Signing(String::new()).code(), "SIGNING_ERROR");
        assert_eq!(SdkError::Timeout(String::new()).code(), "TIMEOUT");
        assert_eq!(
            SdkError::Reverted {
                gas_used: 0,
                data: vec![],
                reason: None,
            }
            .code(),
            "CALL_EXCEPTION"
        );
        assert_eq!(
            SdkError::InsufficientBalance {
                required: 0,
                available: 0
            }
            .code(),
            "INSUFFICIENT_FUNDS"
        );
        assert_eq!(
            SdkError::HostFn {
                code: ErrorCode::Forbidden,
                message: String::new()
            }
            .code(),
            "ERR_FORBIDDEN"
        );
        assert_eq!(
            SdkError::InvalidAddress(String::new()).code(),
            "INVALID_ARGUMENT"
        );
        assert_eq!(
            SdkError::InvalidArgument(String::new()).code(),
            "INVALID_ARGUMENT"
        );
        assert_eq!(
            SdkError::InvalidResponse(String::new()).code(),
            "INVALID_RESPONSE"
        );
        assert_eq!(SdkError::Other(String::new()).code(), "OTHER");
    }

    // ── Structured revert_reason wiring ──────────────────────

    #[test]
    fn revert_category_accessors_branch_on_category() {
        use crate::types::{RevertCategory, RevertReason};

        let eng = SdkError::Reverted {
            gas_used: 100,
            data: vec![],
            reason: Some(RevertReason {
                category: RevertCategory::EngineValidation,
                message: "nonce out of window".into(),
            }),
        };
        assert!(matches!(
            eng.revert_category(),
            Some(RevertCategory::EngineValidation)
        ));
        assert!(eng.is_engine_validation_revert());
        assert!(!eng.is_contract_revert());
        assert!(!eng.is_vm_trap());

        let con = SdkError::Reverted {
            gas_used: 100,
            data: vec![],
            reason: Some(RevertReason {
                category: RevertCategory::Contract,
                message: "ERR_FORBIDDEN".into(),
            }),
        };
        assert!(con.is_contract_revert());

        let vm = SdkError::Reverted {
            gas_used: 100,
            data: vec![],
            reason: Some(RevertReason {
                category: RevertCategory::Vm,
                message: "Trap(MemoryOutOfBounds)".into(),
            }),
        };
        assert!(vm.is_vm_trap());

        // No structured reason → accessors return None / false.
        let bare = SdkError::Reverted {
            gas_used: 100,
            data: vec![],
            reason: None,
        };
        assert!(bare.revert_category().is_none());
        assert!(!bare.is_engine_validation_revert());
    }

    #[test]
    fn format_revert_prefers_structured_reason() {
        use crate::types::{RevertCategory, RevertReason};
        let err = SdkError::Reverted {
            gas_used: 42_000,
            data: b"insufficient balance".to_vec(),
            reason: Some(RevertReason {
                category: RevertCategory::EngineValidation,
                message: "insufficient balance for fee+value: needed=42000, available=21000".into(),
            }),
        };
        let msg = format!("{err}");
        assert!(msg.contains("EngineValidation"));
        assert!(msg.contains("needed=42000"));
        assert!(msg.contains("gas=42000"));
    }

    #[test]
    fn from_receipt_maps_each_status() {
        use crate::types::{Receipt, ReceiptStatus, RevertCategory, RevertReason};
        fn receipt(status: ReceiptStatus, reason: Option<RevertReason>) -> Receipt {
            Receipt {
                tx_hash: "0x".into(),
                wave_id: "0x1".into(),
                tx_index: "0x0".into(),
                status,
                gas_used: "0x5208".into(),
                fee_paid: "0x5208".into(),
                return_data: "0x".into(),
                events: vec![],
                revert_reason: reason,
            }
        }

        // Success → None
        assert!(SdkError::from_receipt(&receipt(ReceiptStatus::Success, None)).is_none());

        // OutOfGas → synthesised Vm category
        let err = SdkError::from_receipt(&receipt(ReceiptStatus::OutOfGas, None)).unwrap();
        assert!(err.is_vm_trap());

        // Reverted + structured reason → carried through
        let err = SdkError::from_receipt(&receipt(
            ReceiptStatus::Reverted,
            Some(RevertReason {
                category: RevertCategory::Contract,
                message: "ERR_FORBIDDEN".into(),
            }),
        ))
        .unwrap();
        assert!(err.is_contract_revert());

        // Reverted + no structured reason → bare (older engine)
        let err = SdkError::from_receipt(&receipt(ReceiptStatus::Reverted, None)).unwrap();
        assert!(err.is_revert());
        assert!(err.revert_category().is_none());
    }
}
