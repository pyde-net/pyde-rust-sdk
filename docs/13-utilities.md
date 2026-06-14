# 13. Utilities

[← back to TOC](README.md) · prev: [Compatibility](12-compatibility.md) · next: [Constants →](14-constants.md)

---

Reference chapter for every public helper in `crate::util`.
These are small functions you'll reach for constantly when
building calldata, formatting amounts, or shipping bytes to and
from the wire.

## Table of contents

- [13.1 Hex helpers](#131-hex-helpers)
- [13.2 PYDE ↔ quanta conversion](#132-pyde--quanta-conversion)
- [13.3 General unit conversion](#133-general-unit-conversion)
- [13.4 Byte / slice helpers](#134-byte--slice-helpers)
- [13.5 Address helpers (moved)](#135-address-helpers-moved)

---

## 13.1 Hex helpers

### `is_hex_string(value)`

| | |
|---|---|
| Signature | `fn is_hex_string(value: &str) -> bool` |
| `value` | The candidate string. Optional `0x` prefix is tolerated. |
| Returns | `true` if `value` is non-empty, even-length, and all chars are hex digits (with optional `0x`). |

```rust,no_run
use pyde_rust_sdk::util::is_hex_string;

# fn run() {
assert!(is_hex_string("0xdeadbeef"));
assert!(is_hex_string("deadbeef"));         // bare also ok
assert!(!is_hex_string("0xabc"));           // odd length
assert!(!is_hex_string("0xzz"));            // non-hex char
assert!(!is_hex_string(""));                // empty
# }
```

### `hexlify(data)`

| | |
|---|---|
| Signature | `fn hexlify(data: &[u8]) -> String` |
| `data` | Byte slice to encode. |
| Returns | `"0x..."` lower-case hex string. Always prefixed. |

```rust,no_run
use pyde_rust_sdk::util::hexlify;

# fn run() {
assert_eq!(hexlify(&[0xDE, 0xAD, 0xBE, 0xEF]), "0xdeadbeef");
assert_eq!(hexlify(&[]), "0x");
# }
```

This is the canonical format for **every** wire payload —
addresses, hashes, signatures, calldata, return data. Always
emits `0x` prefix.

### `get_bytes(value)`

| | |
|---|---|
| Signature | `fn get_bytes(value: &str) -> Result<Vec<u8>, String>` |
| `value` | `0x`-prefixed (or bare) hex string. |
| Returns | `Ok(decoded_bytes)` or `Err("invalid hex: ...")`. |

```rust,no_run
use pyde_rust_sdk::util::get_bytes;

# fn run() -> Result<(), String> {
assert_eq!(get_bytes("0xdeadbeef")?, vec![0xDE, 0xAD, 0xBE, 0xEF]);
assert_eq!(get_bytes("deadbeef")?, vec![0xDE, 0xAD, 0xBE, 0xEF]);
assert!(get_bytes("zz").is_err());
# Ok(()) }
```

Returns `Result<_, String>` rather than `Result<_, SdkError>`
because hex utilities live below the SDK error machinery — use
in pure contexts without pulling the error crate.

### `to_be_hex(value, width)`

| | |
|---|---|
| Signature | `fn to_be_hex(value: u128, width: Option<usize>) -> String` |
| `value` | The integer to encode. |
| `width` | Optional byte width to left-pad to. `Some(32)` → 64 hex digits. `None` → minimum even-length. |
| Returns | `"0x..."` big-endian hex string. |

```rust,no_run
use pyde_rust_sdk::util::to_be_hex;

# fn run() {
assert_eq!(to_be_hex(0x42, None), "0x42");
assert_eq!(to_be_hex(0x42, Some(2)), "0x0042");
assert_eq!(to_be_hex(0x42, Some(32)).len(), 2 + 64);  // "0x" + 64 hex
# }
```

Useful for fixed-width fields in calldata where you need exact
byte alignment.

### `data_length(hex_str)`

| | |
|---|---|
| Signature | `fn data_length(hex_str: &str) -> usize` |
| Returns | The decoded byte length — `hex_str.len() / 2` after stripping `0x`. |

```rust,no_run
use pyde_rust_sdk::util::data_length;

# fn run() {
assert_eq!(data_length("0xabcd"), 2);
assert_eq!(data_length("abcd"), 2);
assert_eq!(data_length("0x"), 0);
# }
```

Computes byte length without actually decoding — useful for
sizing checks before allocating buffers.

---

## 13.2 PYDE ↔ quanta conversion

PYDE has **9 decimal places**. All `value` fields on a tx are
`u128` quanta — the smallest unit. **Never use floats** for
balance math.

### `parse_quanta(value)`

| | |
|---|---|
| Signature | `fn parse_quanta(value: &str) -> Result<u128, String>` |
| `value` | Human-readable decimal PYDE amount. |
| Returns | `Ok(quanta)` or `Err(msg)` on malformed input. |

```rust,no_run
use pyde_rust_sdk::util::parse_quanta;

# fn run() -> Result<(), String> {
assert_eq!(parse_quanta("1.5")?, 1_500_000_000);
assert_eq!(parse_quanta("1")?, 1_000_000_000);
assert_eq!(parse_quanta("0.000000001")?, 1);          // 1 quanta
assert_eq!(parse_quanta("0")?, 0);
assert_eq!(parse_quanta("100.123456789")?, 100_123_456_789);

// Rejects:
assert!(parse_quanta("-1").is_err());                 // negative
assert!(parse_quanta("1.0000000001").is_err());       // too many decimals
assert!(parse_quanta("one").is_err());                // non-numeric
assert!(parse_quanta("1e9").is_err());                // scientific notation
# Ok(()) }
```

### `format_quanta(value)`

| | |
|---|---|
| Signature | `fn format_quanta(value: u128) -> String` |
| `value` | Raw quanta. |
| Returns | `"<whole>.<fraction>"` with trailing zeros stripped; whole numbers as `"N.0"`. |

```rust,no_run
use pyde_rust_sdk::util::format_quanta;

# fn run() {
assert_eq!(format_quanta(1_500_000_000), "1.5");
assert_eq!(format_quanta(1), "0.000000001");          // 1 quanta
assert_eq!(format_quanta(1_000_000_000), "1.0");      // whole-PYDE → "N.0"
assert_eq!(format_quanta(0), "0.0");
# }
```

### Round-trip guarantee

`format_quanta(parse_quanta(s)?) == s` for every valid input.

```rust,no_run
use pyde_rust_sdk::util::{format_quanta, parse_quanta};

# fn run() -> Result<(), String> {
for s in &["0.5", "100.123456789", "0.000000123", "9999999999.999999999"] {
    let raw = parse_quanta(s)?;
    assert_eq!(format_quanta(raw), *s);
}
# Ok(()) }
```

---

## 13.3 General unit conversion

If you ever need non-PYDE precision (e.g., interacting with a
contract that uses 18-decimal token units like ERC20-USDC),
`parse_units` / `format_units` take the decimals explicitly.

### `parse_units(value, decimals)`

| | |
|---|---|
| Signature | `fn parse_units(value: &str, decimals: u32) -> Result<u128, String>` |
| `value` | Human-readable decimal amount. |
| `decimals` | Number of decimal places. Max `38` (u128 capacity). |
| Returns | `Ok(raw_units)` or `Err(msg)`. |

```rust,no_run
use pyde_rust_sdk::util::parse_units;

# fn run() -> Result<(), String> {
// 9 decimals (PYDE)
assert_eq!(parse_units("1.5", 9)?, 1_500_000_000);

// 18 decimals (ERC20-style)
assert_eq!(parse_units("1.5", 18)?, 1_500_000_000_000_000_000);
assert_eq!(parse_units("100", 18)?, 100_000_000_000_000_000_000);

// 0 decimals (integer-only)
assert_eq!(parse_units("42", 0)?, 42);
assert!(parse_units("42.5", 0).is_err());     // any decimals reject
# Ok(()) }
```

### `format_units(value, decimals)`

| | |
|---|---|
| Signature | `fn format_units(value: u128, decimals: u32) -> String` |
| `value` | Raw integer units. |
| `decimals` | Number of decimal places to format with. |
| Returns | Human-readable string with trailing zeros stripped. |

```rust,no_run
use pyde_rust_sdk::util::format_units;

# fn run() {
// 9 decimals (PYDE)
assert_eq!(format_units(1_500_000_000, 9), "1.5");

// 18 decimals (ERC20-style)
assert_eq!(format_units(1_500_000_000_000_000_000, 18), "1.5");

// 6 decimals (USDC-style)
assert_eq!(format_units(1_500_000, 6), "1.5");
# }
```

### `PYDE_DECIMALS` constant

```rust,ignore
pub const PYDE_DECIMALS: u32 = 9;
```

Used internally by `parse_quanta` / `format_quanta`. Exposed
so you can reference the spec value without hard-coding it.

---

## 13.4 Byte / slice helpers

Small helpers for the cases that come up when building canonical
pre-image buffers (calldata, hash inputs).

### `concat_bytes(values)`

| | |
|---|---|
| Signature | `fn concat_bytes(values: &[&[u8]]) -> Vec<u8>` |
| Returns | All slices concatenated. Pre-allocated to the exact target length. |

```rust,no_run
use pyde_rust_sdk::util::concat_bytes;

# fn run() {
let a = [1u8, 2, 3];
let b = [4u8, 5];
let c = concat_bytes(&[&a, &b]);
assert_eq!(c, vec![1, 2, 3, 4, 5]);
# }
```

Common pattern when building hash pre-images (e.g., `Poseidon2(domain ‖ nonce ‖ payload)`).

### `zero_pad_value(data, length)`

| | |
|---|---|
| Signature | `fn zero_pad_value(data: &[u8], length: usize) -> Result<Vec<u8>, String>` |
| `data` | Bytes to pad. |
| `length` | Target total length. |
| Returns | Left-padded result. `Err` if `data.len() > length`. |

```rust,no_run
use pyde_rust_sdk::util::zero_pad_value;

# fn run() -> Result<(), String> {
let padded = zero_pad_value(&[0x42], 4)?;
assert_eq!(padded, vec![0, 0, 0, 0x42]);

// Errors when input is too big:
assert!(zero_pad_value(&[0; 8], 4).is_err());
# Ok(()) }
```

### `strip_zeros(data)`

| | |
|---|---|
| Signature | `fn strip_zeros(data: &[u8]) -> Vec<u8>` |
| Returns | `data` with leading zero bytes removed. Empty if all bytes are zero. |

```rust,no_run
use pyde_rust_sdk::util::strip_zeros;

# fn run() {
assert_eq!(strip_zeros(&[0, 0, 0x01, 0x02]), vec![1, 2]);
assert_eq!(strip_zeros(&[0, 0, 0]), Vec::<u8>::new());
assert_eq!(strip_zeros(&[0x01, 0x02]), vec![1, 2]);
# }
```

The inverse of `zero_pad_value`.

---

## 13.5 Address helpers (moved)

Pre-newtype migration, the SDK exposed free functions like
`parse_address`, `format_address`, `is_zero_address`,
`address_eq`. These now live as inherent methods on
`crate::types::Address`:

| Old free function | New method |
|---|---|
| `parse_address(s)` | [`Address::from_hex`](04-wallets.md#42-wallet--high-level-keypair) |
| `format_address(&a)` | `Address::to_hex` or `format!("{a}")` |
| `is_zero_address(&a)` | `Address::is_zero` |
| `address_eq(&a, &b)` | `a == b` |

See [Concepts §3.3](03-concepts.md#33-addresses) for full
address API + derivation methods.
