# 6. Providers

[← back to TOC](README.md) · prev: [Transactions](05-transactions.md) · next: [Contracts →](07-contracts.md)

---

A `Provider` is the SDK's RPC client. Pick a transport
(`HttpTransport` or `WsTransport`), wrap it in `RootProvider`,
hand it out as an `Arc<dyn Provider>` to anything that needs to
talk to the chain.

## Quickstart

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:8545")?;
let provider = Arc::new(RootProvider::new(transport));

let chain_id = provider.chain_id().await?;
let wave_id = provider.wave_id().await?;
println!("chain {chain_id}, head wave {wave_id}");
# Ok(()) }
```

## Transports

| Transport | URL prefix | Use case |
|---|---|---|
| `HttpTransport` | `http://`, `https://` | Simple request/response over reqwest + rustls. The default. |
| `WsTransport` | `ws://`, `wss://` | Persistent socket — needed for subscriptions; also fine for request/response. |

Both implement the `Transport` trait, which `RootProvider`
generic-parameterises:

```rust,ignore
pub struct RootProvider<T: Transport> { /* … */ }
```

For WebSocket:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::ws::{WsProvider, WsTransport};
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let transport = WsTransport::connect("ws://127.0.0.1:8546").await?;
let provider = Arc::new(WsProvider::new(transport));

// All Provider methods work on WS too — same trait.
let chain_id = provider.chain_id().await?;
# Ok(()) }
```

## RPC method catalogue

23 methods in total. Naming convention: `pyde_<camelCase>` on
the wire → `snake_case` on the trait.

### Chain info

| Method | Returns | Wire |
|---|---|---|
| `chain_id()` | `u64` | `pyde_chainId` |
| `wave_id()` | `u64` | `pyde_waveId` |
| `get_node_info()` | `NodeInfo` | `pyde_getNodeInfo` |
| `get_metrics()` | `Value` | `pyde_getMetrics` |

### Account reads

| Method | Returns | Wire |
|---|---|---|
| `get_balance(addr)` | `u128` quanta | `pyde_getBalance` |
| `get_nonce(addr)` | `u64` (next expected) | `pyde_getNonce` |
| `get_account(addr)` | `AccountInfo` | `pyde_getAccount` |
| `get_contract_code(addr)` | `Vec<u8>` (WASM) | `pyde_getContractCode` |
| `get_storage_slot(slot)` | `Option<Vec<u8>>` | `pyde_getStorageSlot` |
| `resolve_name(name)` | `Option<Address>` | `pyde_resolveName` |

### Transactions

| Method | Returns | Wire |
|---|---|---|
| `send_raw_transaction(&tx)` | `TxHash` | `pyde_sendRawTransaction` |
| `call(&CallRequest)` | `Vec<u8>` | `pyde_call` |
| `simulate_transaction(&tx)` | `SimulationResult` | `pyde_simulateTransaction` |
| `get_transaction_receipt(hash)` | `Option<Receipt>` | `pyde_getTransactionReceipt` |
| `get_receipt(hash)` | `Option<Receipt>` (alias) | `pyde_getReceipt` |
| `get_tx(hash)` | `Option<Tx>` | `pyde_getTransaction` |

### Wave + events

| Method | Returns | Wire |
|---|---|---|
| `get_wave(wave_id)` | `Option<Value>` (raw shape — wallets/explorers parse) | `pyde_getWave` |
| `get_events(&filter)` | `Vec<Event>` | `pyde_getEvents` |
| `get_logs(&filter)` | `LogPage` (paginated) | `pyde_getLogs` |

### Validators + snapshots

| Method | Returns | Wire |
|---|---|---|
| `get_validator(addr)` | `Option<Value>` | `pyde_getValidator` |
| `get_operator_validators(operator)` | `Vec<Value>` | `pyde_getOperatorValidators` |
| `get_snapshot()` | `Value` (heavy — full state at head) | `pyde_getSnapshot` |
| `get_snapshot_manifest()` | `Value` (`wave_id, state_root, chunks…`) | `pyde_getSnapshotManifest` |

`get_snapshot` is heavy — use it only for full state sync. For
sync-on-the-fly use `get_snapshot_manifest` + fetch chunks
on-demand (the wire shape is described in the chain spec).

### Convenience helper (on `RootProvider`)

| Method | What |
|---|---|
| `send_transaction(&tx)` | `send_raw_transaction` + wraps the returned hash in a [`PendingTx`](#pending-transactions) ready to poll. |

This is the one method that's NOT on the `Provider` trait —
it lives on the concrete `RootProvider<T>` because returning
`PendingTx` needs `Arc<Self>`, which trait methods can't carry.

## `PendingTx`

`send_transaction` returns a `PendingTx`:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::{Provider, Wallet, TxBuilder};
# async fn run() -> pyde_rust_sdk::Result<()> {
# let transport = HttpTransport::new("http://127.0.0.1:8545")?;
# let provider = Arc::new(RootProvider::new(transport));
# let mut tx = TxBuilder::new().from(pyde_rust_sdk::types::Address::ZERO).build()?;
let pending = provider.send_transaction(&tx).await?;

println!("submitted: {}", pending.hash());

// Poll until the receipt lands or the timeout / deadline trips.
let receipt = pending.wait_for_receipt().await?;
println!("wave {}, status {:?}", receipt.wave_id_u64(), receipt.status);
# Ok(()) }
```

Tune the polling cadence + deadline:

```rust,no_run
# use std::sync::Arc;
# use std::time::Duration;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::{Provider, TxBuilder};
# async fn run() -> pyde_rust_sdk::Result<()> {
# let transport = HttpTransport::new("http://127.0.0.1:8545")?;
# let provider = Arc::new(RootProvider::new(transport));
# let tx = TxBuilder::new().from(pyde_rust_sdk::types::Address::ZERO).build()?;
let receipt = provider
    .send_transaction(&tx).await?
    .with_poll_interval(Duration::from_millis(50))   // tight loop on devnet
    .with_timeout(Duration::from_secs(30))           // abort if no receipt
    .wait_for_receipt().await?;
# Ok(()) }
```

`wait_for_receipt` cross-checks the returned receipt's tx_hash
against the polled hash — guards against a misbehaving RPC node
mis-routing receipts.

## Calling read-only contract methods

`get_balance` / `get_nonce` go to chain primitives. For arbitrary
contract view-calls use `call(&CallRequest)`:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::{Address, CallRequest};
# use pyde_rust_sdk::Provider;
# async fn run() -> pyde_rust_sdk::Result<()> {
# let transport = HttpTransport::new("http://127.0.0.1:8545")?;
# let provider = Arc::new(RootProvider::new(transport));
let calldata = vec![/* selector || borsh-encoded args */];
let req = CallRequest {
    to: Address::ZERO.to_hex(),
    data: format!("0x{}", hex::encode(&calldata)),
    from: None,
    value: None,
    gas: None,
};
let result_bytes = provider.call(&req).await?;
# Ok(()) }
```

All string-typed because the wire shape is JSON-RPC hex. The
[`pyde_abi!` macro](07-contracts.md#typed-wrappers) hides this.

For typed view-calls use the [`pyde_abi!` macro](07-contracts.md#typed-wrappers)
which generates `async fn get_count(&self) -> Result<u64>`-shaped
wrappers and handles the calldata + result decoding for you.

## Simulating

`simulate_transaction(&tx)` runs the tx without committing,
returning gas, observed access list, and any revert reason.
Used by gas estimation + dry-run UIs:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::Tx;
# use pyde_rust_sdk::Provider;
# async fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
# let transport = HttpTransport::new("http://127.0.0.1:8545")?;
# let provider = Arc::new(RootProvider::new(transport));
let sim = provider.simulate_transaction(tx).await?;
if let Some(receipt) = sim.receipt {
    println!("estimated gas: {}", receipt.gas_used);
    println!("status: {}", receipt.status);
}
println!("reads: {} slots", sim.access_list.reads.len());
println!("writes: {} slots", sim.access_list.writes.len());
# Ok(()) }
```

The observed `access_list` is the most useful part for
performance-sensitive code — feed it back into the real tx's
`access_list` so the chain's scheduler can parallelise.

## Custom transports

`Transport` is a small trait:

```rust,ignore
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn request(&self, method: &str, params: Value)
        -> Result<Value, SdkError>;
}
```

Implement it to wrap a different HTTP client, route over a Unix
socket, mock for tests, or batch JSON-RPC calls. Anything that
satisfies the request/response contract works.

```rust,ignore
use async_trait::async_trait;
use serde_json::Value;
use pyde_rust_sdk::error::{Result, SdkError};
use pyde_rust_sdk::provider::Transport;

struct MockTransport {
    canned: std::collections::HashMap<String, Value>,
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, method: &str, _params: Value) -> Result<Value> {
        self.canned.get(method).cloned()
            .ok_or_else(|| SdkError::Other(format!("no canned response for {method}")))
    }
}
```

Then `RootProvider::new(MockTransport { … })` and your tests have
a deterministic Provider.
