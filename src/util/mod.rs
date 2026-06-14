//! Hex and unit-formatting helpers — the small utilities every
//! dapp / wallet needs.
//!
//! Address-specific helpers used to live here pre-newtype migration;
//! they're now inherent methods on [`crate::types::Address`]:
//!
//! | Old free function       | New method                          |
//! |-------------------------|-------------------------------------|
//! | `parse_address(s)`       | [`Address::from_hex`]               |
//! | `format_address(&a)`     | [`Address::to_hex`] or `format!("{a}")` |
//! | `is_zero_address(&a)`    | [`Address::is_zero`]                |
//! | `address_eq(&a, &b)`     | `a == b`                            |
//!
//! What stays here: hex encode/decode, unit formatting (PYDE ↔ quanta),
//! and small slice helpers used by calldata builders.
//!
//! [`Address::from_hex`]: crate::types::Address::from_hex
//! [`Address::to_hex`]: crate::types::Address::to_hex
//! [`Address::is_zero`]: crate::types::Address::is_zero

// ── Hex helpers ─────────────────────────────────────────────────────

/// True iff `value` is a syntactically-valid hex string — non-empty,
/// even-length, all hex digits, with optional `0x` prefix.
#[must_use]
pub fn is_hex_string(value: &str) -> bool {
    let hex_str = value.trim_start_matches("0x");
    !hex_str.is_empty() && hex_str.len() % 2 == 0 && hex_str.chars().all(|c| c.is_ascii_hexdigit())
}

/// Encode a byte slice as a `0x`-prefixed lower-case hex string.
///
/// Convention for all wire-format payloads — addresses, hashes,
/// signatures, calldata, return data. Mirrors ethers/alloy `hex()`.
#[must_use]
pub fn hexlify(data: &[u8]) -> String {
    format!("0x{}", hex::encode(data))
}

/// Decode a `0x`-prefixed (or bare) hex string into a byte vector.
///
/// # Errors
/// Returns a message string on bad input. Returns `String` rather
/// than [`crate::SdkError`] so this can be used in pure contexts
/// without pulling in the SDK error machinery.
pub fn get_bytes(value: &str) -> Result<Vec<u8>, String> {
    let hex_str = value.trim_start_matches("0x");
    hex::decode(hex_str).map_err(|e| format!("invalid hex: {e}"))
}

/// Convert a `u128` to a `0x`-prefixed big-endian hex string,
/// optionally left-padded to a fixed byte width.
///
/// With `width = Some(32)` the output is exactly 64 hex digits —
/// useful for fixed-width fields in calldata.
#[must_use]
pub fn to_be_hex(value: u128, width: Option<usize>) -> String {
    let raw = format!("{value:x}");
    let padded = if let Some(w) = width {
        format!("{raw:0>width$}", width = w * 2)
    } else if raw.len() % 2 == 0 {
        raw
    } else {
        format!("0{raw}")
    };
    format!("0x{padded}")
}

/// Concatenate multiple byte slices into a single `Vec<u8>`.
///
/// Pre-allocates the exact target length — common pattern when
/// building canonical pre-image bytes for hashing.
#[must_use]
pub fn concat_bytes(values: &[&[u8]]) -> Vec<u8> {
    let total: usize = values.iter().map(|v| v.len()).sum();
    let mut out = Vec::with_capacity(total);
    for v in values {
        out.extend_from_slice(v);
    }
    out
}

/// Left-pad `data` with zero bytes to reach `length`.
///
/// # Errors
/// Returns an error if `data.len() > length` (nothing to pad —
/// caller passed too much input).
pub fn zero_pad_value(data: &[u8], length: usize) -> Result<Vec<u8>, String> {
    if data.len() > length {
        return Err(format!(
            "value {} bytes exceeds pad length {length}",
            data.len()
        ));
    }
    let mut padded = vec![0u8; length];
    padded[length - data.len()..].copy_from_slice(data);
    Ok(padded)
}

/// Strip leading zero bytes from `data` (opposite of
/// [`zero_pad_value`]). Returns empty if all bytes are zero.
#[must_use]
pub fn strip_zeros(data: &[u8]) -> Vec<u8> {
    let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    data[start..].to_vec()
}

/// Byte length of a hex string after stripping the optional
/// `0x` prefix.
#[must_use]
pub fn data_length(hex_str: &str) -> usize {
    let h = hex_str.trim_start_matches("0x");
    h.len() / 2
}

// ── Unit formatting (1 PYDE = 10^9 quanta) ──────────────────────────

/// Decimal places between PYDE and its smallest unit (quanta).
///
/// 1 PYDE = 10^9 quanta. All on-chain `value` fields are
/// denominated in quanta (the `u128` field on a transaction).
/// UI layers convert via [`format_quanta`] and [`parse_quanta`].
pub const PYDE_DECIMALS: u32 = 9;

/// Parse a human-readable decimal amount (e.g. `"1.5"`) into raw
/// integer units at the given precision.
///
/// `parse_units("1.5", 9) → 1_500_000_000`.
/// `parse_units("100", 18) → 100_000_000_000_000_000_000`.
///
/// # Errors
/// - Negative inputs (Pyde uses unsigned balances)
/// - More fractional digits than `decimals` permits
/// - Non-decimal characters
/// - `decimals > 38` (would overflow `u128`)
pub fn parse_units(value: &str, decimals: u32) -> Result<u128, String> {
    if decimals > 38 {
        return Err(format!(
            "decimals {decimals} exceeds u128 precision (max 38)"
        ));
    }
    let trimmed = value.trim();
    if trimmed.starts_with('-') {
        return Err("negative values not supported for u128 units".into());
    }

    let parts: Vec<&str> = trimmed.split('.').collect();
    if parts.len() > 2 {
        return Err(format!("invalid numeric string: {value}"));
    }

    let whole = parts[0];
    let fraction = if parts.len() == 2 { parts[1] } else { "" };

    if fraction.len() > decimals as usize {
        return Err(format!(
            "too many decimal places: \"{value}\" has {} but only {decimals} allowed",
            fraction.len()
        ));
    }

    if !whole.chars().all(|c| c.is_ascii_digit())
        || (!fraction.is_empty() && !fraction.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(format!("invalid numeric string: {value}"));
    }

    let padded = format!("{fraction:0<width$}", width = decimals as usize);
    let combined = format!("{whole}{padded}");
    combined
        .parse::<u128>()
        .map_err(|e| format!("overflow: {e}"))
}

/// Format a raw integer amount as a human-readable decimal at the
/// given precision.
///
/// `format_units(1_500_000_000, 9) → "1.5"`.
/// `format_units(1_000_000, 9) → "0.001"`.
///
/// Trailing zeros in the fractional part are stripped. A whole
/// number returns `"<whole>.0"` for unambiguous parsing.
#[must_use]
pub fn format_units(value: u128, decimals: u32) -> String {
    if decimals > 38 {
        return format!("{value}");
    }
    let divisor = 10u128.pow(decimals);
    let whole = value / divisor;
    let remainder = value % divisor;

    let frac = format!("{remainder:0>width$}", width = decimals as usize);
    let trimmed = frac.trim_end_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };

    format!("{whole}.{trimmed}")
}

/// Parse a human-readable PYDE amount to raw quanta.
///
/// Convenience wrapper for [`parse_units(value, PYDE_DECIMALS)`].
/// `parse_quanta("1.5") → 1_500_000_000`.
pub fn parse_quanta(value: &str) -> Result<u128, String> {
    parse_units(value, PYDE_DECIMALS)
}

/// Format raw quanta as a human-readable PYDE amount.
///
/// Convenience wrapper for [`format_units(value, PYDE_DECIMALS)`].
/// `format_quanta(1_500_000_000) → "1.5"`.
#[must_use]
pub fn format_quanta(value: u128) -> String {
    format_units(value, PYDE_DECIMALS)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn hex_helpers_round_trip() {
        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let s = hexlify(&bytes);
        assert_eq!(s, "0xdeadbeef");
        let back = get_bytes(&s).unwrap();
        assert_eq!(bytes, back);
    }

    #[test]
    fn is_hex_string_rejects_bad_input() {
        assert!(is_hex_string("0xab"));
        assert!(is_hex_string("ab"));
        assert!(!is_hex_string("0xabc")); // odd length
        assert!(!is_hex_string("0xzz")); // non-hex
        assert!(!is_hex_string("")); // empty
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
            assert_eq!(&format_quanta(raw), s, "round-trip {s}");
        }
    }

    #[test]
    fn to_be_hex_pads_to_width() {
        assert_eq!(to_be_hex(0x42, None), "0x42");
        assert_eq!(to_be_hex(0x42, Some(2)), "0x0042");
        assert_eq!(to_be_hex(0x42, Some(32)).len(), 2 + 64);
    }

    #[test]
    fn concat_and_pad_helpers() {
        let a = vec![1u8, 2, 3];
        let b = vec![4u8, 5];
        let c = concat_bytes(&[&a, &b]);
        assert_eq!(c, vec![1, 2, 3, 4, 5]);

        let padded = zero_pad_value(&[0x42], 4).unwrap();
        assert_eq!(padded, vec![0, 0, 0, 0x42]);

        let stripped = strip_zeros(&[0, 0, 0x01, 0x02]);
        assert_eq!(stripped, vec![1, 2]);
    }

    #[test]
    fn data_length_strips_prefix() {
        assert_eq!(data_length("0xabcd"), 2);
        assert_eq!(data_length("abcd"), 2);
        assert_eq!(data_length("0x"), 0);
    }
}
