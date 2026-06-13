//! Address, hex, and unit-formatting helpers.
//!
//! These are the small utilities every dapp / wallet needs: parse and
//! format 32-byte addresses, convert between PYDE and raw quanta, hexlify
//! arbitrary bytes. They have no chain dependencies — just pure functions
//! over strings and byte slices.
//!
//! Salvaged largely intact from the pre-pivot SDK; the only semantics
//! change is decimals (1 PYDE = 10^9 quanta — Pyde's smallest unit, set
//! by [`PYDE_DECIMALS`]).

use crate::error::SdkError;
use crate::types::Address;

// ── Address helpers ─────────────────────────────────────────────────

/// The 32-byte zero address (all bytes 0x00).
///
/// Used as `to` for contract deployments and as a sentinel for "no
/// recipient." Distinct from a never-written account, which doesn't
/// exist in state at all.
pub const ZERO_ADDRESS: Address = [0u8; 32];

/// Parse a `0x`-prefixed (or bare) 64-character hex string into a 32-byte
/// address.
///
/// # Errors
/// Returns [`SdkError::InvalidAddress`] if the input is not exactly
/// 64 hex characters after stripping the optional `0x` prefix, or if
/// any character is not a valid hex digit.
pub fn parse_address(s: &str) -> Result<Address, SdkError> {
    let hex = s.trim_start_matches("0x");
    if hex.len() != 64 {
        return Err(SdkError::InvalidAddress(format!(
            "expected 64 hex chars, got {}",
            hex.len()
        )));
    }
    let bytes =
        hex::decode(hex).map_err(|e| SdkError::InvalidAddress(format!("bad hex: {}", e)))?;
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&bytes);
    Ok(addr)
}

/// Format a 32-byte address as a `0x`-prefixed 64-character hex string.
///
/// The canonical Pyde address representation in wallets, explorers, and
/// JSON-RPC payloads. Always lower-case hex.
pub fn format_address(addr: &Address) -> String {
    format!("0x{}", hex::encode(addr))
}

/// True iff the given address is the zero address ([`ZERO_ADDRESS`]).
///
/// Useful as a quick check before signing a tx — sending to the zero
/// address is almost always a mistake unless deploying a contract
/// (where the deploy handler interprets `to == ZERO_ADDRESS` correctly).
pub fn is_zero_address(addr: &Address) -> bool {
    *addr == ZERO_ADDRESS
}

/// True iff `s` is a syntactically-valid hex address — exactly 64 hex
/// digits after stripping an optional `0x` prefix.
///
/// Does NOT verify the address corresponds to a live account on-chain.
/// Use [`Provider::get_account`] for that.
pub fn is_valid_address(s: &str) -> bool {
    let hex = s.trim_start_matches("0x");
    hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Convenience equality check for two addresses (just `==` under the
/// hood, but reads more naturally in call chains).
pub fn address_eq(a: &Address, b: &Address) -> bool {
    a == b
}

// ── Hex helpers ─────────────────────────────────────────────────────

/// True iff `value` is a syntactically-valid hex string — non-empty,
/// even-length, all hex digits (with optional `0x` prefix).
pub fn is_hex_string(value: &str) -> bool {
    let hex = value.trim_start_matches("0x");
    !hex.is_empty() && hex.len().is_multiple_of(2) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

/// Encode a byte slice as a `0x`-prefixed lower-case hex string.
///
/// Convention for all wire-format payloads in the SDK: addresses, hashes,
/// signatures, calldata, return data. Mirrors ethers/alloy `hex()` and
/// JS `ethers.utils.hexlify`.
pub fn hexlify(data: &[u8]) -> String {
    format!("0x{}", hex::encode(data))
}

/// Decode a `0x`-prefixed (or bare) hex string into a byte vector.
///
/// # Errors
/// Returns an error message string if the input contains non-hex
/// characters or has odd length. Returns a string rather than
/// [`SdkError`] so it can be used in pure contexts without pulling in
/// the SDK error machinery.
pub fn get_bytes(value: &str) -> Result<Vec<u8>, String> {
    let hex_str = value.trim_start_matches("0x");
    hex::decode(hex_str).map_err(|e| format!("Invalid hex: {}", e))
}

/// Convert a `u128` to a `0x`-prefixed big-endian hex string, optionally
/// left-padded to a fixed byte width.
///
/// With `width = Some(32)` the output is exactly 64 hex digits — useful
/// for encoding values as fixed-width fields in calldata.
pub fn to_be_hex(value: u128, width: Option<usize>) -> String {
    let hex = format!("{:x}", value);
    let padded = if let Some(w) = width {
        format!("{:0>width$}", hex, width = w * 2)
    } else if hex.len() % 2 != 0 {
        format!("0{}", hex)
    } else {
        hex
    };
    format!("0x{}", padded)
}

/// Concatenate multiple byte slices into a single `Vec<u8>`.
///
/// Pre-allocates the exact target length to avoid intermediate
/// reallocations. Common pattern: building canonical pre-image bytes
/// for hashing.
pub fn concat_bytes(values: &[&[u8]]) -> Vec<u8> {
    let total: usize = values.iter().map(|v| v.len()).sum();
    let mut result = Vec::with_capacity(total);
    for v in values {
        result.extend_from_slice(v);
    }
    result
}

/// Left-pad `data` with zero bytes to reach `length`.
///
/// # Errors
/// Returns an error if `data.len() > length` (nothing to pad — caller
/// passed too much input).
pub fn zero_pad_value(data: &[u8], length: usize) -> Result<Vec<u8>, String> {
    if data.len() > length {
        return Err(format!(
            "Value {} bytes exceeds pad length {}",
            data.len(),
            length
        ));
    }
    let mut padded = vec![0u8; length];
    padded[length - data.len()..].copy_from_slice(data);
    Ok(padded)
}

/// Strip leading zero bytes from `data` (the opposite of
/// [`zero_pad_value`]). Returns an empty `Vec` if all bytes are zero.
pub fn strip_zeros(data: &[u8]) -> Vec<u8> {
    let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    data[start..].to_vec()
}

/// Byte length of a hex string after stripping the optional `0x` prefix.
pub fn data_length(hex: &str) -> usize {
    let h = hex.trim_start_matches("0x");
    h.len() / 2
}

// ── Unit formatting (1 PYDE = 10^9 quanta) ──────────────────────────

/// Decimal places between PYDE and its smallest unit (quanta).
///
/// 1 PYDE = 10^9 quanta. All on-chain `value` fields are denominated in
/// quanta (the `u128` field on a transaction). UI layers convert via
/// [`format_quanta`] and [`parse_quanta`].
pub const PYDE_DECIMALS: u32 = 9;

/// Parse a human-readable decimal amount (e.g. `"1.5"`) into raw integer
/// units at the given precision.
///
/// `parse_units("1.5", 9)` → `Ok(1_500_000_000)`.
/// `parse_units("100", 18)` → `Ok(100_000_000_000_000_000_000)`.
///
/// # Errors
/// - Negative inputs (Pyde uses unsigned `u128` for balances)
/// - More fractional digits than `decimals` permits
/// - Non-decimal characters
/// - Decimals > 38 (overflows `u128` precision)
pub fn parse_units(value: &str, decimals: u32) -> Result<u128, String> {
    if decimals > 38 {
        return Err(format!(
            "decimals {} exceeds u128 precision (max 38)",
            decimals
        ));
    }
    let trimmed = value.trim();
    if trimmed.starts_with('-') {
        return Err("Negative values not supported for u128 units".into());
    }

    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("Invalid numeric string: {}", value));
    }

    let whole = parts[0];
    let fraction = if parts.len() == 2 { parts[1] } else { "" };

    if fraction.len() > decimals as usize {
        return Err(format!(
            "Too many decimal places: \"{}\" has {} but only {} allowed",
            value,
            fraction.len(),
            decimals
        ));
    }

    if !whole.chars().all(|c| c.is_ascii_digit())
        || (!fraction.is_empty() && !fraction.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(format!("Invalid numeric string: {}", value));
    }

    let padded = format!("{:0<width$}", fraction, width = decimals as usize);
    let combined = format!("{}{}", whole, padded);
    combined
        .parse::<u128>()
        .map_err(|e| format!("Overflow: {}", e))
}

/// Format a raw integer amount as a human-readable decimal at the given
/// precision.
///
/// `format_units(1_500_000_000, 9)` → `"1.5"`.
/// `format_units(1_000_000, 9)` → `"0.001"`.
///
/// Trailing zeros in the fractional part are stripped. A value with no
/// fractional component returns `"<whole>.0"` for unambiguous parsing
/// (so `format_units(5, 9)` is `"0.000000005"`, not `"0.000000005."`).
pub fn format_units(value: u128, decimals: u32) -> String {
    if decimals > 38 {
        return format!("{}", value);
    }
    let divisor = 10u128.pow(decimals);
    let whole = value / divisor;
    let remainder = value % divisor;

    let frac_str = format!("{:0>width$}", remainder, width = decimals as usize);
    let trimmed = frac_str.trim_end_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };

    format!("{}.{}", whole, trimmed)
}

/// Parse a human-readable PYDE amount to raw quanta.
///
/// Convenience wrapper for [`parse_units(value, PYDE_DECIMALS)`].
/// `parse_quanta("1.5")` → `Ok(1_500_000_000)`.
pub fn parse_quanta(value: &str) -> Result<u128, String> {
    parse_units(value, PYDE_DECIMALS)
}

/// Format raw quanta as a human-readable PYDE amount.
///
/// Convenience wrapper for [`format_units(value, PYDE_DECIMALS)`].
/// `format_quanta(1_500_000_000)` → `"1.5"`.
pub fn format_quanta(value: u128) -> String {
    format_units(value, PYDE_DECIMALS)
}

#[cfg(test)]
mod tests {
    // Tests are allowed to use unwrap/expect/panic — the strict lint
    // bar exists for production paths; tests get to assert directly.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parse_and_format_address_round_trip() {
        let addr_hex = "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let addr = parse_address(addr_hex).expect("parse");
        assert_eq!(format_address(&addr), addr_hex);
    }

    #[test]
    fn zero_address_detected() {
        assert!(is_zero_address(&ZERO_ADDRESS));
        let one = [0u8; 32];
        assert!(is_zero_address(&one));
    }

    #[test]
    fn parse_quanta_handles_decimals() {
        assert_eq!(parse_quanta("1").unwrap(), 1_000_000_000);
        assert_eq!(parse_quanta("1.5").unwrap(), 1_500_000_000);
        assert_eq!(parse_quanta("0.000000001").unwrap(), 1);
        assert_eq!(parse_quanta("0").unwrap(), 0);
    }

    #[test]
    fn format_quanta_strips_trailing_zeros() {
        assert_eq!(format_quanta(1_500_000_000), "1.5");
        assert_eq!(format_quanta(1), "0.000000001");
        assert_eq!(format_quanta(1_000_000_000), "1.0");
    }

    #[test]
    fn quanta_round_trips() {
        for s in &[
            "0.5",
            "100.123456789",
            "0.000000123",
            "9999999999.999999999",
        ] {
            let raw = parse_quanta(s).unwrap_or_else(|e| panic!("parse {s}: {e}"));
            assert_eq!(format_quanta(raw), *s, "round-trip {s}");
        }
    }

    #[test]
    fn invalid_address_rejected() {
        assert!(parse_address("0xabc").is_err()); // too short
        assert!(parse_address("0xzzzz...").is_err()); // non-hex
        assert!(!is_valid_address("0xabc"));
    }
}
