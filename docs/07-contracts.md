# 7. Contracts

[← back to TOC](README.md) · prev: [Providers](06-providers.md) · next: [Events →](08-events.md)

---

Contracts on Pyde are WASM modules with a borsh-encoded ABI
embedded in a `pyde.abi` custom section. The SDK gives you:

1. **`TxBuilder::deploy`** to put a contract on chain.
2. **`pyde_abi!` proc-macro** to generate compile-time typed
   wrappers from an ABI JSON file.
3. **`Contract` runtime** to call contracts you don't know at
   compile time (explorers, indexers, multi-contract wallets).

## Table of contents

- [7.1 Deployment](#71-deployment)
- [7.2 `ContractType`](#72-contracttype)
- [7.3 Typed wrappers via `pyde_abi!`](#73-typed-wrappers-via-pyde_abi)
- [7.4 Dynamic loading](#74-dynamic-loading)
- [7.5 `Contract` API reference](#75-contract-api-reference)
- [7.6 The `Value` enum](#76-the-value-enum)
- [7.7 Codec functions](#77-codec-functions)
- [7.8 Decoded events](#78-decoded-events)
- [7.9 Cross-contract calls](#79-cross-contract-calls)
- [7.10 `extract_abi` — parse a deployed WASM](#710-extract_abi--parse-a-deployed-wasm)

---

## 7.1 Deployment

### Build a WASM contract

Outside the SDK — use [`otigen`](https://github.com/pyde-net/otigen):

```sh
otigen init --lang rust counter
cd counter
otigen build
```

**Expected output:**
```
✓ Compiled → ./build/contract.wasm
✓ Built "counter" → ./artifacts/counter.bundle
  wasm: 5,073 bytes (blake3 ...)
  abi:  141 bytes (blake3 ...)
```

The bundle directory contains:
```
artifacts/counter.bundle/
├── contract.wasm       ← the deploy payload
├── abi.json            ← human-readable ABI
└── manifest.json       ← bundle manifest (versions, hashes)
```

### Deploy from Rust

```rust,no_run
use pyde_rust_sdk::types::{Address, ContractType};
use pyde_rust_sdk::{Provider, Signer, TxBuilder, Wallet};

# async fn run(provider: std::sync::Arc<dyn Provider>, wallet: Wallet) -> pyde_rust_sdk::Result<()> {
let wasm = std::fs::read("./artifacts/counter.bundle/contract.wasm")?;
let chain_id = provider.chain_id().await?;
let nonce = provider.get_nonce(&wallet.address()).await?;

let mut tx = TxBuilder::new()
    .from(wallet.address())
    .chain_id(chain_id)
    .nonce(nonce)
    .gas_limit(5_000_000)
    .deploy(
        "counter".to_string(),
        wasm,
        ContractType::Contract,
        Vec::new(),                 // constructor calldata (empty for no-arg `init`)
    )?
    .build()?;
wallet.sign_tx(&mut tx).await?;

let receipt = provider.send_transaction(&tx).await?.wait_for_receipt().await?;
println!("deployed at: {}", Address::from_contract_name("counter"));
println!("status: {:?}", receipt.status);
# Ok(()) }
```

**Expected output:**
```
deployed at: 0x12345678...
status: Success
```

### Wire shape

`Deploy` is envelope-style:

| Field | Value |
|---|---|
| `tx.to` | `Address::ZERO` (set automatically by `.deploy`) |
| `tx.data` | `borsh(DeployData { name, wasm_bytes, contract_type, init_calldata })` |
| `tx.tx_type` | `TxType::Deploy = 0x01` |

The deployed address is **deterministic** from the contract
name:

```
address = Poseidon2("pyde-contract/" || name_bytes)
```

So you can predict the address before broadcast:

```rust,no_run
use pyde_rust_sdk::types::Address;

let predicted = Address::from_contract_name("counter");
println!("will deploy at: {predicted}");
```

This makes deploy-then-call orchestration straightforward — your
follow-up calls can reference the predicted address without
parsing the deploy receipt.

### Constructor calldata

If your contract declares a function with the `constructor`
attribute, `init_calldata` is what the chain feeds it at deploy
time. Encoding follows whatever convention the contract source
expects — Borsh of a typed struct, raw little-endian primitives,
ABI-style packed encoding, etc. The SDK doesn't interpret these
bytes; it just ships them.

For a no-arg constructor (or no constructor at all), pass
`Vec::new()`.

---

## 7.2 `ContractType`

```rust,ignore
#[repr(u8)]
pub enum ContractType {
    Contract = 0x00,
    Parachain = 0x01,
}
```

| Variant | Tag | Host fns allowed |
|---|---|---|
| `Contract::Contract` | `0x00` | §7 — standard contract host fn set |
| `Contract::Parachain` | `0x01` | §7 + §8 (parachain-only privileged fns) |

99% of deploys are `Contract`. Parachains are reserved for
opt-in scaling sidechains; see the chain spec.

---

## 7.3 Typed wrappers via `pyde_abi!`

The SDK ships a proc-macro that reads a contract's ABI JSON at
**compile time** and generates a typed Rust wrapper. No runtime
ABI fetch.

### Example

```rust,ignore
use std::sync::Arc;
use pyde_rust_sdk::{Address, Provider, Wallet};

pyde_rust_sdk::pyde_abi!(Counter, "abi/counter.json");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider: Arc<dyn Provider> = /* construct */ todo!();
    let wallet = Wallet::generate()?;
    let counter = Counter::new(Address::from_contract_name("counter"), provider);

    // View functions
    let count: u64 = counter.get_count().await?;
    println!("count = {count}");

    // Non-view functions
    let pending = counter.add(&wallet, 5, 200_000, 0).await?;
    let receipt = pending.wait_for_receipt().await?;
    println!("status: {:?}", receipt.status);
    Ok(())
}
```

### Generated signatures

| ABI function attr | Generated signature |
|---|---|
| `attributes = ["entry", "view"]` | `async fn name(&self, args...) -> Result<RetType>` — invokes `provider.call(...)`. |
| `attributes = ["entry"]` (state-mutating) | `async fn name(&self, signer: &dyn Signer, args..., gas_limit: u64, value: u128) -> Result<PendingTx>` — builds + signs + submits. |

| Generated method | What |
|---|---|
| `Counter::new(address, provider)` | Construct an instance pointed at an existing deployment. |
| `Counter::address() -> Address` | The contract's address. |

### Where to get the ABI JSON

`otigen build` emits `artifacts/<name>.bundle/abi.json`
alongside the WASM. Copy that file into your dapp's source tree
(typically `abi/counter.json`) and point the macro at it.

The macro re-runs on every `cargo build`; if you regenerate the
ABI (because you added or renamed a contract function),
`cargo build` picks it up automatically.

### Parameter types — Rust → ABI

| Rust type in `pyde_abi!` signature | ABI `params[i].param_type` |
|---|---|
| `u8` / `i8` | `U8` / `I8` |
| `u16` / `i16` | `U16` / `I16` |
| `u32` / `i32` | `U32` / `I32` |
| `u64` / `i64` | `U64` / `I64` |
| `u128` / `i128` | `U128` / `I128` |
| `bool` | `Bool` |
| `Address` | `Address` |
| `Vec<u8>` | `Bytes` |
| `String` | `String` |
| `Vec<T>` | `Array(Box<T>)` |
| Tuple | `Tuple(Vec<...>)` |

---

## 7.4 Dynamic loading

When you don't know the contract at compile time — explorers,
indexers, multi-contract wallets — use `Contract::load`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let contract = Contract::load("counter", provider).await?;

// `call` is for view functions — returns Option<Value>.
let result = contract.call("get_count", vec![]).await?;
if let Some(Value::U64(n)) = result {
    println!("count = {n}");
}
# Ok(()) }
```

**Expected output:**
```
count = 42
```

Three constructors:

```rust,ignore
// By name — looks up the address via provider.resolve_name().
Contract::load("counter", provider).await?;

// By raw address — useful when the name registry hasn't been
// updated yet or the contract was deployed without one.
Contract::load_at(address, provider).await?;

// You already have the ABI — pass it in directly.
Contract::new(address, abi, provider);
```

Both `load` variants fetch the WASM from chain
(`provider.get_contract_code(addr)`) and parse the `pyde.abi`
custom section.

---

## 7.5 `Contract` API reference

### `Contract::new(address, abi, provider)`

| | |
|---|---|
| Signature | `fn new(address: Address, abi: ContractAbi, provider: Arc<dyn Provider>) -> Contract` |
| Use case | You already have the ABI in hand (bundled in your dapp). |

### `Contract::load(name, provider)`

| | |
|---|---|
| Signature | `async fn load(name: &str, provider: Arc<dyn Provider>) -> Result<Contract, SdkError>` |
| Errors | `SdkError::InvalidArgument` if the name doesn't resolve (`"contract \"<name>\" is not registered"`); `SdkError::InvalidArgument` if the loaded WASM has no `pyde.abi` section. |

### `Contract::load_at(address, provider)`

| | |
|---|---|
| Signature | `async fn load_at(address: Address, provider: Arc<dyn Provider>) -> Result<Contract, SdkError>` |
| Errors | Same as `load` minus the name-resolution failure mode. |

### Accessors

| Method | Returns |
|---|---|
| `address()` | `Address` |
| `abi()` | `&ContractAbi` |
| `provider()` | `Arc<dyn Provider>` (clone) |

### `Contract::call(name, args)`

| | |
|---|---|
| Signature | `async fn call(&self, name: &str, args: Vec<Value>) -> Result<Option<Value>, SdkError>` |
| `name` | Function name from the ABI. |
| `args` | Args as `Value`, in declaration order. |
| Returns | `Some(value)` if the function has a return type, `None` for void. |
| Errors | `SdkError::InvalidArgument` if no such function or arg type mismatch. `SdkError::Reverted` if the contract reverts. |

For a view function. Uses `provider.call(...)` internally.

### `Contract::call_with(name, args, overrides)`

Same as `call` but takes a `CallOverrides` to set `from`,
`value`, or `gas` for the simulated call:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::contract::Contract;
# use pyde_rust_sdk::types::{Address, CallOverrides};
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("counter", provider).await?;
let overrides = CallOverrides {
    from: Some(Address::ZERO),
    value: None,
    gas: Some(500_000),
};
let _ = c.call_with("get_count", vec![], overrides).await?;
# Ok(()) }
```

### `Contract::send(signer, name, args, gas_limit, value)`

| | |
|---|---|
| Signature | `async fn send(&self, signer: &dyn Signer, name: &str, args: Vec<Value>, gas_limit: u64, value: u128) -> Result<PendingTx, SdkError>` |
| Returns | A `PendingTx` ready for `.wait_for_receipt()`. |

State-mutating call — builds the tx, signs it, submits via
`provider.send_transaction(...)`.

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::contract::{Contract, Value};
# use pyde_rust_sdk::{Provider, Wallet};
# async fn run(provider: Arc<dyn Provider>, wallet: Wallet) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("counter", provider).await?;
let pending = c.send(
    &wallet,
    "add",
    vec![Value::U64(5)],
    200_000,    // gas_limit
    0,          // value (quanta)
).await?;
let _ = pending.wait_for_receipt().await?;
# Ok(()) }
```

### `Contract::build_tx(signer, name, args, gas_limit, value)`

| | |
|---|---|
| Signature | `async fn build_tx(&self, signer: &dyn Signer, name: &str, args: Vec<Value>, gas_limit: u64, value: u128) -> Result<Tx, SdkError>` |
| Returns | An unsigned `Tx`. Sign + submit yourself. |

Use this when you need to inspect or modify the `Tx` between
build and submission — e.g., to set a custom `access_list`,
`deadline`, or `fee_payer`.

### `Contract::event_filter()`

| | |
|---|---|
| Signature | `fn event_filter(&self) -> LogFilter` |
| Returns | A `LogFilter` scoped to this contract's address (no topic filter). |

### `Contract::event_filter_for(event_name)`

| | |
|---|---|
| Signature | `fn event_filter_for(&self, event_name: &str) -> Result<LogFilter, SdkError>` |
| Errors | `SdkError::InvalidArgument` if the event name isn't in the ABI. |

Narrows to a single event by signature hash.

### `Contract::decode_event(&event)`

| | |
|---|---|
| Signature | `fn decode_event(&self, log: &Event) -> Result<DecodedEvent, SdkError>` |
| Returns | `DecodedEvent { name, params: Vec<(String, Value)> }`. |
| Errors | `SdkError::InvalidArgument` if the event's first topic doesn't match any signature in the ABI. |

---

## 7.6 The `Value` enum

`Value` is the SDK's runtime representation of an arbitrary ABI
value:

```rust,ignore
pub enum Value {
    Bool(bool),
    I8(i8), I16(i16), I32(i32), I64(i64), I128(i128),
    U8(u8), U16(u16), U32(u32), U64(u64), U128(u128),
    Address(Address),
    Bytes(Vec<u8>),
    String(String),
    Array(Vec<Value>),
    Tuple(Vec<Value>),
}
```

Constructing:

```rust,no_run
use pyde_rust_sdk::contract::Value;
use pyde_rust_sdk::types::Address;

let _ = Value::U64(42);
let _ = Value::Bool(true);
let _ = Value::Address(Address::ZERO);
let _ = Value::String("hello".to_string());
let _ = Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]);
let _ = Value::Array(vec![Value::U64(1), Value::U64(2), Value::U64(3)]);
let _ = Value::Tuple(vec![Value::Address(Address::ZERO), Value::U128(1_000_000_000)]);
```

Pattern-matching:

```rust,no_run
use pyde_rust_sdk::contract::Value;

# fn run(v: Value) {
match v {
    Value::U64(n) => println!("number: {n}"),
    Value::Address(a) => println!("address: {a}"),
    Value::String(s) => println!("string: {s}"),
    other => println!("other: {other:?}"),
}
# }
```

---

## 7.7 Codec functions

Lower-level codec helpers in
[`src/contract/codec.rs`](../src/contract/codec.rs):

| Function | Use case |
|---|---|
| `encode_value(&Value) -> Vec<u8>` | Borsh-encode a single value. |
| `decode_value(&ParamType, &[u8]) -> Result<Value>` | Decode a single value given its expected type. |
| `encode_calldata(&FunctionAbi, &[Value]) -> Result<Vec<u8>>` | Build full calldata: selector + encoded args. |
| `decode_return(&FunctionAbi, &[u8]) -> Result<Option<Value>>` | Decode a function's return bytes. |

Variable-length sequences (`Bytes`, `String`, `Array`) are
capped at `MAX_DECODE_ELEMENTS = 1_000_000` on decode to keep
the SDK robust against hostile RPC responses.

### Selector

A function's selector is the first 4 bytes of `Blake3` of its
canonical signature string. Held in `FunctionAbi.selector: [u8; 4]`
when the ABI is parsed.

```rust,no_run
# use pyde_rust_sdk::types::FunctionAbi;
# fn run(f: &FunctionAbi) {
let selector: [u8; 4] = f.selector;
println!("{:02x}{:02x}{:02x}{:02x}", selector[0], selector[1], selector[2], selector[3]);
# }
```

---

## 7.8 Decoded events

A `Contract` knows its ABI, so it can decode raw `Event` records
into typed `DecodedEvent`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::Contract;
use pyde_rust_sdk::types::{Event, LogFilter};
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("counter", provider.clone()).await?;

// LogFilter scoped to this contract's address.
let filter: LogFilter = c.event_filter();
let logs = provider.get_logs(&filter).await?;

for ev in logs.entries {
    let decoded = c.decode_event(&ev)?;
    println!("{}({:?})", decoded.name, decoded.params);
}
# Ok(()) }
```

**Example output:**
```
Increment([("by", U64(5)), ("new_count", U64(42))])
Decrement([("by", U64(1)), ("new_count", U64(41))])
```

`event_filter_for("Transfer")` narrows to a single event name.
See [Events §8](08-events.md).

### `DecodedEvent` shape

```rust,ignore
pub struct DecodedEvent {
    pub name: String,                  // event name from the ABI
    pub params: Vec<(String, Value)>,  // (param_name, decoded_value)
}
```

### Event signature topic

If you need the topic hash without holding a `Contract`
instance (e.g., for building a filter against an event by
signature alone):

```rust,no_run
use pyde_rust_sdk::contract::event_signature_topic;
use pyde_rust_sdk::types::EventAbi;

# fn run(event_abi: &EventAbi) {
let topic: [u8; 32] = event_signature_topic(event_abi);
println!("0x{}", hex::encode(topic));
# }
```

Blake3 of the canonical event signature string. Matches what the
chain puts in `Event.topics[0]`.

---

## 7.9 Cross-contract calls

Contracts can call other contracts via the `pyde::call::execute`
host fn — gated by gas + the calling contract's `access_list`.
From the SDK side, you treat the outer contract like any other;
its `call` / `send` invocations may internally fan out, and the
chain charges gas for the whole call tree.

### Recommended gas headroom

Reserve `GAS_CROSS_CALL_ORCHESTRATOR = 500_000` on the outer
tx's `gas_limit` to leave headroom for the wrapper:

```rust,no_run
use pyde_rust_sdk::constants::GAS_CROSS_CALL_ORCHESTRATOR;
# use pyde_rust_sdk::{Contract, Provider, Signer, Wallet};
# use pyde_rust_sdk::contract::Value;
# async fn run(c: Contract, w: Wallet) -> pyde_rust_sdk::Result<()> {
let pending = c.send(
    &w,
    "orchestrate",
    vec![],
    GAS_CROSS_CALL_ORCHESTRATOR + 200_000,  // wrapper + inner call budget
    0,
).await?;
# Ok(()) }
```

See [Constants §14.3](14-constants.md#143-gas-constants).

---

## 7.10 `extract_abi` — parse a deployed WASM

If you have a contract's WASM bytes and just want the ABI (e.g.,
for an explorer that scans `pyde_getContractCode` responses):

```rust,no_run
use pyde_rust_sdk::abi::extract_abi;
use pyde_rust_sdk::types::ContractAbi;

# fn run(wasm: &[u8]) -> pyde_rust_sdk::Result<()> {
let abi: ContractAbi = extract_abi(wasm)?;
println!("functions: {}", abi.functions.len());
println!("abi version: 0x{:08x}", abi.pyde_abi_version);
# Ok(()) }
```

### `extract_abi(wasm)`

| | |
|---|---|
| Signature | `fn extract_abi(wasm: &[u8]) -> Result<ContractAbi, SdkError>` |
| Returns | The parsed ABI. |
| Errors | `SdkError::InvalidArgument` if the WASM has no `pyde.abi` custom section, the section is malformed, or the ABI version exceeds `ContractAbi::MAX_SUPPORTED`. |

### Supported ABI versions

```rust,ignore
pub const V1_0: u32          = 0x0001_0000;
pub const V1_1: u32          = 0x0001_0001;
pub const V1_2: u32          = 0x0001_0002;
pub const MAX_SUPPORTED: u32 = Self::V1_2;
```

Minor bumps are additive (new optional fields); major bumps
require an SDK upgrade. Upgrade the SDK if you hit
`SdkError::InvalidArgument` for "unsupported ABI version".
