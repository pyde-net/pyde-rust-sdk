//! `Transport` trait — abstracts over HTTP and WebSocket.
//!
//! Lets the `Provider` trait dispatch uniformly through whatever
//! channel the user has wired. The HTTP impl ships here; the WS
//! impl lives in [`crate::ws`].

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

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

// ── Retry policy ─────────────────────────────────────────────────────

/// Retry policy for [`HttpTransport`] requests.
///
/// Defaults: up to 3 retries, 100 ms base delay, doubled per attempt,
/// capped at 5 s, with ±25% jitter to avoid thundering-herd
/// retry storms when many clients hit the same transient failure.
///
/// Only **transient** failures retry: connection refused, TCP / TLS
/// errors, request timeouts, and HTTP 5xx responses. Real JSON-RPC
/// error envelopes (`{"error": {…}}`) are returned to the caller
/// immediately — those mean the chain processed the request and
/// rejected it, which retrying won't change.
#[derive(Debug, Clone, Copy)]
pub struct RetryConfig {
    /// Maximum number of *additional* attempts after the first one.
    /// `max_retries = 0` disables retry. Default `3`.
    pub max_retries: u32,
    /// Initial back-off before retry #1. Default 100 ms.
    pub base_delay: Duration,
    /// Cap on per-attempt back-off (after exponential growth + jitter).
    /// Default 5 s.
    pub max_delay: Duration,
    /// Jitter factor in the range `[0.0, 1.0]`. The actual delay is
    /// uniformly sampled from `delay * (1 - jitter) … delay * (1 + jitter)`.
    /// Default `0.25` (±25%).
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            jitter_factor: 0.25,
        }
    }
}

impl RetryConfig {
    /// Disable retries — fail on the first transient error.
    /// Useful for tests + scripts that want a strict "first try wins"
    /// failure mode.
    #[must_use]
    pub const fn no_retry() -> Self {
        Self {
            max_retries: 0,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            jitter_factor: 0.0,
        }
    }

    /// Compute the delay before retry attempt `attempt` (0-indexed
    /// among retries, so `attempt=0` is the delay before the first
    /// retry — i.e., after the original attempt failed).
    fn delay_for(&self, attempt: u32) -> Duration {
        let base = self.base_delay.as_millis() as f64;
        let exp = base * 2_f64.powi(attempt as i32);
        let capped = exp.min(self.max_delay.as_millis() as f64);

        let jitter_low = capped * (1.0 - self.jitter_factor);
        let jitter_high = capped * (1.0 + self.jitter_factor);
        let sampled = jitter_low + rand::random::<f64>() * (jitter_high - jitter_low);
        Duration::from_millis(sampled.max(0.0) as u64)
    }
}

/// Classify whether an error from a single HTTP attempt is worth
/// retrying.
///
/// Retry on:
/// - `Connection` errors (TCP / TLS / refused / timeout)
/// - HTTP 5xx (server-side intermittent)
/// - HTTP 429 (rate-limited; honor server's `Wait for Ns` hint
///   if present — see [`parse_retry_after_secs`])
///
/// Don't retry on:
/// - HTTP 4xx other than 429 (real client error — won't change)
/// - JSON-RPC error envelopes (chain rejected — won't change)
/// - Malformed-response errors (server bug — won't change)
fn is_transient(err: &SdkError) -> bool {
    matches!(err, SdkError::Connection(_))
        || matches!(err, SdkError::Rpc(msg) if msg.starts_with("HTTP 5"))
        || matches!(err, SdkError::Rpc(msg) if msg.starts_with("HTTP 429"))
}

/// Parse the engine's `"Wait for Ns"` retry-after hint from a
/// rate-limit error message. Returns `None` if the pattern isn't
/// present.
///
/// Engine emits 429 bodies shaped like
/// `"HTTP 429 Too Many Requests: Too Many Requests! Wait for 80s"`.
/// The retry loop uses this to override exponential backoff with
/// the server's explicit hint when available.
fn parse_retry_after_secs(msg: &str) -> Option<u64> {
    let after = msg.find("Wait for ")?;
    let rest = &msg[after + "Wait for ".len()..];
    let end = rest.find('s')?;
    rest[..end].parse::<u64>().ok()
}

// ── HttpTransport ────────────────────────────────────────────────────

/// HTTP transport built on `reqwest`.
///
/// One `reqwest::Client` per `HttpTransport`; thread-safe + cheap
/// to clone (reqwest internally uses an `Arc`-shared connection
/// pool). Holds the endpoint URL + a request-id counter + a
/// configurable retry policy ([`RetryConfig`]).
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    next_id: std::sync::atomic::AtomicU64,
    retry: RetryConfig,
}

impl HttpTransport {
    /// Build a new transport pointing at `url`.
    ///
    /// `url` is normally an `http://` or `https://` endpoint. TLS
    /// is via `rustls` (no system TLS required); `reqwest`'s
    /// connection pool is enabled by default with a 30-second idle
    /// timeout. Retry policy defaults to [`RetryConfig::default`];
    /// override via [`Self::with_retry_config`].
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
            retry: RetryConfig::default(),
        })
    }

    /// Borrow the endpoint URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Replace the retry policy. Builder-style.
    ///
    /// ```rust,no_run
    /// use std::time::Duration;
    /// use pyde_rust_sdk::provider::{HttpTransport, RetryConfig};
    ///
    /// # fn run() -> pyde_rust_sdk::Result<()> {
    /// let transport = HttpTransport::new("http://127.0.0.1:9933")?
    ///     .with_retry_config(RetryConfig {
    ///         max_retries: 5,
    ///         base_delay: Duration::from_millis(50),
    ///         max_delay: Duration::from_secs(10),
    ///         jitter_factor: 0.5,
    ///     });
    /// # Ok(()) }
    /// ```
    #[must_use]
    pub fn with_retry_config(mut self, retry: RetryConfig) -> Self {
        self.retry = retry;
        self
    }

    /// Borrow the current retry policy.
    #[must_use]
    pub fn retry_config(&self) -> &RetryConfig {
        &self.retry
    }

    fn next_request_id(&self) -> u64 {
        self.next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }

    /// One attempt — no retry. Used inside [`Self::send`]'s loop.
    async fn send_once(&self, method: &str, params: Value) -> Result<Value, SdkError> {
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

/// Cap on how long the 429 "Wait for Ns" hint can override the
/// configured `max_delay`. Stops a hostile server from forcing
/// arbitrarily-long sleeps; 60 s is generous enough for any
/// legitimate rate-limit window.
const MAX_RETRY_AFTER_SECS: u64 = 60;

#[async_trait]
impl Transport for HttpTransport {
    async fn send(&self, method: &str, params: Value) -> Result<Value, SdkError> {
        let mut last_err: Option<SdkError> = None;
        let mut server_hint_delay: Option<Duration> = None;
        for attempt in 0..=self.retry.max_retries {
            // The first iteration (attempt = 0) is the original
            // request; subsequent iterations are retries — each
            // preceded by either the server's `Retry-After` hint
            // (if 429) or our exponentially backed-off + jittered
            // sleep.
            if attempt > 0 {
                let delay = server_hint_delay
                    .take()
                    .unwrap_or_else(|| self.retry.delay_for(attempt - 1));
                tokio::time::sleep(delay).await;
            }
            // Need to clone params per attempt because send_once
            // takes it by value (serde_json::Value is cheap to
            // clone — refcounted strings + small Vecs).
            match self.send_once(method, params.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) if is_transient(&e) => {
                    // On 429: extract the server's `Wait for Ns`
                    // hint and use it as the NEXT sleep duration
                    // (capped at MAX_RETRY_AFTER_SECS to bound
                    // worst case). Falls back to exp backoff if
                    // the hint isn't present / malformed.
                    if let SdkError::Rpc(msg) = &e {
                        if let Some(secs) = parse_retry_after_secs(msg) {
                            let capped = secs.min(MAX_RETRY_AFTER_SECS);
                            server_hint_delay = Some(Duration::from_secs(capped));
                        }
                    }
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        // Loop exhausted retries — return the last transient error
        // with a hint so users know retry was attempted.
        Err(last_err.unwrap_or_else(|| {
            SdkError::Connection(format!(
                "all {} retries exhausted on {method}",
                self.retry.max_retries
            ))
        }))
    }
}

impl std::fmt::Debug for HttpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpTransport")
            .field("url", &self.url)
            .field("retry", &self.retry)
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
        let t = HttpTransport::new("http://127.0.0.1:9933").unwrap();
        assert_eq!(t.url(), "http://127.0.0.1:9933");
    }

    #[test]
    fn id_increments_per_request() {
        let t = HttpTransport::new("http://127.0.0.1:9933").unwrap();
        let a = t.next_request_id();
        let b = t.next_request_id();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn default_retry_config_has_3_retries() {
        let t = HttpTransport::new("http://127.0.0.1:9933").unwrap();
        assert_eq!(t.retry_config().max_retries, 3);
        assert_eq!(t.retry_config().base_delay, Duration::from_millis(100));
        assert_eq!(t.retry_config().max_delay, Duration::from_secs(5));
        assert!((t.retry_config().jitter_factor - 0.25).abs() < 1e-9);
    }

    #[test]
    fn no_retry_config_disables_retries() {
        let cfg = RetryConfig::no_retry();
        assert_eq!(cfg.max_retries, 0);
        assert_eq!(cfg.base_delay, Duration::from_millis(0));
    }

    #[test]
    fn with_retry_config_overrides_default() {
        let custom = RetryConfig {
            max_retries: 7,
            base_delay: Duration::from_millis(25),
            max_delay: Duration::from_secs(2),
            jitter_factor: 0.5,
        };
        let t = HttpTransport::new("http://127.0.0.1:9933")
            .unwrap()
            .with_retry_config(custom);
        assert_eq!(t.retry_config().max_retries, 7);
        assert_eq!(t.retry_config().base_delay, Duration::from_millis(25));
    }

    #[test]
    fn delay_grows_exponentially_within_cap() {
        let cfg = RetryConfig {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            jitter_factor: 0.0, // disable jitter for deterministic math
        };
        // attempt 0: 100 ms
        // attempt 1: 200 ms
        // attempt 2: 400 ms
        // attempt 3: 800 ms
        // attempt 4: 1600 ms
        // attempt 5: 3200 ms
        // attempt 6: 5000 ms (capped at max_delay)
        assert_eq!(cfg.delay_for(0), Duration::from_millis(100));
        assert_eq!(cfg.delay_for(1), Duration::from_millis(200));
        assert_eq!(cfg.delay_for(2), Duration::from_millis(400));
        assert_eq!(cfg.delay_for(5), Duration::from_millis(3200));
        assert_eq!(cfg.delay_for(6), Duration::from_millis(5000));
        // Way past — still capped.
        assert_eq!(cfg.delay_for(20), Duration::from_millis(5000));
    }

    #[test]
    fn jittered_delay_stays_within_bounds() {
        let cfg = RetryConfig {
            max_retries: 5,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            jitter_factor: 0.25,
        };
        // attempt 0: base 100ms → 75..125ms with 25% jitter.
        for _ in 0..100 {
            let d = cfg.delay_for(0);
            assert!(
                d >= Duration::from_millis(74) && d <= Duration::from_millis(126),
                "delay {:?} outside jitter window [74..126]",
                d
            );
        }
    }

    #[test]
    fn is_transient_classifies_connection_as_retryable() {
        assert!(is_transient(&SdkError::Connection("refused".into())));
    }

    #[test]
    fn is_transient_classifies_5xx_as_retryable() {
        assert!(is_transient(&SdkError::Rpc("HTTP 502: bad gateway".into())));
        assert!(is_transient(&SdkError::Rpc(
            "HTTP 503: service unavailable".into()
        )));
    }

    #[test]
    fn is_transient_classifies_4xx_as_non_retryable() {
        assert!(!is_transient(&SdkError::Rpc("HTTP 404: not found".into())));
        assert!(!is_transient(&SdkError::Rpc(
            "HTTP 400: bad request".into()
        )));
    }

    #[test]
    fn is_transient_classifies_429_as_retryable() {
        // Engine emits this exact shape via its rate limiter; 429
        // should bypass exp backoff and use the server hint (see
        // parse_retry_after_secs test) but at minimum it must
        // retry rather than surface to the caller immediately.
        assert!(is_transient(&SdkError::Rpc(
            "HTTP 429 Too Many Requests: Too Many Requests! Wait for 80s".into()
        )));
        assert!(is_transient(&SdkError::Rpc(
            "HTTP 429: rate limited".into()
        )));
    }

    #[test]
    fn parse_retry_after_secs_extracts_engine_hint() {
        assert_eq!(
            parse_retry_after_secs("HTTP 429 Too Many Requests: Too Many Requests! Wait for 80s"),
            Some(80)
        );
        assert_eq!(parse_retry_after_secs("HTTP 429: Wait for 1s"), Some(1));
        assert_eq!(
            parse_retry_after_secs("Wait for 9999s before retry"),
            Some(9999)
        );
    }

    #[test]
    fn parse_retry_after_secs_handles_missing() {
        assert_eq!(parse_retry_after_secs("HTTP 429: rate limited"), None);
        assert_eq!(parse_retry_after_secs("HTTP 502: bad gateway"), None);
        assert_eq!(parse_retry_after_secs(""), None);
        // Malformed (number missing) — returns None, never panics
        assert_eq!(parse_retry_after_secs("Wait for s"), None);
        assert_eq!(parse_retry_after_secs("Wait for abcs"), None);
    }

    // Compile-time check that the retry-after cap stays in the
    // "honest user-experience" range. If someone bumps
    // MAX_RETRY_AFTER_SECS above 60s in a refactor, this fails
    // to compile — forcing the change to surface in code review.
    const _: () = assert!(super::MAX_RETRY_AFTER_SECS <= 60);

    #[test]
    fn is_transient_classifies_chain_errors_as_non_retryable() {
        // Real JSON-RPC error envelopes shouldn't retry — the chain
        // processed the request and rejected it.
        assert!(!is_transient(&SdkError::InvalidArgument(
            "nonce too low".into()
        )));
        assert!(!is_transient(&SdkError::InvalidResponse(
            "malformed envelope".into()
        )));
    }
}
