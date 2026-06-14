# 7. Contracts

[← back to TOC](README.md) · prev: [Providers](06-providers.md) · next: [Events →](08-events.md)

---

Contracts on Pyde are WASM modules with a borsh-encoded ABI
embedded in a `pyde.abi` custom section. The SDK gives you:

1. **`TxBuilder::deploy`** to put a contract on chain.
2. **`pyde_abi!` macro** to generate compile-time typed wrappers.
3. **`Contract` runtime** to call contracts you don't know at
   compile time.

## Deployment

### Build a WASM contract

Outside the SDK — use [`otigen`](https://github.com/pyde-net/otigen):

```sh
otigen init --lang rust counter
cd counter
otigen build
# → ./artifacts/counter.bundle/contract.wasm
```

### Deploy

```rust,no_run
use pyde_rust_sdk::types::{Address, ContractType};
use pyde_rust_sdk::{Provider, Signer, TxBuilder, Wallet};

# async fn run(provider: std::sync::Arc<dyn Provider>, wallet: Wallet) -> pyde_rust_sdk::Result<()> {
let wasm = std::fs::read("./artifacts/counter.bundle/contract.wasm").unwrap();
let chain_id = provider.chain_id().await?;
let nonce = provider.get_nonce(&wallet.address()).await?;

let mut tx = TxBuilder::new()
    .from(wallet.address())
    .chain_id(chain_id)
    .nonce(nonce)
    .gas_limit(5_000_000)
    .deploy(
        "counter".to_string(),     // name — becomes the on-chain handle
        wasm,                       // bytecode
        ContractType::Contract,     // or ContractType::Parachain
        Vec::new(),                 // constructor calldata (empty for no-arg `init`)
    )?
    .build()?;
wallet.sign_tx(&mut tx).await?;

let receipt = provider.send_transaction(&tx).await?.wait_for_receipt().await?;
println!("deployed at: {}", Address::from_contract_name("counter"));
# Ok(()) }
```

`Deploy` is envelope-style:
- `tx.to = Address::ZERO` (set by `.deploy`)
- `tx.data = borsh(DeployData { name, wasm_bytes, contract_type, init_calldata })`

The deployed address is **deterministic** from the name:

```rust,no_run
use pyde_rust_sdk::types::Address;

let predicted = Address::from_contract_name("counter");
println!("will deploy at: {predicted}");
```

So you can predict the address before broadcast — useful for
ENS-style name reservation flows.

### `ContractType`

| Variant | Tag | Allowed host fns |
|---|---|---|
| `Contract::Contract` | `0x00` | §7 — standard contract host fn set |
| `Contract::Parachain` | `0x01` | §7 + §8 (parachain-only privileged fns) |

99% of deploys are `Contract`. Parachains are reserved for
opt-in scaling sidechains — see the chain spec.

## Typed wrappers via `pyde_abi!`

The SDK ships a proc-macro that reads a contract's ABI JSON at
**compile time** and generates a typed Rust wrapper:

```rust,ignore
use std::sync::Arc;
use pyde_rust_sdk::{Address, Provider, Wallet};

pyde_rust_sdk::pyde_abi!(Counter, "abi/counter.json");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider: Arc<dyn Provider> = /* construct */ todo!();
    let wallet = Wallet::generate()?;
    let counter = Counter::new(Address::from_contract_name("counter"), provider);

    // View functions become `async fn name(&self) -> Result<RetType>`
    let count: u64 = counter.get_count().await?;

    // Non-view functions take a signer + value + gas_limit
    let pending = counter.add(&wallet, 5, 200_000, 0).await?;
    let receipt = pending.wait_for_receipt().await?;
    Ok(())
}
```

What the macro generates:

| ABI function attr | Generated signature |
|---|---|
| `attributes = ["entry", "view"]` | `async fn name(&self, args...) -> Result<RetType>` — invokes `provider.call(...)` |
| `attributes = ["entry"]` (state-mutating) | `async fn name(&self, signer, args..., gas_limit, value) -> Result<PendingTx>` — builds + signs + submits |

ABI is baked in at compile time — **no runtime fetch**, no
ABI-mismatch surprises after deploy.

### Where to get the ABI JSON

`otigen build` emits `artifacts/<name>.bundle/abi.json` alongside
the WASM. Copy that into your dapp's source tree (`abi/counter.json`)
and point the macro at it.

## Dynamic contract loading

When you don't know the contract at compile time — explorers,
indexers, multi-contract wallets — use `Contract::load`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let contract = Contract::load("counter", provider).await?;

// `call` for view functions — returns `Option<Value>`.
let result = contract.call("get_count", vec![]).await?;
if let Some(Value::U64(n)) = result {
    println!("count = {n}");
}
# Ok(()) }
```

Three constructors:

```rust,ignore
// By name — looks up address via provider.resolve_name().
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

## `Value` — the dynamic type

`Value` is the SDK's encoding of an arbitrary ABI value:

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

Encode + decode helpers in [`src/contract/codec.rs`](../src/contract/codec.rs):
- `encode_value(&Value) -> Vec<u8>`
- `decode_value(&ParamType, &[u8]) -> Result<Value>`
- `encode_calldata(&FunctionAbi, &[Value]) -> Result<Vec<u8>>`
- `decode_return(&FunctionAbi, &[u8]) -> Result<Option<Value>>`

Variable-length sequences (`Bytes`, `String`, `Array`) are
capped at `MAX_DECODE_ELEMENTS = 1_000_000` on decode to make
the SDK robust against hostile RPC responses.

## State-mutating calls (dynamic)

`Contract::call(...)` is read-only (uses `provider.call`).
For state-mutating calls use `Contract::send(...)` (signs +
submits) or `Contract::build_tx(...)` (returns an unsigned `Tx`
for caller-controlled signing):

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::{Provider, Wallet};

# async fn run(provider: Arc<dyn Provider>, wallet: Wallet) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("counter", provider).await?;
let pending = c.send(
    &wallet,
    "add",
    vec![Value::U64(5)],
    200_000,   // gas_limit
    0,         // value (quanta)
).await?;
let _ = pending.wait_for_receipt().await?;
# Ok(()) }
```

`call_with` is the read-only equivalent that takes
`CallOverrides` (override `from`, `value`, `gas`) — useful for
"what would happen if X called this" diagnostics.

## Decoded events

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

`event_filter_for("Transfer")` narrows to a single event name.
See [Events](08-events.md).

## Cross-contract calls

The chain supports contracts calling other contracts via the
`pyde::call::execute` host fn (gated by gas + access list). From
the SDK side, you treat the outer contract like any other — its
`call`/`send` invocations may internally fan out, and the chain
charges gas for the whole tree.

Reserve `GAS_CROSS_CALL_ORCHESTRATOR = 500_000` (in
`src/constants.rs`) on the outer tx's `gas_limit` to leave
headroom for the wrapper.
