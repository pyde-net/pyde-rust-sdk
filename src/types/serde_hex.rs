//! Serde helpers that accept BOTH the wire shape the engine actually
//! emits (`0x`-prefixed lowercase hex strings, per spec §17.4) AND
//! the bare numeric shape (kept for backward-compat with older
//! engines and the SDK's own JSON-fixture corpus).
//!
//! Plugged into individual struct fields via
//! `#[serde(deserialize_with = "deserialize_u64_hex_or_int")]`. The
//! borsh path on the same struct is unaffected — borsh reads bytes,
//! not field-name-keyed JSON.

use serde::de::{Deserializer, Error};
use serde::Deserialize;

/// Accept a `u64` as either a bare JSON number or a `0x`-prefixed
/// hex string. The hex path is what `pyde_getTx` / `pyde_getReceipt`
/// / etc. actually ship.
///
/// # Errors
/// Returns a serde error when the value is neither a number nor a
/// hex string the standard `u64::from_str_radix` can parse.
pub fn deserialize_u64_hex_or_int<'de, D>(de: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    parse_u64(StringOrNumber::deserialize(de)?)
}

/// Accept a `u128` as either a bare JSON number or a `0x`-prefixed
/// hex string.
///
/// # Errors
/// Returns a serde error when the value is neither a number nor a
/// hex string the standard `u128::from_str_radix` can parse.
pub fn deserialize_u128_hex_or_int<'de, D>(de: D) -> Result<u128, D::Error>
where
    D: Deserializer<'de>,
{
    parse_u128(StringOrNumber::deserialize(de)?)
}

/// Optional-`u64` variant for `Option<u64>` fields (e.g. `Tx::deadline`).
///
/// # Errors
/// Same as [`deserialize_u64_hex_or_int`] when the inner value is
/// present but malformed; explicit JSON `null` is preserved as `None`.
pub fn deserialize_opt_u64_hex_or_int<'de, D>(de: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Option<StringOrNumber> = Option::deserialize(de)?;
    raw.map(parse_u64).transpose()
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringOrNumber {
    String(String),
    Number(serde_json::Number),
}

fn parse_u64<E: Error>(v: StringOrNumber) -> Result<u64, E> {
    match v {
        StringOrNumber::String(s) => {
            let body = s.strip_prefix("0x").unwrap_or(&s);
            u64::from_str_radix(body, 16).map_err(|e| E::custom(format!("invalid hex u64: {e}")))
        }
        StringOrNumber::Number(n) => n
            .as_u64()
            .ok_or_else(|| E::custom(format!("number out of u64 range: {n}"))),
    }
}

fn parse_u128<E: Error>(v: StringOrNumber) -> Result<u128, E> {
    match v {
        StringOrNumber::String(s) => {
            let body = s.strip_prefix("0x").unwrap_or(&s);
            u128::from_str_radix(body, 16).map_err(|e| E::custom(format!("invalid hex u128: {e}")))
        }
        StringOrNumber::Number(n) => {
            // u64 range is fine — JSON numbers can't natively carry
            // > 2^53 without precision loss anyway, so anything
            // larger comes through the string branch.
            n.as_u64()
                .map(u128::from)
                .ok_or_else(|| E::custom(format!("number out of u128 range: {n}")))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct U64Wrap {
        #[serde(deserialize_with = "deserialize_u64_hex_or_int")]
        v: u64,
    }

    #[derive(Deserialize)]
    struct U128Wrap {
        #[serde(deserialize_with = "deserialize_u128_hex_or_int")]
        v: u128,
    }

    #[derive(Deserialize)]
    struct OptWrap {
        #[serde(deserialize_with = "deserialize_opt_u64_hex_or_int", default)]
        v: Option<u64>,
    }

    #[test]
    fn u64_accepts_hex_string() {
        let w: U64Wrap = serde_json::from_str(r#"{"v":"0x7a69"}"#).unwrap();
        assert_eq!(w.v, 0x7a69);
    }

    #[test]
    fn u64_accepts_bare_int() {
        let w: U64Wrap = serde_json::from_str(r#"{"v":31337}"#).unwrap();
        assert_eq!(w.v, 31337);
    }

    #[test]
    fn u128_accepts_hex_string() {
        let w: U128Wrap = serde_json::from_str(r#"{"v":"0xf4240"}"#).unwrap();
        assert_eq!(w.v, 0xf_4240);
    }

    #[test]
    fn opt_u64_accepts_null() {
        let w: OptWrap = serde_json::from_str(r#"{"v":null}"#).unwrap();
        assert_eq!(w.v, None);
    }

    #[test]
    fn opt_u64_accepts_hex_string() {
        let w: OptWrap = serde_json::from_str(r#"{"v":"0x186a0"}"#).unwrap();
        assert_eq!(w.v, Some(0x1_86a0));
    }

    #[test]
    fn opt_u64_absent_is_none_via_default() {
        let w: OptWrap = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(w.v, None);
    }
}
