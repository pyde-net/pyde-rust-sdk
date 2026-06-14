# 9. Errors

[← back to TOC](README.md) · prev: [Events](08-events.md) · next: [Multisig →](10-multisig.md)

---

Every SDK call that can fail returns `Result<T, SdkError>`. This
chapter covers what each variant means, how to decode revert
reasons from contract calls, and how to map host-fn error codes
to dapp-facing messages.

## `SdkError`

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

| Variant | When |
|---|---|
| `InvalidAddress` | Bad hex address — wrong length, non-hex chars |
| `InvalidArgument` | Bad input to a SDK fn (e.g. unknown tx_type, malformed borsh, `from` not set on `TxBuilder`) |
| `Signing` | FALCON keygen / sign failed, or pubkey doesn't pair with secret |
| `Rpc` | Server returned a JSON-RPC error envelope (`{code, message, data}`) |
| `Transport` | Network-level failure — connect refused, TLS handshake, timeout |
| `Reverted` | Contract execution reverted; reason / raw bytes carried if available |
| `HostFn` | Host fn returned a structured `ErrorCode` (HOST_FN_ABI §4) |
| `NotFound` | Resource doesn't exist (tx, receipt, name, etc.) |
| `Other` | Catch-all for things that don't fit (borsh errors, JSON parse, etc.) |

## Inspecting revert reasons

When a contract reverts, the SDK surfaces `SdkError::Reverted`
with `reason` (best-effort decoded string) + `raw` (the bytes
the contract passed to `pyde.revert`).

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

`reason` is produced by `decode_revert_reason(&raw)` which tries
three encodings in order:
1. Borsh `String` decode.
2. Legacy `u64-prefixed` length encoding.
3. Raw UTF-8.

Whichever decodes to a printable string first wins.

## HOST_FN_ABI §4 error codes

Host functions like `pyde.sstore`, `pyde.call::execute`, and
`pyde.transfer` return structured error codes when the chain
detects something it can name precisely. The SDK exposes 18
codes mirroring `engine/crates/types/src/error.rs`:

| Code | Constant | Meaning |
|------|----------|---------|
|  -1  | `ERR_INVALID_INPUT`            | Malformed calldata, bad parameter |
|  -2  | `ERR_NOT_FOUND`                | Slot / account / name doesn't exist |
|  -3  | `ERR_INSUFFICIENT_BALANCE`     | Account doesn't hold enough quanta |
|  -4  | `ERR_OUT_OF_GAS`               | Gas budget exhausted |
|  -5  | `ERR_FORBIDDEN`                | Caller is not authorised |
|  -6  | `ERR_ACCESS_LIST_VIOLATION`    | Touched a slot not declared in tx.access_list |
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

The `ErrorCode` enum mirrors these:

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

Helpers on `ErrorCode`:

```rust,no_run
use pyde_rust_sdk::types::error_code::ErrorCode;

let e = ErrorCode::Forbidden;
assert_eq!(e.as_i32(), -5);
assert_eq!(e.name(), "ERR_FORBIDDEN");
assert_eq!(ErrorCode::from_i32(-5), Some(ErrorCode::Forbidden));
```

## Extracting an `ErrorCode` from any `SdkError`

Contracts can emit `ERR_*` codes in two ways:
1. **Named token** in the revert reason string (e.g. `"ERR_FORBIDDEN: caller is not admin"`).
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
    }
    None => {
        // Revert had no recognisable code — fall back to the
        // raw reason string.
    }
}
# }
```

The named-token scan is **longest-match-wins** — `ERR_CROSS_CALL_OUT_OF_GAS`
wins over `ERR_CROSS_CALL_FAILED` even though both substrings
appear in the longer string.

## Dapp UX pattern

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
            _ => format!("Reverted ({}): see logs.", code.name()),
        };
    }
    if let SdkError::Reverted { reason: Some(r), .. } = err {
        return format!("Reverted: {r}");
    }
    "Unexpected error — see logs.".into()
}
```

The live `halt_methods.rs` example walks through this pattern
against a real reverting contract — see
[Examples](11-examples.md#halt_methods).

## RPC errors

The chain's JSON-RPC error envelope comes through as
`SdkError::Rpc { code, message, data }`. JSON-RPC codes:

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

## Debugging tips

The SDK doesn't have internal logging — by design, it stays
silent so it doesn't pollute your dapp's stdout. For
investigating a failed call:

1. **Use `simulate_transaction(&tx)`** before submitting. The
   simulation result includes the revert reason and gas usage
   without committing the tx.
2. **Check `Receipt.return_data`** on a `Reverted` receipt — the
   raw bytes from `pyde.revert` are there.
3. **Inspect `Tx` borsh** with `tx::encode(&tx)` if you suspect
   field drift; compare against the chain's expected shape (see
   [Compatibility](12-compatibility.md#tx-wire-format)).
4. **Wrap the transport** with a logging `Transport` impl (see
   [Providers — Custom transports](06-providers.md#custom-transports))
   if you need per-call request/response dumps.
