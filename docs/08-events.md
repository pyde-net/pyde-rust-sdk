# 8. Events

[← back to TOC](README.md) · prev: [Contracts](07-contracts.md) · next: [Errors →](09-errors.md)

---

Pyde contracts emit `Event` records — wave/tx/event-positional
log entries with topics + data. The SDK gives you two access
paths: paginated historical query via `get_logs`, and a live
WebSocket stream via `subscribe_logs`.

## The `Event` record

```rust,ignore
pub struct Event {
    pub wave_id:     String,         // hex u64
    pub tx_index:    String,         // hex u32
    pub event_index: String,         // hex u32
    pub address:     String,         // emitting contract
    pub topics:      Vec<String>,    // up to 4, each 32-byte hex
    pub data:        String,         // hex bytes
}
```

`Event` is also re-exported as `Log` for Ethereum-vocab muscle
memory:

```rust,ignore
pub type Log = Event;
```

Topics:
- `topics[0]` is the event-signature hash (Blake3 of the
  canonical signature string), populated by the chain on emit.
- `topics[1..]` are the indexed parameter values, in declaration
  order.

## Historical query

`provider.get_logs(&filter)` returns a paginated `LogPage`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:8545")?;
let provider = Arc::new(RootProvider::new(transport));

let filter = LogFilter {
    from_wave: Some("0x0".into()),
    to_wave: None,                            // up to current head
    contracts: vec!["0xdead…".into()],        // OR-match across these
    topics: vec![],                           // no topic constraint
    cursor: None,
    limit: Some(500),
};
let page = provider.get_logs(&filter).await?;
println!("got {} events", page.entries.len());

// Resume on the next page if there's more.
if let Some(cursor) = page.next_cursor {
    let next = LogFilter { cursor: Some(cursor), ..filter };
    let _ = provider.get_logs(&next).await?;
}
# Ok(()) }
```

### Filter semantics

- **wave range** — `from_wave` ≤ `Event.wave_id` ≤ `to_wave`.
  Unset = no bound. Hex-encoded `u64`.
- **contracts** — empty = match any; otherwise OR-match across
  the listed addresses.
- **topics** — list-of-optional-list, evaluated AND-across-positions,
  OR-within-position. Mirrors EVM filter semantics:
  - `topics[i] = None` matches any topic at position `i`.
  - `topics[i] = Some(vec![a, b])` matches if the event's
    `topics[i]` is `a` OR `b`.
- **cursor** — opaque token from a prior page's `next_cursor`.
- **limit** — server-capped at 5,000; default 500.

### A typed contract makes filters easy

When you have a `Contract` instance (see
[Contracts](07-contracts.md)), it knows its address + event
signatures and can build the filter for you:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::Contract;
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("erc20-usdc", provider.clone()).await?;

// All events from this contract.
let all = provider.get_logs(&c.event_filter()).await?;

// Just one event name.
let transfers = provider.get_logs(&c.event_filter_for("Transfer")?).await?;
# Ok(()) }
```

## Live subscriptions

WebSocket subscriptions stream events as they're emitted. Only
`subscribe_logs` is wired up in v1 — the other three subscription
kinds (`subscribe_new_waves`, `subscribe_pending_txs`,
`subscribe_events`) are stubbed and return an immediate error
until the engine ships them.

```rust,no_run
use std::sync::Arc;
use futures_util::StreamExt;
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::ws::WsProvider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let provider = WsProvider::connect_ws("ws://127.0.0.1:8546").await?;

let filter = LogFilter {
    contracts: vec!["0xdead…".into()],
    ..Default::default()
};
let mut sub = provider.subscribe_logs(filter).await?;
println!("sub id: {}", sub.id());

// Pull frames one at a time:
while let Some(frame) = sub.recv().await {
    let ev = frame?;
    println!("event in wave {}: {:?}", ev.wave_id, ev.topics);
}
# Ok(()) }
```

`Subscription` exposes:

| Method | What |
|---|---|
| `recv()` | Next frame as `Option<Result<Event>>`. `None` = stream closed. |
| `into_stream()` | Convert to a `futures::Stream<Item = Result<Event>>` for combinators. |
| `id()` | The chain-assigned subscription id (echo for `pyde_unsubscribe`). |
| `unsubscribe()` | Drop the subscription cleanly. Implicit drop also works but emits no `pyde_unsubscribe` frame. |

### Stream-style consumption

```rust,no_run
use std::sync::Arc;
use futures_util::StreamExt;
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::ws::WsProvider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let provider = WsProvider::connect_ws("ws://127.0.0.1:8546").await?;
let sub = provider.subscribe_logs(LogFilter::default()).await?;

let mut stream = sub.into_stream();
while let Some(Ok(ev)) = stream.next().await {
    println!("{:?}", ev);
}
# Ok(()) }
```

`into_stream` consumes the `Subscription` — call `unsubscribe`
beforehand if you need to release the chain-side resource
cleanly.

## Decoding event data

Topics are pre-decoded (32-byte hashes / indexed values).
`Event.data` is hex-encoded borsh of the non-indexed params.
The `Contract` runtime does the full decode for you:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::{Contract, DecodedEvent};
use pyde_rust_sdk::types::Event;
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>, ev: Event) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("counter", provider).await?;
let decoded: DecodedEvent = c.decode_event(&ev)?;
println!("{}({:?})", decoded.name, decoded.params);
# Ok(()) }
```

`DecodedEvent`:

```rust,ignore
pub struct DecodedEvent {
    pub name: String,        // event name from the ABI
    pub params: Vec<(String, Value)>,   // (param_name, decoded_value)
}
```

`decode_event` returns `SdkError::InvalidArgument` if the
event's first topic doesn't match any signature in the
contract's ABI (i.e. the contract emitted an unknown event
since the ABI was bundled).

## Event signature hash

If you need the topic hash without a `Contract` (e.g. building a
filter for an event signature without holding the contract
instance):

```rust,no_run
use pyde_rust_sdk::contract::event_signature_topic;
use pyde_rust_sdk::types::EventAbi;

# fn run(event_abi: &EventAbi) {
let topic: [u8; 32] = event_signature_topic(event_abi);
# }
```

It's a `Blake3` hash of the canonical event signature string —
matches what the chain emits in `topics[0]`.
