//! WebSocket transport + event subscriptions.
//!
//! ## T7 stub
//!
//! Real implementation lands in T9 (Phase A2). This module will own:
//!
//! - [`WsProvider`] — WebSocket-backed `Provider` impl built on
//!   `tokio-tungstenite`. Implements the same trait surface as
//!   [`HttpProvider`] plus the subscription methods that only work
//!   over a persistent connection.
//! - [`Subscription<T>`] — typed handle returned by `subscribe_*`
//!   methods. Exposes `into_stream()` for `futures_util::StreamExt`
//!   consumption. Drops cleanly on `Subscription::unsubscribe()` or
//!   when the handle is dropped (best-effort; node may have already
//!   torn down the channel).
//! - Subscription kinds — wraps the node's two-method dispatcher
//!   (`pyde_subscribe` / `pyde_unsubscribe`) with typed helpers:
//!     * `subscribe_new_waves() -> Subscription<BlockHeader>`
//!     * `subscribe_logs(filter) -> Subscription<Log>`
//!     * `subscribe_events(addr, topic) -> Subscription<Event>`
//!     * `subscribe_pending_txs() -> Subscription<TxHash>`
//!
//! Reconnection strategy: exponential backoff up to a configurable cap;
//! the `Subscription<T>` stream surfaces a `Reconnected` event when the
//! underlying socket re-establishes so callers can re-fetch missed
//! state if they need at-most-once semantics.
