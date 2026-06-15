# 6. Providers

[← back to TOC](README.md) · prev: [Transactions](05-transactions.md) · next: [Contracts →](07-contracts.md)

---

A `Provider` is the SDK's RPC client. Pick a transport
(`HttpTransport` or `WsTransport`), wrap it in `RootProvider` /
`WsProvider`, hand it out as an `Arc<dyn Provider>` to anything
that needs to talk to the chain.

## Table of contents

- [6.1 Transports](#61-transports)
- [6.2 Building a provider](#62-building-a-provider)
- [6.3 The `Provider` trait — all 26 methods](#63-the-provider-trait--all-26-methods)
- [6.4 Calling read-only contract methods](#64-calling-read-only-contract-methods)
- [6.5 Simulating](#65-simulating)
- [6.6 Retry policy](#66-retry-policy)
- [6.7 `PendingTx`](#67-pendingtx)
- [6.8 Custom transports](#68-custom-transports)

---

## 6.1 Transports

| Transport | URL prefix | Use case |
|---|---|---|
| `HttpTransport` | `http://`, `https://` | Simple request/response over reqwest + rustls. The default. Retries transient failures automatically. |
| `WsTransport` | `ws://`, `wss://` | Persistent socket — needed for subscriptions; also fine for request/response. |

Both implement the `Transport` trait, which `RootProvider`
generic-parameterises:

```rust,ignore
pub struct RootProvider<T: Transport> { /* … */ }
```

### `HttpTransport::new(url)`

| | |
|---|---|
| Signature | `fn new(url: impl Into<String>) -> Result<HttpTransport, SdkError>` |
| `url` | `http://` or `https://` endpoint. TLS is via rustls. |
| Returns | A configured transport with retry defaults from `RetryConfig::default()`. |
| Errors | `SdkError::InvalidArgument` if `url` is malformed (caught up-front, not on first send). |

```rust,no_run
use pyde_rust_sdk::provider::HttpTransport;
# fn run() -> pyde_rust_sdk::Result<()> {
let t = HttpTransport::new("http://127.0.0.1:9933")?;
# Ok(()) }
```

### `WsTransport::connect(url)`

| | |
|---|---|
| Signature | `async fn connect(url: &str) -> Result<WsTransport, SdkError>` |
| `url` | `ws://` or `wss://` endpoint, typically `ws://host:port/ws`. |
| Returns | An open WebSocket session ready to use. Read loop is spawned in the background. |
| Errors | `SdkError::Connection` on TCP or WS handshake failure. |

```rust,no_run
use pyde_rust_sdk::ws::WsTransport;
# async fn run() -> pyde_rust_sdk::Result<()> {
let t = WsTransport::connect("ws://127.0.0.1:9933/ws").await?;
# Ok(()) }
```

Note the `/ws` path — Pyde serves WebSocket on the same port as
HTTP under that path.

---

## 6.2 Building a provider

### HTTP

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:9933")?;
let provider = Arc::new(RootProvider::new(transport));

let chain_id = provider.chain_id().await?;
let wave_id = provider.wave_id().await?;
println!("chain {chain_id}, head wave {wave_id}");
# Ok(()) }
```

**Expected output:**
```
chain 31337, head wave 1234
```

### WebSocket

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::ws::WsProvider;
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let provider = WsProvider::connect_ws("ws://127.0.0.1:9933/ws").await?;

// All Provider methods work on WS — same trait.
let chain_id = provider.chain_id().await?;
# Ok(()) }
```

`WsProvider::connect_ws` is a one-liner that internally calls
`WsTransport::connect` + wraps it in a provider.

### Convenience alias

The type alias `HttpProvider = RootProvider<HttpTransport>`
keeps signatures short:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpProvider, HttpTransport, RootProvider};

# fn run() -> pyde_rust_sdk::Result<()> {
let provider: Arc<HttpProvider> = Arc::new(
    RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?),
);
# Ok(()) }
```

---

## 6.3 The `Provider` trait — all 26 methods

Naming convention: `pyde_<camelCase>` on the wire → `snake_case`
on the trait.

### Chain info — 4 methods

#### `chain_id()`

| | |
|---|---|
| Signature | `async fn chain_id(&self) -> Result<u64, SdkError>` |
| Wire | `pyde_chainId` |
| Returns | The chain id (e.g. `31337` for devnet, custom on mainnet). |
| Errors | `Connection` / `Rpc` on transport/server failures. |

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let id = provider.chain_id().await?;
println!("{id}");
# Ok(()) }
```
**Expected output:** `31337`

#### `wave_id()`

| | |
|---|---|
| Signature | `async fn wave_id(&self) -> Result<u64, SdkError>` |
| Wire | `pyde_waveId` |
| Returns | The current head wave number. Increments at least every `--tick-ms` even with no txs. |

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let w = provider.wave_id().await?;
println!("head: {w}");
# Ok(()) }
```
**Expected output:** `head: 1234` (varies)

#### `get_node_info()`

| | |
|---|---|
| Signature | `async fn get_node_info(&self) -> Result<NodeInfo, SdkError>` |
| Wire | `pyde_getNodeInfo` |
| Returns | `NodeInfo { name, version, peer_id, … }`. |

#### `get_metrics()`

| | |
|---|---|
| Signature | `async fn get_metrics(&self) -> Result<Value, SdkError>` |
| Wire | `pyde_getMetrics` |
| Returns | Server-defined metrics blob (`serde_json::Value`). Shape varies by node implementation. |

---

### Account reads — 6 methods

#### `get_balance(addr)`

| | |
|---|---|
| Signature | `async fn get_balance(&self, addr: &Address) -> Result<u128, SdkError>` |
| Wire | `pyde_getBalance` |
| Returns | Account balance in **quanta** (`u128`). Use `format_quanta` to display. |

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::{Provider, util::format_quanta};
# use pyde_rust_sdk::types::Address;
# async fn run(provider: Arc<dyn Provider>, addr: Address) -> pyde_rust_sdk::Result<()> {
let bal = provider.get_balance(&addr).await?;
println!("{} PYDE ({} quanta)", format_quanta(bal), bal);
# Ok(()) }
```
**Expected output:** `10 PYDE (10000000000 quanta)`

#### `get_nonce(addr)`

| | |
|---|---|
| Signature | `async fn get_nonce(&self, addr: &Address) -> Result<u64, SdkError>` |
| Wire | `pyde_getNonce` |
| Returns | Next expected nonce (bottom of the 16-slot window). |

#### `get_account(addr)`

| | |
|---|---|
| Signature | `async fn get_account(&self, addr: &Address) -> Result<AccountInfo, SdkError>` |
| Wire | `pyde_getAccount` |
| Returns | `AccountInfo { account_type, balance, nonce, code_hash, auth_keys, … }`. |

**No `Option` wrapping.** The engine synthesises a fresh-EOA
fallback (`account_type: "eoa"`, `balance: 0`, `nonce: 0`,
zero code + storage roots) for addresses it has never seen.
You always get an `AccountInfo` back; you never get `None`.

If you actually need "is this a real on-chain account?" rather
than "give me the record":

```rust
# use pyde_rust_sdk::types::{Address, AccountInfo};
# fn run(info: AccountInfo) -> bool {
let is_real = info.balance > 0
    || info.nonce > 0
    || info.code_hash != [0u8; 32];
is_real
# }
```

(The fallback exists so wallets can quote a zero balance for a
brand-new recipient before any tx funds it — no separate
"address exists?" round-trip needed.)

#### `get_contract_code(addr)`

| | |
|---|---|
| Signature | `async fn get_contract_code(&self, addr: &Address) -> Result<Vec<u8>, SdkError>` |
| Wire | `pyde_getContractCode` |
| Returns | The deployed WASM bytecode. Empty vec if address has no code. |

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::{Provider, abi::extract_abi};
# use pyde_rust_sdk::types::Address;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let wasm = provider.get_contract_code(&Address::from_contract_name("counter")).await?;
let abi = extract_abi(&wasm)?;
println!("functions: {}", abi.functions.len());
# Ok(()) }
```

#### `get_storage_slot(slot)`

| | |
|---|---|
| Signature | `async fn get_storage_slot(&self, slot: &[u8; 32]) -> Result<Option<Vec<u8>>, SdkError>` |
| Wire | `pyde_getStorageSlot` |
| Returns | The 32-byte slot's value (variable-length bytes). `None` if the slot has never been written. |

The chain stores state as a flat 32-byte-key → variable-value
map (JMT). `pyde_getStorageSlot` takes the **fully-derived
slot key** — the SDK doesn't fabricate it. Contracts derive
the key at `sstore` / `sload` time inside the contract's WASM
via the `pyde.sstore` / `pyde.sload` host fns (per
HOST_FN_ABI §7.6); explorers that want to read state need to
re-derive the same key with whichever convention the contract
used.

##### Two slot-derivation conventions in use

Pyde contracts use **one of two** Poseidon2-based derivations,
depending on which toolchain compiled them. Both produce a
valid 32-byte slot key; the engine just stores values under
opaque keys and doesn't care which scheme produced them.

###### A. PIP-2 clustered keys (otigen-compiled Rust contracts)

The default for contracts scaffolded with `otigen init --lang rust`
and compiled with `otigen build`. Used by every Rust contract
the canonical templates produce. Layout (per
[PIP-2](https://github.com/pyde-net/pips/blob/main/pip-0002-clustered-state-keys.md)):

```
                  32 bytes total
┌──────────────────────────────┬──────────────────────────────┐
│ contract_address[..16]       │ Poseidon2(disc || …)[..16]   │
│ (high 16 bytes — cluster     │ (intra-contract hash —       │
│  prefix for RocksDB locality)│  low 16 bytes)               │
└──────────────────────────────┴──────────────────────────────┘
```

Per-shape derivations:

| Shape | `disc` | Intra-contract preimage |
|---|---|---|
| Primitive field (`counter: u64`) | `0x04` | `0x04 ‖ slot_index_le` (u64 LE) |
| Map entry (`balances: map<Address, u128>` at `key`) | `0x05` | `Poseidon2(0x04 ‖ slot_index_le) ‖ 0x05 ‖ map_key_bytes` |
| Nested map (`map<K1, map<K2, V>>` at `(k1, k2)`) | `0x05` | chains the map step twice |

`slot_index` is a compile-time `u64` the otigen compiler
emits per declared state field — explorers re-deriving the
key need the contract's `state_schema` (from the `pyde.abi`
custom section) to translate field-name → slot_index.

Engine source: `engine/crates/state/src/slot_key.rs`
(`storage_slot_key`, `map_entry_key`, `nested_map_entry_key`).

###### B. Field-name Poseidon2 (hand-rolled Go / C contracts)

The pattern used by hand-rolled contracts that don't go through
otigen's Rust-only slot-index assignment — including this SDK's
own `examples/contracts/access-guard/main.go`. Layout:

```
slot_key = Poseidon2(contract_address ‖ field_name_bytes ‖ key_bytes_optional)
                     32 bytes          variable           variable (empty for primitives)
```

Example from `access-guard/main.go`:

```go
var fieldAdmin   = []byte("admin")
var fieldCounter = []byte("counter")

func deriveSlot(field []byte, key []byte) [32]byte {
    var preimage [32 + 96]byte
    total := 32 + len(field) + len(key)
    self_address(int32(uintptr(unsafe.Pointer(&preimage[0]))))
    copy(preimage[32:32+len(field)],   field)
    copy(preimage[32+len(field):total], key)
    var out [32]byte
    hash_poseidon2(...)
    return out
}
```

No cluster prefix, no slot-index lookup needed. Explorers
re-deriving the key only need the contract's field-name string
+ optional map-key bytes.

##### Which convention is in use for a contract?

The contract's bundle manifest declares it. For
otigen-Rust-compiled bundles, the `state_schema` in
`pyde.abi` carries the slot-index assignments — PIP-2 clustered.
For hand-rolled Go/C contracts with no
`state_schema`-driven layout, fall back to the field-name
convention.

When in doubt: fetch the contract's WASM via `get_contract_code(addr)`,
extract the ABI via `extract_abi(wasm)` ([Contracts §7.10](07-contracts.md#710-extract_abi--parse-a-deployed-wasm)),
and inspect `state_schema` — present means PIP-2 clustered.

#### `resolve_name(name)`

| | |
|---|---|
| Signature | `async fn resolve_name(&self, name: &str) -> Result<Option<Address>, SdkError>` |
| Wire | `pyde_resolveName` |
| Returns | The address for a registered name, or `None` if unregistered. |

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
if let Some(addr) = provider.resolve_name("counter").await? {
    println!("counter at: {addr}");
} else {
    println!("no contract named 'counter'");
}
# Ok(()) }
```

---

### Transactions — 7 methods

#### `send_raw_transaction(&tx)`

| | |
|---|---|
| Signature | `async fn send_raw_transaction(&self, tx: &Tx) -> Result<TxHash, SdkError>` |
| Wire | `pyde_sendRawTransaction` |
| Returns | The canonical `tx_hash`. The tx is now in the mempool; poll for the receipt separately. |
| Errors | `SdkError::Rpc` for chain-level rejections (bad sig, nonce out of window, insufficient balance, etc.). |

#### `send_raw_encrypted_transaction(envelope_hex)`

| | |
|---|---|
| Signature | `async fn send_raw_encrypted_transaction(&self, envelope_hex: &str) -> Result<TxHash, SdkError>` |
| Wire | `pyde_sendRawEncryptedTransaction` |
| Returns | The 32-byte Blake3 envelope hash (NOT the inner plaintext `tx_hash`). |

MEV-protection sister of `send_raw_transaction`. Wallets encrypt
a plaintext `Tx` under the chain's current threshold pubkey
(from `get_threshold_public_key()`), borsh-encode the result as
an `EncryptedTxEnvelope`, and submit the bytes hex-encoded
here.

The returned hash is `Blake3(version || ciphertext_len_le ||
ciphertext)` — used for envelope-level identification only. The
inner plaintext carries its own Poseidon2 `tx_hash` that's only
knowable post-decryption, and receipts go under THAT hash; poll
`get_receipt(plaintext_hash)` or `get_transaction_receipt(plaintext_hash)`
after the wave commits.

**v1 mock-DKG warning:** check `get_threshold_public_key().scheme`
first. If it reports `"mock"` (the v1 default until real
Kyber-768 crypto lands), encrypted submissions will **sit
unprocessed**. Treat `scheme != "kyber-768"` as "encrypted path
not yet ready, fall back to plaintext."

Engine v1 size limits: min 1213 bytes, max 128 KiB. Bigger /
smaller envelopes get rejected with `SdkError::Rpc` carrying
the engine's `EncryptedAdmissionError` variant.

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let pk = provider.get_threshold_public_key().await?;
let Some(pk) = pk else {
    println!("DKG not ready, fall back to plaintext send");
    return Ok(());
};
if pk.scheme != "kyber-768" {
    println!("v1 mock — submissions will sit; using plaintext");
    return Ok(());
}
// Encrypt a Tx under pk.public_key into a borsh-encoded
// EncryptedTxEnvelope (implementation lives in pyde-crypto):
let envelope_hex = encrypt_tx_envelope(/* &tx, &pk */);
let envelope_hash = provider.send_raw_encrypted_transaction(&envelope_hex).await?;
println!("encrypted envelope submitted: {envelope_hash}");
# Ok(()) }
# fn encrypt_tx_envelope() -> String { String::new() }
```

#### `call(&CallRequest)`

| | |
|---|---|
| Signature | `async fn call(&self, req: &CallRequest) -> Result<Vec<u8>, SdkError>` |
| Wire | `pyde_call` |
| Returns | Raw return bytes from the view-call. Empty vec if the entry returns void. |

See [§6.4](#64-calling-read-only-contract-methods) for the full
`CallRequest` shape.

#### `simulate_transaction(&tx)`

| | |
|---|---|
| Signature | `async fn simulate_transaction(&self, tx: &Tx) -> Result<SimulationResult, SdkError>` |
| Wire | `pyde_simulateTransaction` |
| Returns | `SimulationResult { receipt: Option<SimulationReceipt>, access_list: SimulationAccessList }`. |

See [§6.5](#65-simulating).

#### `get_transaction_receipt(hash)`

| | |
|---|---|
| Signature | `async fn get_transaction_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError>` |
| Wire | `pyde_getTransactionReceipt` |
| Returns | The receipt if the tx has committed, `None` if still pending. |

#### `get_receipt(hash)`

| | |
|---|---|
| Signature | `async fn get_receipt(&self, hash: &TxHash) -> Result<Option<RawReceipt>, SdkError>` |
| Wire | `pyde_getReceipt` |
| Returns | A `RawReceipt` (different shape from `get_transaction_receipt` — raw-serde, see below). `None` if the receipt isn't in the consensus archive. |

**Different wire shape from `get_transaction_receipt`.** The
two methods read from different storage tiers and the engine
emits them in two distinct formats:

| Field | `Receipt` (from `get_transaction_receipt`) | `RawReceipt` (from `get_receipt`) |
|---|---|---|
| `tx_hash` | hex string | 32-byte int array |
| `wave_id` / `tx_index` / `gas_used` / `fee_paid` | hex string | raw integers |
| `status` | snake_case enum | PascalCase enum |
| `return_data` | hex string | `Vec<u8>` byte array |

Use `get_transaction_receipt` for dapp / wallet flows; use
`get_receipt` for archival queries past the hot-state TTL.
`PendingTx::wait_for_receipt` uses `get_transaction_receipt`
internally.

#### `get_tx(hash)`

| | |
|---|---|
| Signature | `async fn get_tx(&self, hash: &TxHash) -> Result<Option<Tx>, SdkError>` |
| Wire | `pyde_getTx` |
| Returns | The original `Tx` if the chain still has it; nodes may prune old txs. |

Engine emits raw-serde `Tx`; the SDK's derived `Deserialize`
on `Tx` (over `Address`, `FalconSignature`, `FeePayer` newtypes
with derived serde) accepts the raw shape directly. No separate
type needed.

---

### Waves + events — 4 methods

#### `get_wave(wave_id)`

| | |
|---|---|
| Signature | `async fn get_wave(&self, wave_id: u64) -> Result<Option<Value>, SdkError>` |
| Wire | `pyde_getWave` |
| Returns | Raw wave shape (`serde_json::Value`). Wallets / explorers parse it. |

`wave_id` is sent on the wire as a **bare JSON number**, not a
hex string — engine quirk shared with `get_hard_finality_cert`.

#### `get_hard_finality_cert(wave_id)`

| | |
|---|---|
| Signature | `async fn get_hard_finality_cert(&self, wave_id: u64) -> Result<Option<Value>, SdkError>` |
| Wire | `pyde_getHardFinalityCert` |
| Returns | The cert (raw `Value`) if the wave is finalised, `None` if not finalised / never existed. |

The bundle of ≥85 (`QUORUM`) validator signatures proving wave
`wave_id` is **hard-finalised**. Used by light clients, cross-
chain bridges, and zk-rollup verifiers that don't want to
trust the RPC node's "this is final" claim.

Returned as raw `serde_json::Value` for now. Shape:

```jsonc
{
  "commit": <WaveCommitRecord>,
  "signatures": [
    [<signer_id_u32>, [<FALCON sig bytes>]],
    ...
  ]
}
```

Like `get_wave`, the `wave_id` param goes on the wire as a
bare JSON number.

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>, wave_id: u64) -> pyde_rust_sdk::Result<()> {
match provider.get_hard_finality_cert(wave_id).await? {
    Some(cert) => {
        let sig_count = cert["signatures"].as_array().map_or(0, |a| a.len());
        println!("wave {wave_id} finalised with {sig_count} sigs");
    }
    None => println!("wave {wave_id} not yet finalised"),
}
# Ok(()) }
```

#### `get_events(&filter)`

| | |
|---|---|
| Signature | `async fn get_events(&self, filter: &EventFilter) -> Result<Vec<Event>, SdkError>` |
| Wire | `pyde_getEvents` |
| Returns | Matching events. No pagination — use `get_logs` for multi-contract / multi-topic queries. |

#### `get_logs(&filter)`

| | |
|---|---|
| Signature | `async fn get_logs(&self, filter: &LogFilter) -> Result<LogPage, SdkError>` |
| Wire | `pyde_getLogs` |
| Returns | `LogPage { entries: Vec<Event>, next_cursor: Option<LogCursor> }`. |

See [Events §8.2](08-events.md#82-historical-query) for filter
semantics + pagination.

---

### Validators + snapshots — 4 methods

#### `get_validator(addr)`

| | |
|---|---|
| Signature | `async fn get_validator(&self, addr: &Address) -> Result<Option<Value>, SdkError>` |
| Wire | `pyde_getValidator` |
| Returns | Validator record shape; `None` if address isn't a validator. |

#### `get_operator_validators(operator)`

| | |
|---|---|
| Signature | `async fn get_operator_validators(&self, operator: &Address) -> Result<Vec<Value>, SdkError>` |
| Wire | `pyde_getOperatorValidators` |
| Returns | All validators operated by the given account. |

#### `get_snapshot()`

| | |
|---|---|
| Signature | `async fn get_snapshot(&self) -> Result<Value, SdkError>` |
| Wire | `pyde_getSnapshot` |
| Returns | Full state snapshot at head (large — multi-MB). |

**Heavy.** Use only for full state sync. For light/incremental
sync use `get_snapshot_manifest` + fetch chunks on demand.

#### `get_snapshot_manifest()`

| | |
|---|---|
| Signature | `async fn get_snapshot_manifest(&self) -> Result<Value, SdkError>` |
| Wire | `pyde_getSnapshotManifest` |
| Returns | `{wave_id, state_root, chunk_size, chunk_count, chunk_hashes}`. Lightweight. |

---

### Encrypted mempool — 1 method

#### `get_threshold_public_key()`

| | |
|---|---|
| Signature | `async fn get_threshold_public_key(&self) -> Result<Option<ThresholdPublicKey>, SdkError>` |
| Wire | `pyde_getThresholdPublicKey` |
| Returns | The current DKG-epoch pubkey wallets encrypt under, or `None` if no DKG ceremony has run yet. |

Wallets need this before submitting via
[`send_raw_encrypted_transaction`](#send_raw_encrypted_transactionenvelope_hex)
for MEV protection. Per-epoch — refresh per encrypted submit
(cheap, no consensus round-trip).

`ThresholdPublicKey` shape:

```rust,ignore
pub struct ThresholdPublicKey {
    pub epoch: String,        // DKG epoch, hex string
    pub scheme: String,       // "mock" (v1 default) or "kyber-768"
    pub public_key: String,   // pubkey bytes, hex string
}
```

**v1 mock-DKG**: v1 boot writes a deterministic mock pubkey
(`scheme: "mock"`, `epoch: "0x0"`) so the encrypted-mempool
path is reachable from the first wave. Real Kyber-768 crypto
overwrites it at the per-epoch combine. Until then,
encrypted submissions **will sit unprocessed**. Always check
`scheme` before encrypting:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
match provider.get_threshold_public_key().await? {
    Some(pk) if pk.scheme == "kyber-768" => {
        // Real crypto — safe to encrypt + submit.
    }
    Some(pk) => {
        eprintln!("encrypted path not ready yet (scheme: {})", pk.scheme);
        // Fall back to plaintext send.
    }
    None => {
        eprintln!("DKG hasn't run yet — fall back to plaintext");
    }
}
# Ok(()) }
```

---

### Convenience helper (on `RootProvider`, not on the trait) — 1 method

#### `send_transaction(&tx)`

| | |
|---|---|
| Signature | `async fn send_transaction(self: &Arc<Self>, tx: &Tx) -> Result<PendingTx, SdkError>` |
| Wire | `pyde_sendRawTransaction` (then wraps the hash) |
| Returns | A `PendingTx` ready to poll for the receipt. |

This is the **one** method that's not on the `Provider` trait —
it lives on the concrete `RootProvider<T>` because returning
`PendingTx` needs `Arc<Self>`, which trait methods can't carry.

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::Tx;
# async fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
# let provider = Arc::new(RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?));
let pending = provider.send_transaction(tx).await?;
let receipt = pending.wait_for_receipt().await?;
# Ok(()) }
```

---

## 6.4 Calling read-only contract methods

`get_balance` / `get_nonce` go to chain primitives. For arbitrary
contract view-calls use `call(&CallRequest)`:

### `CallRequest` shape

```rust,ignore
pub struct CallRequest {
    pub to: String,                  // contract address, hex
    pub data: String,                // borsh CallPayload hex
    pub from: Option<String>,        // attribution address, optional
    pub value: Option<String>,       // quanta hex, optional
    pub gas: Option<String>,         // gas budget hex, optional (default 10,000,000)
}
```

All fields are string-typed because the wire shape is JSON-RPC
hex.

### Example — raw call

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::{Address, CallRequest};
# use pyde_rust_sdk::Provider;
# async fn run() -> pyde_rust_sdk::Result<()> {
# let provider = Arc::new(RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?));
let calldata = vec![/* selector || borsh-encoded args */];
let req = CallRequest {
    to: Address::ZERO.to_hex(),
    data: format!("0x{}", hex::encode(&calldata)),
    from: None,
    value: None,
    gas: None,
};
let result_bytes = provider.call(&req).await?;
println!("{} bytes returned", result_bytes.len());
# Ok(()) }
```

For typed view-calls use the [`pyde_abi!`
macro](07-contracts.md#73-typed-wrappers-via-pyde_abi) which
generates `async fn get_count(&self) -> Result<u64>`-shaped
wrappers and handles the calldata + result decoding for you.

---

## 6.5 Simulating

`simulate_transaction(&tx)` runs the tx without committing,
returning gas, observed access list, and any revert reason.

### `SimulationResult` shape

```rust,ignore
pub struct SimulationResult {
    pub receipt: Option<SimulationReceipt>,  // None if routed to no-op
    pub access_list: SimulationAccessList,   // observed reads/writes
}

pub struct SimulationReceipt {
    pub status: String,        // "Success" | "Reverted" | "OutOfGas"
    pub gas_used: String,      // hex
    pub fee_paid: String,      // quanta hex
    pub return_data: String,   // hex
}

pub struct SimulationAccessList {
    pub reads: Vec<SimulationRead>,
    pub writes: Vec<String>,  // 32-byte slot hashes, hex
}
```

### Example

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::Tx;
# use pyde_rust_sdk::Provider;
# async fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
# let provider = Arc::new(RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?));
let sim = provider.simulate_transaction(tx).await?;
if let Some(receipt) = sim.receipt {
    println!("estimated gas: {}", receipt.gas_used);
    println!("status: {}", receipt.status);
}
println!("reads: {} slots", sim.access_list.reads.len());
println!("writes: {} slots", sim.access_list.writes.len());
# Ok(()) }
```

**Expected output:**
```
estimated gas: 0x186a0
status: Success
reads: 2 slots
writes: 1 slots
```

The observed `access_list` is the most useful part for
performance-sensitive code — feed it back into the real tx's
`access_list` so the chain's scheduler can parallelise.

---

## 6.6 Retry policy

`HttpTransport` retries transient failures by default —
connection refused, TCP/TLS errors, request timeouts, and HTTP
5xx responses. Real JSON-RPC error envelopes (chain rejected
your tx) are returned immediately, since retrying won't change
the outcome.

### `RetryConfig` defaults

| Field | Default | Meaning |
|---|---|---|
| `max_retries` | `3` | Up to 3 retries after the first attempt (so 4 attempts total). |
| `base_delay` | `100 ms` | Delay before retry #1. |
| `max_delay` | `5 s` | Cap on per-attempt delay (after exponential growth + jitter). |
| `jitter_factor` | `0.25` | Actual delay is uniformly sampled from `delay × (1 ± jitter)`. |

The growth pattern at default settings: 100ms → 200ms → 400ms,
then cap.

### Customising

```rust,no_run
use std::time::Duration;
use pyde_rust_sdk::provider::{HttpTransport, RetryConfig};

# fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:9933")?
    .with_retry_config(RetryConfig {
        max_retries: 5,
        base_delay: Duration::from_millis(50),
        max_delay: Duration::from_secs(10),
        jitter_factor: 0.5,
    });
# Ok(()) }
```

### Disabling

If your dapp needs strict first-try-wins semantics:

```rust,no_run
use pyde_rust_sdk::provider::{HttpTransport, RetryConfig};

# fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:9933")?
    .with_retry_config(RetryConfig::no_retry());
# Ok(()) }
```

### What gets retried vs not

| Error | Retried? |
|---|---|
| `Connection refused` | yes |
| TLS handshake failure | yes |
| Request timeout | yes |
| HTTP 502 / 503 / 504 | yes |
| HTTP 400 / 404 (client errors) | no |
| JSON-RPC error envelope (chain rejection) | no |
| Malformed response envelope | no |

---

## 6.7 `PendingTx`

`send_transaction` returns a `PendingTx`:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::Tx;
# async fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
# let provider = Arc::new(RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?));
let pending = provider.send_transaction(tx).await?;

println!("submitted: {}", pending.hash());

// Poll until the receipt lands or the timeout trips.
let receipt = pending.wait_for_receipt().await?;
println!("wave {}, status {:?}", receipt.wave_id_u64(), receipt.status);
# Ok(()) }
```

**Expected output:**
```
submitted: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
wave 12, status Success
```

### `PendingTx` API

| Method | What |
|---|---|
| `hash() -> TxHash` | The canonical tx hash. |
| `with_poll_interval(d) -> Self` | Override poll cadence (default 1 s). |
| `with_timeout(d) -> Self` | Override total deadline (default 30 s). |
| `wait_for_receipt() -> Result<Receipt>` | Poll until the receipt lands or timeout. |

### Default constants

| Constant | Value |
|---|---|
| `DEFAULT_POLL_INTERVAL` | `250 ms` |
| `DEFAULT_TIMEOUT` | `60 s` |

See [Constants §14.6](14-constants.md#146-pending-tx-defaults).

### Tuning

```rust,no_run
# use std::sync::Arc;
# use std::time::Duration;
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use pyde_rust_sdk::types::Tx;
# async fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
# let provider = Arc::new(RootProvider::new(HttpTransport::new("http://127.0.0.1:9933")?));
let receipt = provider
    .send_transaction(tx).await?
    .with_poll_interval(Duration::from_millis(50))   // tight loop on devnet
    .with_timeout(Duration::from_secs(120))          // longer for testnet
    .wait_for_receipt().await?;
# Ok(()) }
```

### Hash cross-check

`wait_for_receipt` cross-checks the returned receipt's `tx_hash`
against the polled hash — guards against a misbehaving RPC node
mis-routing receipts. The comparison is case-insensitive and
0x-prefix tolerant.

If the chain returns a mismatched hash, `wait_for_receipt` errors
with `SdkError::InvalidResponse`.

---

## 6.8 Custom transports

`Transport` is a small trait — you can implement it to wrap a
different HTTP client, route over a Unix socket, mock for tests,
or batch JSON-RPC calls.

```rust,ignore
#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, method: &str, params: Value)
        -> Result<Value, SdkError>;
}
```

### Mock transport for tests

```rust,ignore
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use pyde_rust_sdk::error::{Result, SdkError};
use pyde_rust_sdk::provider::Transport;

struct MockTransport {
    canned: HashMap<String, Value>,
}

#[async_trait]
impl Transport for MockTransport {
    async fn send(&self, method: &str, _params: Value) -> Result<Value> {
        self.canned.get(method).cloned()
            .ok_or_else(|| SdkError::Other(format!("no canned response for {method}")))
    }
}

#[tokio::test]
async fn test_chain_id() {
    use std::sync::Arc;
    use pyde_rust_sdk::provider::RootProvider;
    use pyde_rust_sdk::Provider;

    let mut canned = HashMap::new();
    canned.insert("pyde_chainId".to_string(), json!("0x7a69"));  // 31337
    let provider = Arc::new(RootProvider::new(MockTransport { canned }));
    assert_eq!(provider.chain_id().await.unwrap(), 31337);
}
```

### Logging transport

Wrap `HttpTransport` to log every call:

```rust,ignore
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use pyde_rust_sdk::error::{Result, SdkError};
use pyde_rust_sdk::provider::{HttpTransport, Transport};

struct LoggingTransport {
    inner: Arc<HttpTransport>,
}

#[async_trait]
impl Transport for LoggingTransport {
    async fn send(&self, method: &str, params: Value) -> Result<Value> {
        let started = std::time::Instant::now();
        let result = self.inner.send(method, params.clone()).await;
        let dur = started.elapsed();
        match &result {
            Ok(_) => eprintln!("rpc {method} ok in {:?}", dur),
            Err(e) => eprintln!("rpc {method} err in {:?}: {e}", dur),
        }
        result
    }
}
```
