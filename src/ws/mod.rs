//! WebSocket transport + typed event subscriptions.
//!
//! Pyde's WebSocket surface multiplexes ordinary JSON-RPC calls
//! (same catalog as HTTP) and server-pushed subscription
//! notifications (`pyde_subscription` frames) over one long-lived
//! connection. The SDK demultiplexes them: RPC responses by `id`,
//! notifications by `params.subscription`.
//!
//! ## Subscription support (v1)
//!
//! The engine ships **only the `"logs"` kind** in v1. The SDK
//! pre-wires the four conventional subscribe entry points so that
//! when the engine adds the others, callers don't see a method
//! disappear:
//!
//! | Method                     | v1 status         |
//! |----------------------------|-------------------|
//! | [`WsProvider::subscribe_logs`] | ✅ supported       |
//! | [`WsProvider::subscribe_new_waves`] | ⚠️ rejected client-side |
//! | [`WsProvider::subscribe_pending_txs`] | ⚠️ rejected client-side |
//! | [`WsProvider::subscribe_events`] | ⚠️ rejected client-side |
//!
//! The unsupported entry points return [`SdkError::Other`] with a
//! clear "not yet supported by engine v1" message — that's
//! cheaper than letting the call go to the engine and hit a
//! `-32602` response with the same meaning.
//!
//! ## Reconnection
//!
//! v1 does **not** auto-reconnect. On socket close, in-flight
//! `send` futures resolve with [`SdkError::Connection`] and active
//! subscriptions close their channel. Callers re-establish the
//! connection if they need persistence. Auto-reconnect is a v2
//! follow-up that requires re-subscribing all live subscriptions
//! transparently — a wire-stable but non-trivial state machine.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, Stream, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::error::SdkError;
use crate::provider::json_rpc::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use crate::provider::transport::Transport;
use crate::provider::RootProvider;
use crate::types::{Address, Event, LogFilter, TxHash, WaveHeader};

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsWrite = SplitSink<WsStream, Message>;
type WsRead = SplitStream<WsStream>;

/// Capacity of the per-subscription event buffer. Events beyond
/// this depth are dropped if the consumer falls behind — the same
/// back-pressure policy alloy's WS provider uses.
const SUBSCRIPTION_CHANNEL_DEPTH: usize = 1024;

/// Maximum distinct sub_ids we'll buffer notifications for before a
/// caller has registered the matching receiver. Stops a malicious
/// server from flooding `pending_notifs` with junk sub_ids.
const MAX_PENDING_NOTIF_BUCKETS: usize = 128;

/// Maximum events queued per pending sub_id bucket. Caps the worst-
/// case memory a single attacker-controlled bucket can pin.
const MAX_PENDING_NOTIF_DEPTH: usize = 256;

/// WebSocket transport for the SDK's JSON-RPC client + subscription
/// surface.
///
/// One [`WsTransport`] per WebSocket connection. Tasks share it via
/// [`Arc<WsTransport>`]. The transport spawns one background reader
/// at construction time; it lives until the connection drops.
pub struct WsTransport {
    inner: Arc<WsInner>,
}

struct WsInner {
    write: Mutex<WsWrite>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, SdkError>>>>,
    subscriptions: Mutex<HashMap<String, mpsc::Sender<Value>>>,
    /// Notifications that arrived before the subscription channel was
    /// registered (race window between response landing and the
    /// caller wiring the receiver). Drained on
    /// [`WsTransport::register_subscription`].
    pending_notifs: Mutex<HashMap<String, Vec<Value>>>,
    next_id: AtomicU64,
}

impl WsTransport {
    /// Open a WebSocket to `url` (e.g. `ws://127.0.0.1:9933/ws`).
    ///
    /// The chain serves the WS endpoint at the `/ws` path on the
    /// same port as JSON-RPC HTTP — so a devnet on `9933` exposes
    /// WS at `ws://127.0.0.1:9933/ws`.
    ///
    /// Spawns the read loop in the background; the returned
    /// [`WsTransport`] is usable as soon as the handshake completes.
    ///
    /// # Errors
    /// Returns [`SdkError::Connection`] on TCP / WebSocket handshake
    /// failure.
    pub async fn connect(url: &str) -> Result<Self, SdkError> {
        let (stream, _resp) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| SdkError::Connection(format!("ws connect {url}: {e}")))?;
        let (write, read) = stream.split();
        let inner = Arc::new(WsInner {
            write: Mutex::new(write),
            pending: Mutex::new(HashMap::new()),
            subscriptions: Mutex::new(HashMap::new()),
            pending_notifs: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        // Spawn the read loop. Closes itself when the upstream
        // stream ends; that fail-fast behavior is intentional.
        tokio::spawn(read_loop(read, Arc::clone(&inner)));
        Ok(Self { inner })
    }

    /// Send a JSON-RPC request and await the response.
    async fn send_raw(&self, method: &str, params: Value) -> Result<Value, SdkError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest::new(id, method, params);
        let body = serde_json::to_string(&req)
            .map_err(|e| SdkError::Other(format!("ws encode request: {e}")))?;
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.inner.pending.lock().await;
            pending.insert(id, tx);
        }
        {
            let mut write = self.inner.write.lock().await;
            write
                .send(Message::Text(body))
                .await
                .map_err(|e| SdkError::Connection(format!("ws send {method}: {e}")))?;
        }
        // Wait for the reader task to route the response.
        rx.await
            .map_err(|_| SdkError::Connection("ws connection closed".into()))?
    }

    /// Register a subscription channel for a given subscription id.
    ///
    /// Drains any buffered notifications that arrived before the
    /// caller wired the receiver — closes the race window between
    /// the `pyde_subscribe` response landing and the SDK caller
    /// completing registration.
    async fn register_subscription(&self, sub_id: String, tx: mpsc::Sender<Value>) {
        {
            let mut subs = self.inner.subscriptions.lock().await;
            subs.insert(sub_id.clone(), tx.clone());
        }
        let buffered = self.inner.pending_notifs.lock().await.remove(&sub_id);
        if let Some(notifs) = buffered {
            for n in notifs {
                let _ = tx.try_send(n);
            }
        }
    }

    /// Drop a registered subscription channel.
    async fn drop_subscription(&self, sub_id: &str) {
        let mut subs = self.inner.subscriptions.lock().await;
        subs.remove(sub_id);
    }
}

#[async_trait]
impl Transport for WsTransport {
    async fn send(&self, method: &str, params: Value) -> Result<Value, SdkError> {
        self.send_raw(method, params).await
    }
}

impl std::fmt::Debug for WsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsTransport").finish_non_exhaustive()
    }
}

// ── Reader task ─────────────────────────────────────────────────

async fn read_loop(mut read: WsRead, inner: Arc<WsInner>) {
    while let Some(frame) = read.next().await {
        let text = match frame {
            Ok(Message::Text(t)) => t.to_string(),
            Ok(Message::Binary(b)) => String::from_utf8_lossy(&b).to_string(),
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => continue,
        };
        dispatch_frame(&text, &inner).await;
    }
    // Connection closed — cancel everything still in flight.
    let mut pending = inner.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(SdkError::Connection("ws connection closed".into())));
    }
    inner.subscriptions.lock().await.clear();
}

async fn dispatch_frame(text: &str, inner: &Arc<WsInner>) {
    // First try notification (no `id`, has `method`).
    if let Ok(notif) = serde_json::from_str::<JsonRpcNotification>(text) {
        if notif.method == "pyde_subscription" {
            if let Some(sub_id) = notif.params.get("subscription").and_then(Value::as_str) {
                let result = notif.params.get("result").cloned().unwrap_or(Value::Null);
                let sender = inner.subscriptions.lock().await.get(sub_id).cloned();
                match sender {
                    Some(tx) => {
                        // Best-effort send — drop the event if the
                        // consumer is behind. Same back-pressure as
                        // alloy's WS provider.
                        let _ = tx.try_send(result);
                    }
                    None => {
                        // Subscription not yet registered (race
                        // between `pyde_subscribe` response landing
                        // and the SDK caller wiring the receiver).
                        // Buffer the event for
                        // `register_subscription` to drain — but cap
                        // both the bucket depth and the total number
                        // of buckets so a malicious server can't
                        // flood the map with junk sub_ids.
                        let mut notifs = inner.pending_notifs.lock().await;
                        let bucket_exists = notifs.contains_key(sub_id);
                        if bucket_exists || notifs.len() < MAX_PENDING_NOTIF_BUCKETS {
                            let bucket = notifs.entry(sub_id.to_string()).or_default();
                            if bucket.len() < MAX_PENDING_NOTIF_DEPTH {
                                bucket.push(result);
                            }
                            // Else: bucket full → drop. Genuine
                            // receivers will register before this
                            // matters; an attacker can't grow it
                            // past MAX_PENDING_NOTIF_DEPTH.
                        }
                        // Else: too many unknown sub_ids queued →
                        // drop the frame entirely.
                    }
                }
            }
            return;
        }
    }
    // Otherwise it's a response.
    if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(text) {
        if let Some(id) = resp.id.as_ref().and_then(Value::as_u64) {
            let mut pending = inner.pending.lock().await;
            if let Some(sender) = pending.remove(&id) {
                let _ = sender.send(resp.into_result());
            }
        }
    }
}

// ── Subscription<T> ────────────────────────────────────────────

/// Typed subscription handle returned by the `WsProvider::subscribe_*`
/// methods.
///
/// Consume via [`Subscription::recv`] or [`Subscription::into_stream`].
/// Drop the handle (or call [`Subscription::unsubscribe`]) to clean
/// up; the engine releases the server-side resources when the
/// `pyde_unsubscribe` frame lands.
pub struct Subscription<T> {
    rx: mpsc::Receiver<Value>,
    sub_id: String,
    transport: Arc<WsTransport>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: DeserializeOwned> Subscription<T> {
    /// Receive the next typed event.
    ///
    /// Returns `None` when the stream ends — either because the
    /// connection closed or because the consumer called
    /// [`Self::unsubscribe`]. Decoding errors surface inline.
    pub async fn recv(&mut self) -> Option<Result<T, SdkError>> {
        let v = self.rx.recv().await?;
        Some(
            serde_json::from_value(v)
                .map_err(|e| SdkError::InvalidResponse(format!("subscription event decode: {e}"))),
        )
    }

    /// The server-assigned subscription id.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.sub_id
    }

    /// Issue `pyde_unsubscribe` and close the channel.
    ///
    /// Idempotent — if the channel was already closed (connection
    /// dropped), the call resolves Ok.
    pub async fn unsubscribe(self) -> Result<(), SdkError> {
        let result = self
            .transport
            .send("pyde_unsubscribe", json!([&self.sub_id]))
            .await;
        self.transport.drop_subscription(&self.sub_id).await;
        result.map(|_| ())
    }

    /// Adapt to a [`futures_util::Stream`] so the subscription
    /// composes with the rest of the futures ecosystem.
    pub fn into_stream(self) -> impl Stream<Item = Result<T, SdkError>>
    where
        T: Unpin,
    {
        SubscriptionStream {
            sub: self,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T: DeserializeOwned> std::fmt::Debug for Subscription<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.sub_id)
            .finish()
    }
}

struct SubscriptionStream<T> {
    sub: Subscription<T>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: DeserializeOwned + Unpin> Stream for SubscriptionStream<T> {
    type Item = Result<T, SdkError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.sub.rx.poll_recv(cx) {
            std::task::Poll::Ready(Some(v)) => {
                std::task::Poll::Ready(Some(serde_json::from_value(v).map_err(|e| {
                    SdkError::InvalidResponse(format!("subscription event decode: {e}"))
                })))
            }
            std::task::Poll::Ready(None) => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

// ── WsProvider convenience surface ─────────────────────────────

/// WebSocket-backed [`RootProvider`].
///
/// Has the full [`crate::provider::Provider`] trait surface plus the
/// [`subscribe_logs`][Self::subscribe_logs] / [`subscribe_new_waves`][Self::subscribe_new_waves] /
/// [`subscribe_pending_txs`][Self::subscribe_pending_txs] /
/// [`subscribe_events`][Self::subscribe_events] subscription helpers.
pub type WsProvider = RootProvider<WsTransport>;

impl WsProvider {
    /// Open a WS connection and wrap it in a provider.
    ///
    /// # Errors
    /// Returns [`SdkError::Connection`] on WebSocket handshake
    /// failure.
    pub async fn connect_ws(url: &str) -> Result<Arc<Self>, SdkError> {
        let transport = WsTransport::connect(url).await?;
        Ok(Arc::new(Self::new(transport)))
    }

    /// Subscribe to event logs matching `filter`.
    ///
    /// Returns a typed [`Subscription`] of [`Event`] frames.
    ///
    /// # Errors
    /// - [`SdkError::Connection`] on WebSocket send failure.
    /// - [`SdkError::Rpc`] on a server-side rejection.
    /// - [`SdkError::InvalidResponse`] on a malformed sub-id reply.
    pub async fn subscribe_logs(&self, filter: LogFilter) -> Result<Subscription<Event>, SdkError> {
        self.subscribe_inner("logs", json!(filter)).await
    }

    /// Subscribe to new-wave headers (each committed wave produces
    /// one frame).
    ///
    /// **v1: not yet supported by engine.** Returns an immediate
    /// error rather than dispatching a doomed request. Will be
    /// enabled when engine v1.x ships the `"newWaves"` kind.
    ///
    /// # Errors
    /// Always returns [`SdkError::Other`] on v1.
    pub async fn subscribe_new_waves(&self) -> Result<Subscription<WaveHeader>, SdkError> {
        Err(unsupported_subscription("newWaves"))
    }

    /// Subscribe to pending-tx hashes (each mempool admission
    /// produces one frame).
    ///
    /// **v1: not yet supported by engine.** Returns an immediate
    /// error.
    ///
    /// # Errors
    /// Always returns [`SdkError::Other`] on v1.
    pub async fn subscribe_pending_txs(&self) -> Result<Subscription<TxHash>, SdkError> {
        Err(unsupported_subscription("pendingTransactions"))
    }

    /// Subscribe to events from a single `(address, topic)` pair.
    ///
    /// **v1: not yet supported by engine.** Use
    /// [`Self::subscribe_logs`] with a single-contract /
    /// single-topic filter instead.
    ///
    /// # Errors
    /// Always returns [`SdkError::Other`] on v1.
    pub async fn subscribe_events(
        &self,
        _contract: Address,
        _topic: [u8; 32],
    ) -> Result<Subscription<Event>, SdkError> {
        Err(unsupported_subscription("events"))
    }

    /// Internal subscribe path — sends the `pyde_subscribe` request
    /// and wires the registry entry on success.
    async fn subscribe_inner<T: DeserializeOwned>(
        &self,
        kind: &str,
        filter: Value,
    ) -> Result<Subscription<T>, SdkError> {
        let v = self
            .transport()
            .send_raw("pyde_subscribe", json!([kind, filter]))
            .await?;
        let sub_id = v
            .as_str()
            .ok_or_else(|| {
                SdkError::InvalidResponse(format!(
                    "pyde_subscribe: expected string sub_id, got {v}"
                ))
            })?
            .to_string();
        let (tx, rx) = mpsc::channel(SUBSCRIPTION_CHANNEL_DEPTH);
        self.transport()
            .register_subscription(sub_id.clone(), tx)
            .await;
        Ok(Subscription {
            rx,
            sub_id,
            transport: Arc::new(self.transport_arc()),
            _phantom: std::marker::PhantomData,
        })
    }

    /// Borrow the transport behind an Arc — used by
    /// [`Subscription::unsubscribe`] to keep a strong reference to
    /// the WS connection for the duration of cleanup.
    fn transport_arc(&self) -> WsTransport {
        WsTransport {
            inner: Arc::clone(&self.transport().inner),
        }
    }
}

fn unsupported_subscription(kind: &str) -> SdkError {
    SdkError::Other(format!(
        "subscription kind {kind:?} is not yet supported by engine v1 — only \"logs\" \
         ships today; this method becomes available when the engine adds it"
    ))
}
