//! `Transport` trait — abstracts over HTTP and WebSocket.
//!
//! Lets the `Provider` trait dispatch uniformly through whatever
//! channel the user has wired. The HTTP impl ships here; the WS
//! impl lives in [`crate::ws`].

use async_trait::async_trait;
use serde_json::Value;

use super::json_rpc::{JsonRpcRequest, JsonRpcResponse};
use crate::error::SdkError;

/// Abstraction over the underlying RPC channel.
///
/// HTTP is one-shot request/response. WebSocket multiplexes requests
/// + server-pushed notifications over a single long-lived
/// connection. Both surface the same `send(method, params) → Value`
/// shape; subscription wiring lives off the WS side as a separate
/// surface (`crate::ws::WsProvider`).
#[async_trait]
pub trait Transport: Send + Sync {
    /// Issue a single JSON-RPC call and return the `result` payload.
    ///
    /// # Errors
    /// - [`SdkError::Connection`] on transport-layer failure.
    /// - [`SdkError::Rpc`] / [`SdkError::InvalidArgument`] when the
    ///   server returns an error envelope.
    /// - [`SdkError::InvalidResponse`] on a malformed envelope.
    async fn send(&self, method: &str, params: Value) -> Result<Value, SdkError>;
}

/// HTTP transport built on `reqwest`.
///
/// One `reqwest::Client` per `HttpTransport`; thread-safe + cheap
/// to clone (reqwest internally uses an `Arc`-shared connection
/// pool). Holds the endpoint URL + a request-id counter.
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    next_id: std::sync::atomic::AtomicU64,
}

impl HttpTransport {
    /// Build a new transport pointing at `url`.
    ///
    /// `url` is normally an `http://` or `https://` endpoint. TLS
    /// is via `rustls` (no system TLS required); `reqwest`'s
    /// connection pool is enabled by default with a 30-second idle
    /// timeout.
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidArgument`] if `url` is not a
    /// well-formed URL.
    pub fn new(url: impl Into<String>) -> Result<Self, SdkError> {
        let client = reqwest::Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| SdkError::Connection(format!("reqwest client init: {e}")))?;
        let url = url.into();
        // Parse-validate the URL up-front so a typo surfaces here
        // rather than on the first `send`.
        url::Url::parse(&url)
            .map_err(|e| SdkError::InvalidArgument(format!("invalid RPC url: {e}")))?;
        Ok(Self {
            client,
            url,
            next_id: std::sync::atomic::AtomicU64::new(1),
        })
    }

    /// Borrow the endpoint URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    fn next_request_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, method: &str, params: Value) -> Result<Value, SdkError> {
        let req = JsonRpcRequest::new(self.next_request_id(), method, params);
        let http_resp = self
            .client
            .post(&self.url)
            .json(&req)
            .send()
            .await
            .map_err(|e| SdkError::Connection(format!("POST {}: {e}", self.url)))?;
        let status = http_resp.status();
        let body = http_resp
            .text()
            .await
            .map_err(|e| SdkError::Connection(format!("read response body: {e}")))?;
        if !status.is_success() {
            return Err(SdkError::Rpc(format!("HTTP {status}: {body}")));
        }
        let resp: JsonRpcResponse = serde_json::from_str(&body).map_err(|e| {
            SdkError::InvalidResponse(format!("decode envelope: {e}; body: {body}"))
        })?;
        resp.into_result()
    }
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn rejects_invalid_url() {
        assert!(HttpTransport::new("not a url").is_err());
    }

    #[test]
    fn accepts_valid_url() {
        let t = HttpTransport::new("http://127.0.0.1:8545").unwrap();
        assert_eq!(t.url(), "http://127.0.0.1:8545");
    }

    #[test]
    fn id_increments_per_request() {
        let t = HttpTransport::new("http://127.0.0.1:8545").unwrap();
        let a = t.next_request_id();
        let b = t.next_request_id();
        assert_eq!(b, a + 1);
    }
}
