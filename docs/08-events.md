# 8. Events

[← back to TOC](README.md) · prev: [Contracts](07-contracts.md) · next: [Errors →](09-errors.md)

---

Pyde contracts emit `Event` records — wave/tx/event-positional
log entries with topics + data. The SDK gives you two access
paths: paginated historical query via `get_logs`, and a live
WebSocket stream via `subscribe_logs`.

## Table of contents

- [8.1 The `Event` record](#81-the-event-record)
- [8.2 Historical query](#82-historical-query)
- [8.3 Live subscriptions](#83-live-subscriptions)
- [8.4 `Subscription` API reference](#84-subscription-api-reference)
- [8.5 Decoding event data](#85-decoding-event-data)
- [8.6 Event signature topic](#86-event-signature-topic)
- [8.7 `EventFilter` (simple) vs `LogFilter` (full)](#87-eventfilter-simple-vs-logfilter-full)

---

## 8.1 The `Event` record

```rust,ignore
pub struct Event {
    pub wave_id:     String,         // hex u64
    pub tx_index:    String,         // hex u32
    pub event_index: String,         // hex u32
    pub address:     String,         // emitting contract address (hex)
    pub topics:      Vec<String>,    // up to 4 entries, each 32-byte hex
    pub data:        String,         // hex bytes (borsh of non-indexed params)
}
```

| Field | What |
|---|---|
| `wave_id` | The wave the tx that emitted this event committed in. |
| `tx_index` | The tx's position within that wave. |
| `event_index` | The event's position within the tx (one tx may emit many events). |
| `address` | Which contract emitted it (hex address). |
| `topics` | Up to 4 entries — `topics[0]` is the event-signature hash, `topics[1..]` are the indexed params. |
| `data` | Borsh-encoded non-indexed params, hex-encoded. |

### Backwards-compat alias

```rust,ignore
pub type Log = Event;
```

The `Log` alias is provided for Ethereum-vocabulary muscle
memory. New code should use `Event` directly.

### Topics layout

| Topic position | Content |
|---|---|
| `topics[0]` | Event-signature hash — Blake3 of the canonical signature string, populated by the chain on emit. |
| `topics[1]` | First indexed parameter value, in declaration order. |
| `topics[2]` | Second indexed parameter. |
| `topics[3]` | Third indexed parameter. |

Indexed params count toward the 4-slot limit; the rest of the
params (non-indexed) go in `data`.

---

## 8.2 Historical query

`provider.get_logs(&filter)` returns a paginated `LogPage`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::Provider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:9933")?;
let provider = Arc::new(RootProvider::new(transport));

let filter = LogFilter {
    from_wave: Some("0x0".into()),
    to_wave: None,                                  // up to current head
    contracts: vec!["0xdead…".into()],              // OR-match across these
    topics: vec![],                                 // no topic constraint
    cursor: None,
    limit: Some(500),
};
let page = provider.get_logs(&filter).await?;
println!("got {} events", page.entries.len());

if let Some(cursor) = page.next_cursor {
    let next = LogFilter { cursor: Some(cursor), ..filter };
    let _ = provider.get_logs(&next).await?;
}
# Ok(()) }
```

**Example output:**
```
got 12 events
```

### `LogFilter` shape

```rust,ignore
pub struct LogFilter {
    pub from_wave: Option<String>,            // hex u64, default "0x0"
    pub to_wave: Option<String>,              // hex u64, default current wave
    pub contracts: Vec<String>,               // OR-match across addresses
    pub topics: Vec<Option<Vec<String>>>,     // AND-across-positions, OR-within
    pub cursor: Option<LogCursor>,            // resume from prior page
    pub limit: Option<u64>,                   // capped at 5000, default 500
}
```

### Filter semantics

| Field | Semantics |
|---|---|
| `from_wave` / `to_wave` | `from_wave` ≤ `Event.wave_id` ≤ `to_wave`. Unset = no bound. Hex-encoded `u64`. |
| `contracts` | Empty = match any contract; non-empty = OR-match across listed addresses. |
| `topics` | List-of-optional-list, AND-across-positions, OR-within-position. Mirrors EVM filter semantics. |
| `cursor` | Opaque resume token from a prior page's `next_cursor`. |
| `limit` | Server-capped at 5000; default 500. |

### Topic-filter examples

| Filter | Matches |
|---|---|
| `topics: vec![]` | Anything (no topic constraint). |
| `topics: vec![Some(vec![hash_a])]` | Events with `topics[0] == hash_a`. |
| `topics: vec![Some(vec![hash_a, hash_b])]` | Events with `topics[0]` in `{hash_a, hash_b}`. |
| `topics: vec![None, Some(vec![alice])]` | Events with any `topics[0]`, AND `topics[1] == alice`. |

### `LogPage` shape

```rust,ignore
pub struct LogPage {
    pub entries: Vec<Event>,
    pub next_cursor: Option<LogCursor>,    // None when result fits in one page
}

pub struct LogCursor {
    pub wave_id: String,      // hex
    pub tx_index: String,     // hex
    pub event_index: String,  // hex
}
```

### Paginating

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::types::LogFilter;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let mut filter = LogFilter::default();
let mut all_events = Vec::new();
loop {
    let page = provider.get_logs(&filter).await?;
    all_events.extend(page.entries);
    match page.next_cursor {
        Some(cursor) => filter.cursor = Some(cursor),
        None => break,
    }
}
println!("collected {} events", all_events.len());
# Ok(()) }
```

### A typed contract makes filters easy

When you have a `Contract` instance (see [Contracts §7.5](07-contracts.md#75-contract-api-reference)),
it knows its address + event signatures and can build the filter
for you:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::Contract;
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let c = Contract::load("acme-token", provider.clone()).await?;

// All events from this contract.
let all = provider.get_logs(&c.event_filter()).await?;

// Just one event name.
let transfers = provider.get_logs(&c.event_filter_for("Transfer")?).await?;
# Ok(()) }
```

---

## 8.3 Live subscriptions

WebSocket subscriptions stream events as they're emitted.

### v1 support status

| Method | Status |
|---|---|
| `subscribe_logs` | ✅ Works. |
| `subscribe_new_waves` | ❌ Stubbed — returns `SdkError::Other` immediately. Engine-side gap. |
| `subscribe_pending_txs` | ❌ Stubbed. |
| `subscribe_events` | ❌ Stubbed — use `subscribe_logs` with a narrowed filter instead. |

### Subscribe to logs

```rust,no_run
use std::sync::Arc;
use futures_util::StreamExt;
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::ws::WsProvider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let provider = WsProvider::connect_ws("ws://127.0.0.1:9933/ws").await?;

let filter = LogFilter {
    contracts: vec!["0xdead…".into()],
    ..Default::default()
};
let mut sub = provider.subscribe_logs(filter).await?;
println!("sub id: {}", sub.id());

while let Some(frame) = sub.recv().await {
    let ev = frame?;
    println!("event in wave {}: {:?}", ev.wave_id, ev.topics);
}
# Ok(()) }
```

**Example output:**
```
sub id: 0x42
event in wave 1234: ["0xddf2...", "0xaaaa...", "0xbbbb..."]
event in wave 1235: ["0xddf2...", "0xcccc...", "0xdddd..."]
```

The stream runs until you call `unsubscribe`, drop the
`Subscription`, or the WS connection closes.

---

## 8.4 `Subscription` API reference

### `Subscription::recv()`

| | |
|---|---|
| Signature | `async fn recv(&mut self) -> Option<Result<Event, SdkError>>` |
| Returns | `Some(Ok(ev))` for each event; `Some(Err(_))` if the stream errored mid-flight; `None` when the stream closes cleanly. |

### `Subscription::into_stream()`

| | |
|---|---|
| Signature | `fn into_stream(self) -> impl Stream<Item = Result<Event, SdkError>>` |
| Returns | A `futures::Stream` for combinator-style consumption. |

Consumes the `Subscription` — call `unsubscribe` beforehand if
you want to clean up the chain-side resource.

```rust,no_run
use std::sync::Arc;
use futures_util::StreamExt;
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::ws::WsProvider;

# async fn run() -> pyde_rust_sdk::Result<()> {
let provider = WsProvider::connect_ws("ws://127.0.0.1:9933/ws").await?;
let sub = provider.subscribe_logs(LogFilter::default()).await?;

let mut stream = sub.into_stream();
while let Some(Ok(ev)) = stream.next().await {
    println!("{:?}", ev);
}
# Ok(()) }
```

### `Subscription::id()`

| | |
|---|---|
| Signature | `fn id(&self) -> &str` |
| Returns | The chain-assigned subscription id (used internally for `pyde_unsubscribe`). |

### `Subscription::unsubscribe()`

| | |
|---|---|
| Signature | `async fn unsubscribe(self) -> Result<(), SdkError>` |
| Returns | `Ok(())` if the unsubscribe completed cleanly; `SdkError` if the WS connection died first. |

Drop also works (the WS connection closes the sub server-side
when the registry entry goes), but doesn't emit an explicit
`pyde_unsubscribe` — preferred path is to call `unsubscribe`
explicitly.

### WS provider subscription quotas

The SDK caps the number of pending notifications it will buffer
to keep memory bounded:

| Constant | Default | What |
|---|---|---|
| `MAX_PENDING_NOTIF_BUCKETS` | `128` | Max concurrent subscriptions per `WsTransport`. |
| `MAX_PENDING_NOTIF_DEPTH` | `256` | Max queued events per subscription before the slowest reader is dropped. |

See [Constants §14.5](14-constants.md#145-codec-caps).

---

## 8.5 Decoding event data

Topics are pre-decoded (32-byte hashes / indexed values).
`Event.data` is hex-encoded borsh of the non-indexed params. The
`Contract` runtime does the full decode for you:

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

**Example output:**
```
Increment([("by", U64(5)), ("new_count", U64(42))])
```

### `DecodedEvent` shape

```rust,ignore
pub struct DecodedEvent {
    pub name: String,                       // event name from the ABI
    pub params: Vec<(String, Value)>,       // (param_name, decoded_value)
}
```

`decode_event` returns `SdkError::InvalidArgument` if the
event's first topic doesn't match any signature in the
contract's ABI (i.e. the contract emitted an unknown event
since the ABI was bundled — bumped ABI in production without
re-distributing).

---

## 8.6 Event signature topic

If you need the topic hash without holding a `Contract` instance
(e.g. building a filter for a known event signature without
loading the full ABI):

```rust,no_run
use pyde_rust_sdk::contract::event_signature_topic;
use pyde_rust_sdk::types::EventAbi;

# fn run(event_abi: &EventAbi) {
let topic: [u8; 32] = event_signature_topic(event_abi);
println!("0x{}", hex::encode(topic));
# }
```

It's a Blake3 hash of the canonical event signature string —
matches what the chain emits in `topics[0]`.

---

## 8.7 `EventFilter` (simple) vs `LogFilter` (full)

The SDK exposes two filter types:

| Type | Wire method | Capabilities |
|---|---|---|
| `EventFilter` | `pyde_getEvents` | Basic — by `address` and `wave` range. No pagination, no topic match. |
| `LogFilter` | `pyde_getLogs` | Full — multi-address, topic-position filtering, pagination via cursor. |

Use `LogFilter` for anything beyond trivial single-contract
queries. `EventFilter` is around for back-compat with simpler
explorer integrations.

```rust,ignore
pub struct EventFilter {
    pub address: Option<String>,
    pub from_wave: Option<String>,
    pub to_wave: Option<String>,
}
```

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::types::EventAbi;
use pyde_rust_sdk::types::EventFilter;
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let filter = EventFilter {
    address: Some("0xdead…".to_string()),
    from_wave: Some("0x100".into()),
    to_wave: None,
};
let events = provider.get_events(&filter).await?;
println!("got {} events", events.len());
# Ok(()) }
```
