# 14. Constants

[← back to TOC](README.md) · prev: [Utilities](13-utilities.md)

---

Reference chapter for every public constant in the SDK. These
are the magic numbers you'd otherwise hard-code; the SDK exposes
them so they stay consistent with the chain spec and update if
the spec moves.

## Table of contents

- [14.1 Address constants](#141-address-constants)
- [14.2 Hash + size constants](#142-hash--size-constants)
- [14.3 Gas constants](#143-gas-constants)
- [14.4 Keystore parameters](#144-keystore-parameters)
- [14.5 Codec caps](#145-codec-caps)
- [14.6 Commit-reveal constants](#146-commit-reveal-constants)
- [14.7 Pending tx defaults](#147-pending-tx-defaults)
- [14.8 Multisig domain bytes](#148-multisig-domain-bytes)
- [14.9 Error codes (HOST_FN_ABI §4)](#149-error-codes-host_fn_abi-4)
- [14.10 ABI versions](#1410-abi-versions)
- [14.11 Function attribute bits](#1411-function-attribute-bits)
- [14.12 Nonce window + multisig caps](#1412-nonce-window--multisig-caps)
- [14.13 PYDE units](#1413-pyde-units)
- [14.14 JSON-RPC version](#1414-json-rpc-version)

---

## 14.1 Address constants

In `crate::types::address`:

| Constant | Value | What |
|---|---|---|
| `ADDRESS_LEN` | `32` | Address byte length. Pyde addresses are full 32-byte Poseidon2 digests — no truncation. |
| `Address::ZERO` | `Self([0u8; 32])` | The sentinel zero address. Used by `Deploy` and envelope-style txs. |
| `CREATE2_PREFIX` | `0xFF` | Domain prefix for `Address::create2` derivations. Matches the Ethereum convention. |
| `CONTRACT_ADDRESS_PREFIX` | `b"pyde-contract:"` | Domain prefix for `Address::from_contract_name` derivations. |

```rust,no_run
use pyde_rust_sdk::types::{Address, address::ADDRESS_LEN};

# fn run() {
assert_eq!(ADDRESS_LEN, 32);
assert!(Address::ZERO.is_zero());
# }
```

---

## 14.2 Hash + size constants

In `crate::types::hash` and `crate::types::falcon`:

| Constant | Value | What |
|---|---|---|
| `HASH_LEN` | `32` | All hash types (`TxHash`, `Blake3Hash`, `Poseidon2Hash`) are 32 bytes. |
| `FALCON_PUBKEY_LEN` | `897` | FALCON-512 public key size. |
| `FALCON_SECRET_LEN` | `1281` | FALCON-512 secret key size. |
| `FALCON_SIG_MAX_LEN` | `690` | Theoretical max FALCON-512 signature size; typical sigs average ~666 bytes. |

```rust,no_run
use pyde_rust_sdk::types::{
    FALCON_PUBKEY_LEN, FALCON_SECRET_LEN, FALCON_SIG_MAX_LEN, HASH_LEN,
};

# fn run() {
assert_eq!(HASH_LEN, 32);
assert_eq!(FALCON_PUBKEY_LEN, 897);
assert_eq!(FALCON_SECRET_LEN, 1281);
assert_eq!(FALCON_SIG_MAX_LEN, 690);
# }
```

---

## 14.3 Gas constants

In `crate::constants`. These are **convention** values — what
typical dapps use as a starting gas_limit; not chain-enforced
floors.

| Constant | Value | When |
|---|---|---|
| `GAS_TRANSFER` | `100_000` | A vanilla PYDE transfer (`tx_type=Standard`, empty calldata). Slightly above the engine's `MIN_GAS_LIMIT = 21_000` to absorb hashing + sig-verify costs. |
| `GAS_ERC20_CALL` | `500_000` | An ERC20-style state-mutating call (transfer, approve, transferFrom). |
| `GAS_ERC721_CALL` | `1_000_000` | An ERC721-style state-mutating call (mint, transferFrom, approve, setApprovalForAll). |
| `GAS_DEPLOY` | `10_000_000` | A typical contract deployment (15-30 KiB WASM bundle). |
| `GAS_CROSS_CALL_ORCHESTRATOR` | `2_000_000` | A marketplace-style cross-contract call (`buy(listing_id)` fan-out). |

```rust,no_run
use pyde_rust_sdk::constants::{GAS_TRANSFER, GAS_ERC20_CALL};
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::Address;

# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0)
    .gas_limit(GAS_TRANSFER)
    .transfer(Address::ZERO, 1)
    .build()?;
# Ok(()) }
```

Tune per workload. The actual gas the chain charges is in
`Receipt.gas_used`.

---

## 14.4 Keystore parameters

In `crate::wallet`:

| Constant | Value | What |
|---|---|---|
| `KEYSTORE_VERSION` | `1` | Envelope version this SDK encrypts + decrypts. |
| `ARGON2_M_KIB` | `65_536` | Argon2id memory cost in KiB (64 MiB). |
| `ARGON2_T` | `3` | Argon2id iteration count. |
| `ARGON2_P` | `1` | Argon2id parallelism (lanes). |
| `ARGON2_SALT_LEN` | `16` | Salt size in bytes (random per write). |

```rust,no_run
use pyde_rust_sdk::wallet::{ARGON2_M_KIB, ARGON2_P, ARGON2_T, KEYSTORE_VERSION};

# fn run() {
assert_eq!(KEYSTORE_VERSION, 1);
assert_eq!(ARGON2_M_KIB, 64 * 1024);     // 64 MiB
assert_eq!(ARGON2_T, 3);
assert_eq!(ARGON2_P, 1);
# }
```

Targets ~250 ms on a modern laptop CPU. See [Wallets §4.4](04-wallets.md#44-encrypted-keystore-on-disk).

---

## 14.5 Codec caps

| Constant | Value | What |
|---|---|---|
| `MAX_DECODE_ELEMENTS` | `1_000_000` | Max declared length on any borsh `Vec` / `String` decoded by `src/contract/codec.rs`. Protects against hostile RPC responses that try to allocate gigabytes. |

```rust,no_run
use pyde_rust_sdk::contract::MAX_DECODE_ELEMENTS;
# fn run() {
assert_eq!(MAX_DECODE_ELEMENTS, 1_000_000);
# }
```

### Private WS caps (not part of the public API)

The `WsTransport` enforces additional internal caps on
subscription buffering. These aren't exposed as `pub const`
(they may change without bumping a major version), but worth
knowing:

| Cap | Value | Effect |
|---|---|---|
| Max concurrent subscriptions per transport | `128` | 129th subscription is dropped. |
| Max queued events per subscription | `256` | Events past this cap drop the slowest reader. |

---

## 14.6 Commit-reveal constants

In `crate::tx`. These govern the private mempool's commit-reveal
lane — the MEV protection where a transaction's ordering position
is fixed before its contents are visible.

| Constant | Value | What |
|---|---|---|
| `COMMIT_REVEAL_WINDOW_WAVES` | `120` | Waves the chain allows between a `Commit` landing and its matching `Reveal`. Reveal past this window forfeits the bond. |
| `MIN_COMMIT_BOND` | `1_000_000_000` | Floor bond a `Commit` must post, in quanta (1 PYDE). `required_bond` never returns less than this. |
| `COMMIT_BOND_BPS` | `100` | Bond rate in basis points (1%) applied to a commit's `value_ceiling`; the bond is `max(MIN_COMMIT_BOND, value_ceiling * COMMIT_BOND_BPS / 10_000)`. |

```rust,no_run
use pyde_rust_sdk::tx::{COMMIT_BOND_BPS, COMMIT_REVEAL_WINDOW_WAVES, MIN_COMMIT_BOND, required_bond};

# fn run() {
assert_eq!(COMMIT_REVEAL_WINDOW_WAVES, 120);
assert_eq!(MIN_COMMIT_BOND, 1_000_000_000); // 1 PYDE
assert_eq!(COMMIT_BOND_BPS, 100);           // 1%
assert_eq!(required_bond(0), MIN_COMMIT_BOND);
# }
```

The `Commit` tx (`TxType::Commit = 0x11`) carries a
`CommitPayload` in `tx.data` and posts `required_bond(value_ceiling)`
as `tx.value`; the later `Reveal` tx (`TxType::Reveal = 0x12`)
carries a `RevealPayload`. See [Providers §6.3](06-providers.md)
for `send_private` and the one-call flow.

---

## 14.7 Pending tx defaults

In `crate::provider::pending`:

| Constant | Value | What |
|---|---|---|
| `DEFAULT_POLL_INTERVAL` | `250 ms` | How often `PendingTx::wait_for_receipt` polls `pyde_getReceipt`. |
| `DEFAULT_TIMEOUT` | `60 s` | Total deadline for `wait_for_receipt` before it gives up. |

```rust,no_run
use pyde_rust_sdk::provider::{DEFAULT_POLL_INTERVAL, DEFAULT_TIMEOUT};
use std::time::Duration;

# fn run() {
assert_eq!(DEFAULT_POLL_INTERVAL, Duration::from_millis(250));
assert_eq!(DEFAULT_TIMEOUT, Duration::from_secs(60));
# }
```

Override on a per-call basis:

```rust,no_run
# use std::sync::Arc;
# use std::time::Duration;
# use pyde_rust_sdk::Provider;
# use pyde_rust_sdk::types::Tx;
# async fn run(provider: Arc<pyde_rust_sdk::provider::HttpProvider>, tx: &Tx) -> pyde_rust_sdk::Result<()> {
let receipt = provider
    .send_transaction(tx).await?
    .with_poll_interval(Duration::from_millis(50))
    .with_timeout(Duration::from_secs(120))
    .wait_for_receipt().await?;
# Ok(()) }
```

---

## 14.8 Multisig domain bytes

In `crate::multisig`:

| Constant | Value | Tx type |
|---|---|---|
| `DOMAIN_MULTISIG_TX` | `0x09` | `MultisigTx` |
| `DOMAIN_ROTATE_MULTISIG` | `0x0A` | `RotateMultisig` |
| `DOMAIN_EMERGENCY_PAUSE` | `0x0B` | `EmergencyPause` |
| `DOMAIN_EMERGENCY_RESUME` | `0x0C` | `EmergencyResume` |
| `DOMAIN_DISPUTE_SLASH` | `0x10` | `DisputeSlash` |

```rust,no_run
use pyde_rust_sdk::multisig::{
    DOMAIN_DISPUTE_SLASH, DOMAIN_EMERGENCY_PAUSE, DOMAIN_EMERGENCY_RESUME,
    DOMAIN_MULTISIG_TX, DOMAIN_ROTATE_MULTISIG,
};

# fn run() {
assert_eq!(DOMAIN_MULTISIG_TX, 0x09);
assert_eq!(DOMAIN_ROTATE_MULTISIG, 0x0A);
assert_eq!(DOMAIN_EMERGENCY_PAUSE, 0x0B);
assert_eq!(DOMAIN_EMERGENCY_RESUME, 0x0C);
assert_eq!(DOMAIN_DISPUTE_SLASH, 0x10);
# }
```

These pin the engine's `domain_byte` map; if the engine reshapes
a value, the SDK's tests fail before signatures drift in
production.

See [Multisig §10.1](10-multisig.md#101-the-canonical-message).

---

## 14.9 Error codes (HOST_FN_ABI §4)

In `crate::types::error_code`:

| Constant | Value | Variant |
|---|---|---|
| `ERR_INVALID_INPUT` | `-1` | `ErrorCode::InvalidInput` |
| `ERR_NOT_FOUND` | `-2` | `ErrorCode::NotFound` |
| `ERR_INSUFFICIENT_BALANCE` | `-3` | `ErrorCode::InsufficientBalance` |
| `ERR_OUT_OF_GAS` | `-4` | `ErrorCode::OutOfGas` |
| `ERR_FORBIDDEN` | `-5` | `ErrorCode::Forbidden` |
| `ERR_ACCESS_LIST_VIOLATION` | `-6` | `ErrorCode::AccessListViolation` |
| `ERR_OUTPUT_BUFFER_TOO_SMALL` | `-7` | `ErrorCode::OutputBufferTooSmall` |
| `ERR_INVALID_ADDRESS` | `-8` | `ErrorCode::InvalidAddress` |
| `ERR_REENTRANCY_BLOCKED` | `-9` | `ErrorCode::ReentrancyBlocked` |
| `ERR_CROSS_CALL_FAILED` | `-10` | `ErrorCode::CrossCallFailed` |
| `ERR_CROSS_CALL_OUT_OF_GAS` | `-11` | `ErrorCode::CrossCallOutOfGas` |
| `ERR_VALUE_TRANSFER_NOT_PAYABLE` | `-12` | `ErrorCode::ValueTransferNotPayable` |
| `ERR_INVALID_FUNCTION_NAME` | `-13` | `ErrorCode::InvalidFunctionName` |
| `ERR_XCALL_RATE_LIMITED` | `-14` | `ErrorCode::XcallRateLimited` |
| `ERR_PARACHAIN_ONLY` | `-15` | `ErrorCode::ParachainOnly` |
| `ERR_CIPHERTEXT_INVALID` | `-16` | `ErrorCode::CiphertextInvalid` |
| `ERR_SIGNATURE_INVALID` | `-17` | `ErrorCode::SignatureInvalid` |
| `ERR_INTERNAL` | `-100` | `ErrorCode::Internal` |

```rust,no_run
use pyde_rust_sdk::types::error_code::{ErrorCode, ERR_FORBIDDEN};
# fn run() {
assert_eq!(ERR_FORBIDDEN, -5);
assert_eq!(ErrorCode::Forbidden.as_i32(), -5);
# }
```

See [Errors §9.3](09-errors.md#93-host_fn_abi-4-error-codes).

---

## 14.10 ABI versions

In `crate::types::abi::ContractAbi`:

| Constant | Value | What |
|---|---|---|
| `ContractAbi::V1_0` | `0x0001_0000` | First v1 ABI version. |
| `ContractAbi::V1_1` | `0x0001_0001` | Added optional fields (additive). |
| `ContractAbi::V1_2` | `0x0001_0002` | Current. |
| `ContractAbi::SCHEMA_V1` | `Self::V1_2` | Alias for the latest v1 minor version. |
| `ContractAbi::MAX_SUPPORTED` | `Self::V1_2` | The newest version this SDK can decode. Upgrade the SDK when a contract ships a newer ABI. |

Format: `(major << 16) | minor`. Minor bumps are additive; major
bumps require an SDK upgrade.

```rust,no_run
use pyde_rust_sdk::types::ContractAbi;

# fn run() {
assert_eq!(ContractAbi::V1_0, 0x0001_0000);
assert_eq!(ContractAbi::V1_1, 0x0001_0001);
assert_eq!(ContractAbi::V1_2, 0x0001_0002);
assert_eq!(ContractAbi::MAX_SUPPORTED, ContractAbi::V1_2);
# }
```

---

## 14.11 Function attribute bits

In `crate::types::FunctionAttrs`:

| Constant | Value | Meaning |
|---|---|---|
| `FunctionAttrs::VIEW` | `1 << 0` (1) | Read-only function — never writes state. |
| `FunctionAttrs::PAYABLE` | `1 << 1` (2) | Accepts attached PYDE value. |
| `FunctionAttrs::REENTRANT` | `1 << 2` (4) | Permitted to be re-entered from a sub-call. |
| `FunctionAttrs::SPONSORED` | `1 << 3` (8) | Gas can be paid by `FeePayer::GasTank` / `FeePayer::Paymaster`. |
| `FunctionAttrs::CONSTRUCTOR` | `1 << 4` (16) | Runs once at deploy time. |
| `FunctionAttrs::FALLBACK` | `1 << 5` (32) | Catches calls to unknown selectors. |
| `FunctionAttrs::RECEIVE` | `1 << 6` (64) | Catches plain value transfers (empty calldata). |
| `FunctionAttrs::ENTRY` | `1 << 7` (128) | Public entry point — callable from outside the contract. |

```rust,no_run
use pyde_rust_sdk::types::FunctionAttrs;

# fn run() {
assert_eq!(FunctionAttrs::VIEW, 1);
assert_eq!(FunctionAttrs::ENTRY, 128);
let combined = FunctionAttrs::ENTRY | FunctionAttrs::VIEW;
let attrs = FunctionAttrs::from_bits(combined);
assert!(attrs.has(FunctionAttrs::VIEW));
assert!(attrs.has(FunctionAttrs::ENTRY));
# }
```

Used by the `pyde.abi` custom section's `attrs.bits` field;
contract authors mark these via `[functions.<name>] attributes = [...]`
in `otigen.toml`.

---

## 14.12 Nonce window + multisig caps

In `crate::types::account`:

| Constant | Value | What |
|---|---|---|
| `NONCE_WINDOW_SIZE` | `16` | Width of the per-account nonce window. Chain accepts nonces in `[expected, expected + 15]`. |
| `MAX_MULTISIG_SIGNERS` | `16` | Cap on `AuthKeys::MultiSig::signers.len()`. |

```rust,no_run
use pyde_rust_sdk::types::account::{MAX_MULTISIG_SIGNERS, NONCE_WINDOW_SIZE};

# fn run() {
assert_eq!(NONCE_WINDOW_SIZE, 16);
assert_eq!(MAX_MULTISIG_SIGNERS, 16);
# }
```

See [Concepts §3.4](03-concepts.md#34-nonce-window-16-slots)
and [Wallets §4.8](04-wallets.md#48-auth-keys).

---

## 14.13 PYDE units

In `crate::util`:

| Constant | Value | What |
|---|---|---|
| `PYDE_DECIMALS` | `9` | `1 PYDE = 10^9 quanta`. Used by `parse_quanta` / `format_quanta`. |

```rust,no_run
use pyde_rust_sdk::util::PYDE_DECIMALS;

# fn run() {
assert_eq!(PYDE_DECIMALS, 9);
let one_pyde: u128 = 10u128.pow(PYDE_DECIMALS);
assert_eq!(one_pyde, 1_000_000_000);
# }
```

See [Utilities §13.2](13-utilities.md#132-pyde--quanta-conversion).

---

## 14.14 JSON-RPC version

In `crate::provider::json_rpc`:

| Constant | Value | What |
|---|---|---|
| `JSONRPC_VERSION` | `"2.0"` | The JSON-RPC protocol version this SDK speaks. Pinned to 2.0 per the chain's spec. |

```rust,no_run
use pyde_rust_sdk::provider::JsonRpcRequest;
# fn run() {
let req = JsonRpcRequest::new(1, "pyde_chainId", serde_json::json!([]));
assert_eq!(req.jsonrpc, "2.0");
# }
```
