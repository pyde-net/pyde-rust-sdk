# 9. Errors

[← back to TOC](README.md) · prev: [Events](08-events.md) · next: [Multisig →](10-multisig.md)

---

Every SDK call that can fail returns `Result<T, SdkError>`. This
chapter covers what each variant means, how to decode revert
reasons from contract calls, and how to map host-fn error codes
to dapp-facing messages.

## Table of contents

- [9.1 `SdkError` variants](#91-sdkerror-variants)
- [9.2 Inspecting revert reasons](#92-inspecting-revert-reasons)
- [9.3 HOST_FN_ABI §4 error codes](#93-host_fn_abi-4-error-codes)
- [9.4 `ErrorCode` enum + helpers](#94-errorcode-enum--helpers)
- [9.5 Extracting an `ErrorCode` from any `SdkError`](#95-extracting-an-errorcode-from-any-sdkerror)
- [9.6 Dapp UX pattern — map codes to messages](#96-dapp-ux-pattern--map-codes-to-messages)
- [9.7 RPC error envelope codes](#97-rpc-error-envelope-codes)
- [9.8 Debugging tips](#98-debugging-tips)

---

## 9.1 `SdkError` variants

```rust,ignore
pub enum SdkError {
    InvalidAddress(String),
    InvalidArgument(String),
    Signing(String),
    Rpc { code: i32, message: String, data: Option<Value> },
    Transport(String),
    Reverted { reason: Option<String>, raw: Option<Vec<u8>> },
    HostFn { code: ErrorCode, message: String },
    NotFound,
    Other(String),
}
```

| Variant | When you see it | Typical handling |
|---|---|---|
| `InvalidAddress` | Bad hex address — wrong length, non-hex chars | Validate input on the way in. |
| `InvalidArgument` | Bad input to a SDK fn — unknown tx_type, malformed borsh, `from` not set on `TxBuilder` | Usually a programming bug; surface to dev logs. |
| `Signing` | FALCON keygen / sign failed, or pubkey doesn't pair with secret | Check the signer / HSM is responding. |
| `Rpc` | Server returned a JSON-RPC error envelope (`{code, message, data}`) | Parse `code` for known cases (e.g., `-32603` = internal). |
| `Connection` | Network-level failure — connect refused, TLS handshake, timeout | Retried automatically by `HttpTransport` (3× default). |
| `Reverted` | Contract execution reverted; reason / raw bytes carried if available | See §9.2. |
| `HostFn` | Host fn returned a structured `ErrorCode` (HOST_FN_ABI §4) | See §9.3. |
| `NotFound` | Resource doesn't exist (tx, receipt, name, etc.) | Often a normal not-yet-seen state; check + retry. |
| `InvalidResponse` | RPC server sent a malformed envelope | Server-side bug — log & retry. |
| `Other` | Catch-all for things that don't fit (borsh errors, JSON parse, etc.) | Inspect the inner message. |

---

## 9.2 Inspecting revert reasons

When a contract reverts, the SDK surfaces `SdkError::Reverted`
with:

- `reason: Option<String>` — best-effort decoded human-readable string.
- `raw: Option<Vec<u8>>` — the raw bytes the contract passed to
  `pyde.revert`.

```rust,no_run
use pyde_rust_sdk::error::SdkError;
# fn handle(err: SdkError) {
match err {
    SdkError::Reverted { reason: Some(msg), .. } => {
        println!("contract said: {msg}");
    }
    SdkError::Reverted { raw: Some(bytes), .. } => {
        println!("contract reverted with {} raw bytes", bytes.len());
    }
    SdkError::Reverted { .. } => {
        println!("contract reverted with no reason");
    }
    other => println!("other error: {other}"),
}
# }
```

### `decode_revert_reason` — the algorithm

`reason` is produced by `decode_revert_reason(&raw)` which tries
three encodings in order, returning the first that produces a
printable string:

| Order | Encoding | Why a contract might use it |
|---|---|---|
| 1 | Borsh `String` | The default `pyde.revert("msg")` in Rust contracts. |
| 2 | Legacy u64-prefixed length | Older contracts; some toolchains. |
| 3 | Raw UTF-8 | C / Go contracts that hand-build the buffer. |

Printability check: control codes (other than `\n`, `\r`, `\t`)
disqualify the candidate. Falls through to the next encoding if
the decoded string looks like binary garbage.

---

## 9.3 HOST_FN_ABI §4 error codes

Host functions like `pyde.sstore`, `pyde.call::execute`, and
`pyde.transfer` return structured error codes when the chain
detects something it can name precisely. The SDK exposes 18
codes mirroring `engine/crates/types/src/error.rs`:

### Full table

| Code | Constant | When |
|------|----------|------|
|  -1  | `ERR_INVALID_INPUT`            | Malformed calldata, bad parameter |
|  -2  | `ERR_NOT_FOUND`                | Slot / account / name doesn't exist |
|  -3  | `ERR_INSUFFICIENT_BALANCE`     | Account doesn't hold enough quanta |
|  -4  | `ERR_OUT_OF_GAS`               | Gas budget exhausted |
|  -5  | `ERR_FORBIDDEN`                | Caller is not authorised |
|  -6  | `ERR_ACCESS_LIST_VIOLATION`    | Touched a slot not declared in `tx.access_list` |
|  -7  | `ERR_OUTPUT_BUFFER_TOO_SMALL`  | Caller's `out_len_ptr` was smaller than the data |
|  -8  | `ERR_INVALID_ADDRESS`          | Address fails structural validation |
|  -9  | `ERR_REENTRANCY_BLOCKED`       | Detected re-entry into a guarded function |
| -10  | `ERR_CROSS_CALL_FAILED`        | Sub-contract call returned non-zero |
| -11  | `ERR_CROSS_CALL_OUT_OF_GAS`    | Sub-contract ran out of gas |
| -12  | `ERR_VALUE_TRANSFER_NOT_PAYABLE` | Sent value to a non-payable entry |
| -13  | `ERR_INVALID_FUNCTION_NAME`    | Calldata selector doesn't match any entry |
| -14  | `ERR_XCALL_RATE_LIMITED`       | Cross-contract-call quota exceeded |
| -15  | `ERR_PARACHAIN_ONLY`           | Called a §8 host fn from a `Contract` (only allowed on `Parachain`) |
| -16  | `ERR_CIPHERTEXT_INVALID`       | Threshold-decryption AEAD failed |
| -17  | `ERR_SIGNATURE_INVALID`        | FALCON verify failed |
| -100 | `ERR_INTERNAL`                 | Chain-side bug. Should never reach user code. |

These constants live at
[`crate::types::error_code`](../src/types/error_code.rs) and
mirror the engine byte-for-byte.

---

## 9.4 `ErrorCode` enum + helpers

```rust,ignore
pub enum ErrorCode {
    InvalidInput, NotFound, InsufficientBalance, OutOfGas,
    Forbidden, AccessListViolation, OutputBufferTooSmall,
    InvalidAddress, ReentrancyBlocked, CrossCallFailed,
    CrossCallOutOfGas, ValueTransferNotPayable,
    InvalidFunctionName, XcallRateLimited, ParachainOnly,
    CiphertextInvalid, SignatureInvalid, Internal,
}
```

### `ErrorCode::as_i32() -> i32`

| Returns | The wire integer (e.g. `-5` for `Forbidden`). |
|---|---|

### `ErrorCode::name() -> &'static str`

| Returns | The canonical constant name (e.g. `"ERR_FORBIDDEN"`). Useful for logs and dapp UX. |
|---|---|

### `ErrorCode::from_i32(i32) -> Option<ErrorCode>`

| Returns | The matching variant, or `None` if the integer isn't a known error code. |
|---|---|

### Round-trip example

```rust,no_run
use pyde_rust_sdk::types::error_code::ErrorCode;

# fn run() {
let e = ErrorCode::Forbidden;
assert_eq!(e.as_i32(), -5);
assert_eq!(e.name(), "ERR_FORBIDDEN");
assert_eq!(ErrorCode::from_i32(-5), Some(ErrorCode::Forbidden));
assert_eq!(ErrorCode::from_i32(-999), None);
# }
```

---

## 9.5 Extracting an `ErrorCode` from any `SdkError`

Contracts can embed error codes in revert reasons two ways:

1. **Named token** in the string (e.g. `"ERR_FORBIDDEN: caller is not admin"`).
2. **Negative integer** at the start of the reason (e.g. `"aborted with code -5"`).

`SdkError::error_code()` probes both:

```rust,no_run
use pyde_rust_sdk::error::SdkError;
use pyde_rust_sdk::types::error_code::ErrorCode;

# fn check(err: &SdkError) {
match err.error_code() {
    Some(ErrorCode::Forbidden) => {
        // Show "Not authorised" to the user.
    }
    Some(ErrorCode::InsufficientBalance) => {
        // Show "Top up to continue".
    }
    Some(other) => {
        // Map any code → user-facing message.
        println!("got code {}", other.name());
    }
    None => {
        // Revert had no recognisable code — fall back to raw reason.
    }
}
# }
```

### Longest-match-wins token scan

`ERR_CROSS_CALL_OUT_OF_GAS` wins over `ERR_CROSS_CALL_FAILED`
even though both substrings appear in the longer string. This
means contract authors can include the more specific code first
and the SDK picks it up reliably.

### Integer-code parser

```
"aborted with code -5"            → Some(ErrorCode::Forbidden)
"failed: status -10"              → Some(ErrorCode::CrossCallFailed)
"plain message no integer"        → None
"-9999 random number not a code"  → None  (not in the known set)
```

The parser looks for a leading negative integer in the
whitespace-normalised string. It stops at the first non-digit
after the sign.

---

## 9.6 Dapp UX pattern — map codes to messages

Build a single mapping from `ErrorCode` → user-facing message:

```rust,no_run
use pyde_rust_sdk::error::SdkError;
use pyde_rust_sdk::types::error_code::ErrorCode;

fn user_message(err: &SdkError) -> String {
    if let Some(code) = err.error_code() {
        return match code {
            ErrorCode::Forbidden        => "You're not authorised to do that.".into(),
            ErrorCode::InsufficientBalance => "Top up your account to continue.".into(),
            ErrorCode::OutOfGas         => "Transaction ran out of gas — try raising gas_limit.".into(),
            ErrorCode::AccessListViolation => "Contract touched state outside the declared access list.".into(),
            ErrorCode::ReentrancyBlocked => "Re-entrant call blocked.".into(),
            ErrorCode::ValueTransferNotPayable => "This contract method doesn't accept PYDE.".into(),
            ErrorCode::InvalidFunctionName => "Called a function that doesn't exist on this contract.".into(),
            ErrorCode::CrossCallFailed | ErrorCode::CrossCallOutOfGas
                                         => "A sub-call failed — try again or raise gas.".into(),
            _ => format!("Reverted ({}): see logs.", code.name()),
        };
    }
    if let SdkError::Reverted { reason: Some(r), .. } = err {
        return format!("Reverted: {r}");
    }
    if let SdkError::Connection(msg) = err {
        return format!("Network problem — please retry. ({msg})");
    }
    "Unexpected error — see logs.".into()
}
```

The live `examples/halt_methods.rs` walks through this pattern
against a real reverting Go contract; see [Examples §11](11-examples.md).

---

## 9.7 RPC error envelope codes

The chain's JSON-RPC error envelope comes through as
`SdkError::Rpc { code, message, data }`. Standard JSON-RPC codes:

| Code | Meaning |
|---|---|
| `-32700` | Parse error (malformed JSON) |
| `-32600` | Invalid request |
| `-32601` | Method not found |
| `-32602` | Invalid params |
| `-32603` | Internal error (chain-side) |
| `-32000` to `-32099` | Server-defined; see chain spec for the per-method map |

`data` is the per-method extension — for failed tx submission,
the chain often puts the rejected tx's borsh-decoded fields
there for debugging.

### Pattern-matching `Rpc`

```rust,no_run
use pyde_rust_sdk::error::SdkError;

# fn handle(err: SdkError) {
if let SdkError::Rpc { code: -32601, message, .. } = err {
    println!("RPC method not found: {message}");
}
# }
```

Common cases:

| Server message | Likely cause |
|---|---|
| `"nonce too low"` | You submitted with a nonce below the account's window — get a fresh nonce. |
| `"nonce too high"` | Nonce above the 16-slot window — earlier txs in flight haven't landed yet. |
| `"insufficient balance"` | Account can't cover `value + gas_limit × gas_price`. |
| `"signature does not match account"` | `tx.from` doesn't match the FALCON pubkey on the signed hash. |
| `"deadline expired"` | The `deadline` wave passed before the tx committed. |

---

## 9.8 Debugging tips

The SDK doesn't have internal logging — by design, it stays
silent so it doesn't pollute your dapp's stdout. For
investigating a failed call:

1. **Use `simulate_transaction(&tx)`** before submitting. The
   simulation result includes the revert reason and gas usage
   without committing the tx. See [Providers §6.5](06-providers.md#65-simulating).
2. **Check `Receipt.return_data`** on a `Reverted` receipt — the
   raw bytes from `pyde.revert` are there. See [Providers §6.3](06-providers.md#transactions--6-methods).
3. **Inspect `Tx` borsh** with `tx::encode(&tx)` if you suspect
   field drift; compare against the chain's expected shape (see
   [Compatibility §12.1](12-compatibility.md#121-wire-format-guarantees)).
4. **Wrap the transport** with a logging `Transport` impl (see
   [Providers §6.8](06-providers.md#68-custom-transports)) if
   you need per-call request/response dumps.
5. **Bump retry count** via `RetryConfig` if you suspect transient
   network flakes (see [Providers §6.6](06-providers.md#66-retry-policy)).
