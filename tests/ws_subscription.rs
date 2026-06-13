//! WebSocket subscription tests against an in-process tungstenite
//! server. The server emulates the engine's frame protocol:
//! - `pyde_subscribe(["logs", filter])` → returns a sub-id.
//! - Pushes `pyde_subscription` frames for events.
//! - `pyde_unsubscribe([sub_id])` → returns `true`.
//!
//! Verifies the SDK end-to-end: subscribe → receive typed Event →
//! unsubscribe.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::WsProvider;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Start an in-process WS server that:
///   - Replies to `pyde_subscribe(["logs", _])` with sub id `"0xfeed"`.
///   - Pushes one `pyde_subscription` event.
///   - Replies to `pyde_unsubscribe(["0xfeed"])` with `true`.
///
/// Returns the `ws://` URL the server is listening on.
async fn start_mock_ws_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        // Subscribe.
        let Some(Ok(Message::Text(text))) = ws.next().await else {
            return;
        };
        let req: Value = serde_json::from_str(&text).unwrap();
        let id = req["id"].clone();
        let resp = json!({ "jsonrpc": "2.0", "id": id, "result": "0xfeed" });
        ws.send(Message::Text(resp.to_string())).await.unwrap();

        // Push one event frame.
        let event = json!({
            "jsonrpc": "2.0",
            "method": "pyde_subscription",
            "params": {
                "subscription": "0xfeed",
                "result": {
                    "wave_id": "0x1",
                    "tx_index": "0x0",
                    "event_index": "0x0",
                    "contract_addr": format!("0x{}", "ab".repeat(32)),
                    "topics": [format!("0x{}", "11".repeat(32))],
                    "data": "0xbeef"
                }
            }
        });
        ws.send(Message::Text(event.to_string())).await.unwrap();

        // Unsubscribe.
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: Value = serde_json::from_str(&text).unwrap();
            let id = req["id"].clone();
            let resp = json!({ "jsonrpc": "2.0", "id": id, "result": true });
            ws.send(Message::Text(resp.to_string())).await.unwrap();
        }
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn subscribe_logs_round_trip() {
    let url = start_mock_ws_server().await;
    let provider = WsProvider::connect_ws(&url).await.unwrap();
    let mut sub = provider.subscribe_logs(LogFilter::default()).await.unwrap();
    assert_eq!(sub.id(), "0xfeed");

    // Drain one event with a short timeout so a stalled test fails fast.
    let event = tokio::time::timeout(Duration::from_secs(2), sub.recv())
        .await
        .expect("event timed out")
        .expect("stream closed")
        .expect("decode failed");
    assert_eq!(event.wave_id_u64(), 1);
    assert_eq!(event.topics.len(), 1);

    sub.unsubscribe().await.unwrap();
}

#[tokio::test]
async fn subscribe_new_waves_rejects_in_v1() {
    // No server needed — the call short-circuits before any wire
    // traffic. Use a URL that wouldn't actually open a connection
    // and assert the connect itself bails out, then assert the
    // pre-wired guard fires when we DO connect successfully.
    let url = start_mock_ws_server().await;
    let provider = WsProvider::connect_ws(&url).await.unwrap();
    let err = provider.subscribe_new_waves().await.unwrap_err();
    assert!(matches!(err, pyde_rust_sdk::SdkError::Other(msg) if msg.contains("newWaves")));
}

#[tokio::test]
async fn subscribe_pending_txs_rejects_in_v1() {
    let url = start_mock_ws_server().await;
    let provider = WsProvider::connect_ws(&url).await.unwrap();
    let err = provider.subscribe_pending_txs().await.unwrap_err();
    assert!(
        matches!(err, pyde_rust_sdk::SdkError::Other(msg) if msg.contains("pendingTransactions"))
    );
}
