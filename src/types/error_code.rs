//! Host-function ABI error codes from [HOST_FN_ABI §4][spec].
//!
//! Mirrors `engine/crates/types/src/error.rs` byte-for-byte. Each
//! negative `i32` returned by a host function maps to a named
//! constant + an [`ErrorCode`] variant. The codes are pinned at v1
//! mainnet under the ABI's one-way ratchet — they cannot be removed
//! or have their meaning changed once mainnet ships.
//!
//! [spec]: https://book.pyde.network/companion/HOST_FN_ABI_SPEC#4-error-codes

use serde::{Deserialize, Serialize};

/// Malformed input bytes (non-32-byte hash, non-canonical encoding,
/// …).
pub const ERR_INVALID_INPUT: i32 = -1;
/// Reserved. Storage reads return zero on missing slots; this code
/// surfaces only as a cross-call failure indicator.
pub const ERR_NOT_FOUND: i32 = -2;
/// Caller balance too low for the requested operation.
pub const ERR_INSUFFICIENT_BALANCE: i32 = -3;
/// Gas budget exhausted (typically a trap; returned here for
/// `consume_gas`).
pub const ERR_OUT_OF_GAS: i32 = -4;
/// Operation not permitted in this context (e.g., `sstore` from a
/// `view` function).
pub const ERR_FORBIDDEN: i32 = -5;
/// Accessed slot not in the declared access list.
pub const ERR_ACCESS_LIST_VIOLATION: i32 = -6;
/// Caller's output buffer was smaller than required.
pub const ERR_OUTPUT_BUFFER_TOO_SMALL: i32 = -7;
/// Address format invalid (all-zero, reserved sentinel, …).
pub const ERR_INVALID_ADDRESS: i32 = -8;
/// Cross-call would re-enter a non-`reentrant` function.
pub const ERR_REENTRANCY_BLOCKED: i32 = -9;
/// Sub-call trapped or returned a non-zero error code.
pub const ERR_CROSS_CALL_FAILED: i32 = -10;
/// Sub-call exhausted its forwarded gas.
pub const ERR_CROSS_CALL_OUT_OF_GAS: i32 = -11;
/// Attempted value transfer to a function not marked `payable`.
pub const ERR_VALUE_TRANSFER_NOT_PAYABLE: i32 = -12;
/// `cross_call` target function does not exist on the target.
pub const ERR_INVALID_FUNCTION_NAME: i32 = -13;
/// Parachain cross-message budget exceeded for this wave
/// (parachain only).
pub const ERR_XCALL_RATE_LIMITED: i32 = -14;
/// Function callable only from parachain context.
pub const ERR_PARACHAIN_ONLY: i32 = -15;
/// Threshold-decryption ciphertext malformed.
pub const ERR_CIPHERTEXT_INVALID: i32 = -16;
/// FALCON signature verification failed.
pub const ERR_SIGNATURE_INVALID: i32 = -17;
/// Engine-side bug or unexpected state. Should never occur in a
/// correct implementation; surfaces as a trap in practice.
pub const ERR_INTERNAL: i32 = -100;

/// Strongly-typed wrapper around the [`ERR_*`](self) codes.
///
/// Host functions return raw `i32` for ABI fidelity. The SDK
/// exposes this typed shape so consumer code can match on it
/// rather than comparing raw integers.
///
/// Round-trip invariant: `ErrorCode::from_i32(c.as_i32()) == Some(c)`
/// for every listed variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ErrorCode {
    /// See [`ERR_INVALID_INPUT`].
    InvalidInput = ERR_INVALID_INPUT,
    /// See [`ERR_NOT_FOUND`].
    NotFound = ERR_NOT_FOUND,
    /// See [`ERR_INSUFFICIENT_BALANCE`].
    InsufficientBalance = ERR_INSUFFICIENT_BALANCE,
    /// See [`ERR_OUT_OF_GAS`].
    OutOfGas = ERR_OUT_OF_GAS,
    /// See [`ERR_FORBIDDEN`].
    Forbidden = ERR_FORBIDDEN,
    /// See [`ERR_ACCESS_LIST_VIOLATION`].
    AccessListViolation = ERR_ACCESS_LIST_VIOLATION,
    /// See [`ERR_OUTPUT_BUFFER_TOO_SMALL`].
    OutputBufferTooSmall = ERR_OUTPUT_BUFFER_TOO_SMALL,
    /// See [`ERR_INVALID_ADDRESS`].
    InvalidAddress = ERR_INVALID_ADDRESS,
    /// See [`ERR_REENTRANCY_BLOCKED`].
    ReentrancyBlocked = ERR_REENTRANCY_BLOCKED,
    /// See [`ERR_CROSS_CALL_FAILED`].
    CrossCallFailed = ERR_CROSS_CALL_FAILED,
    /// See [`ERR_CROSS_CALL_OUT_OF_GAS`].
    CrossCallOutOfGas = ERR_CROSS_CALL_OUT_OF_GAS,
    /// See [`ERR_VALUE_TRANSFER_NOT_PAYABLE`].
    ValueTransferNotPayable = ERR_VALUE_TRANSFER_NOT_PAYABLE,
    /// See [`ERR_INVALID_FUNCTION_NAME`].
    InvalidFunctionName = ERR_INVALID_FUNCTION_NAME,
    /// See [`ERR_XCALL_RATE_LIMITED`].
    XCallRateLimited = ERR_XCALL_RATE_LIMITED,
    /// See [`ERR_PARACHAIN_ONLY`].
    ParachainOnly = ERR_PARACHAIN_ONLY,
    /// See [`ERR_CIPHERTEXT_INVALID`].
    CiphertextInvalid = ERR_CIPHERTEXT_INVALID,
    /// See [`ERR_SIGNATURE_INVALID`].
    SignatureInvalid = ERR_SIGNATURE_INVALID,
    /// See [`ERR_INTERNAL`].
    Internal = ERR_INTERNAL,
}

impl ErrorCode {
    /// The raw `i32` returned across the WASM ⇄ host boundary.
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    /// Parse a raw return code. Returns `None` for `0` (success) or
    /// any value not in the spec table.
    #[must_use]
    pub const fn from_i32(code: i32) -> Option<Self> {
        Some(match code {
            ERR_INVALID_INPUT => Self::InvalidInput,
            ERR_NOT_FOUND => Self::NotFound,
            ERR_INSUFFICIENT_BALANCE => Self::InsufficientBalance,
            ERR_OUT_OF_GAS => Self::OutOfGas,
            ERR_FORBIDDEN => Self::Forbidden,
            ERR_ACCESS_LIST_VIOLATION => Self::AccessListViolation,
            ERR_OUTPUT_BUFFER_TOO_SMALL => Self::OutputBufferTooSmall,
            ERR_INVALID_ADDRESS => Self::InvalidAddress,
            ERR_REENTRANCY_BLOCKED => Self::ReentrancyBlocked,
            ERR_CROSS_CALL_FAILED => Self::CrossCallFailed,
            ERR_CROSS_CALL_OUT_OF_GAS => Self::CrossCallOutOfGas,
            ERR_VALUE_TRANSFER_NOT_PAYABLE => Self::ValueTransferNotPayable,
            ERR_INVALID_FUNCTION_NAME => Self::InvalidFunctionName,
            ERR_XCALL_RATE_LIMITED => Self::XCallRateLimited,
            ERR_PARACHAIN_ONLY => Self::ParachainOnly,
            ERR_CIPHERTEXT_INVALID => Self::CiphertextInvalid,
            ERR_SIGNATURE_INVALID => Self::SignatureInvalid,
            ERR_INTERNAL => Self::Internal,
            _ => return None,
        })
    }

    /// Stable identifier for this code — matches the engine's
    /// `ERR_*` constant name. Suitable for log labels, metric
    /// dimensions, and consumer-side `match` arms.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::InvalidInput => "ERR_INVALID_INPUT",
            Self::NotFound => "ERR_NOT_FOUND",
            Self::InsufficientBalance => "ERR_INSUFFICIENT_BALANCE",
            Self::OutOfGas => "ERR_OUT_OF_GAS",
            Self::Forbidden => "ERR_FORBIDDEN",
            Self::AccessListViolation => "ERR_ACCESS_LIST_VIOLATION",
            Self::OutputBufferTooSmall => "ERR_OUTPUT_BUFFER_TOO_SMALL",
            Self::InvalidAddress => "ERR_INVALID_ADDRESS",
            Self::ReentrancyBlocked => "ERR_REENTRANCY_BLOCKED",
            Self::CrossCallFailed => "ERR_CROSS_CALL_FAILED",
            Self::CrossCallOutOfGas => "ERR_CROSS_CALL_OUT_OF_GAS",
            Self::ValueTransferNotPayable => "ERR_VALUE_TRANSFER_NOT_PAYABLE",
            Self::InvalidFunctionName => "ERR_INVALID_FUNCTION_NAME",
            Self::XCallRateLimited => "ERR_XCALL_RATE_LIMITED",
            Self::ParachainOnly => "ERR_PARACHAIN_ONLY",
            Self::CiphertextInvalid => "ERR_CIPHERTEXT_INVALID",
            Self::SignatureInvalid => "ERR_SIGNATURE_INVALID",
            Self::Internal => "ERR_INTERNAL",
        }
    }
}

impl core::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ({})", self.name(), self.as_i32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_codes_match_spec_table() {
        // Wire-load-bearing per HOST_FN_ABI §4.
        assert_eq!(ERR_INVALID_INPUT, -1);
        assert_eq!(ERR_NOT_FOUND, -2);
        assert_eq!(ERR_INSUFFICIENT_BALANCE, -3);
        assert_eq!(ERR_OUT_OF_GAS, -4);
        assert_eq!(ERR_FORBIDDEN, -5);
        assert_eq!(ERR_ACCESS_LIST_VIOLATION, -6);
        assert_eq!(ERR_OUTPUT_BUFFER_TOO_SMALL, -7);
        assert_eq!(ERR_INVALID_ADDRESS, -8);
        assert_eq!(ERR_REENTRANCY_BLOCKED, -9);
        assert_eq!(ERR_CROSS_CALL_FAILED, -10);
        assert_eq!(ERR_CROSS_CALL_OUT_OF_GAS, -11);
        assert_eq!(ERR_VALUE_TRANSFER_NOT_PAYABLE, -12);
        assert_eq!(ERR_INVALID_FUNCTION_NAME, -13);
        assert_eq!(ERR_XCALL_RATE_LIMITED, -14);
        assert_eq!(ERR_PARACHAIN_ONLY, -15);
        assert_eq!(ERR_CIPHERTEXT_INVALID, -16);
        assert_eq!(ERR_SIGNATURE_INVALID, -17);
        assert_eq!(ERR_INTERNAL, -100);
    }

    #[test]
    fn discriminants_match_constants() {
        for code in [
            ErrorCode::InvalidInput,
            ErrorCode::NotFound,
            ErrorCode::InsufficientBalance,
            ErrorCode::OutOfGas,
            ErrorCode::Forbidden,
            ErrorCode::AccessListViolation,
            ErrorCode::OutputBufferTooSmall,
            ErrorCode::InvalidAddress,
            ErrorCode::ReentrancyBlocked,
            ErrorCode::CrossCallFailed,
            ErrorCode::CrossCallOutOfGas,
            ErrorCode::ValueTransferNotPayable,
            ErrorCode::InvalidFunctionName,
            ErrorCode::XCallRateLimited,
            ErrorCode::ParachainOnly,
            ErrorCode::CiphertextInvalid,
            ErrorCode::SignatureInvalid,
            ErrorCode::Internal,
        ] {
            assert_eq!(ErrorCode::from_i32(code.as_i32()), Some(code));
        }
    }

    #[test]
    fn from_i32_rejects_zero_and_unknown() {
        assert!(ErrorCode::from_i32(0).is_none());
        assert!(ErrorCode::from_i32(1).is_none());
        assert!(ErrorCode::from_i32(-50).is_none());
        assert!(ErrorCode::from_i32(i32::MIN).is_none());
    }

    #[test]
    fn name_uses_engine_const_label() {
        assert_eq!(ErrorCode::Forbidden.name(), "ERR_FORBIDDEN");
        assert_eq!(
            ErrorCode::InsufficientBalance.name(),
            "ERR_INSUFFICIENT_BALANCE"
        );
        assert_eq!(ErrorCode::Internal.name(), "ERR_INTERNAL");
    }

    #[test]
    fn display_shows_name_and_value() {
        let s = format!("{}", ErrorCode::Forbidden);
        assert!(s.contains("ERR_FORBIDDEN"));
        assert!(s.contains("-5"));
    }
}
