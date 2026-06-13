//! JSON-RPC 2.0 envelope types.
//!
//! Pyde's JSON-RPC is strict 2.0 — every request carries
//! `"jsonrpc": "2.0"`, a `method` string, an optional `params`
//! array, and an `id` (number or string). Responses pair `id` back
//! with either a `result` payload or an `error` envelope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::SdkError;

/// The canonical JSON-RPC 2.0 version string.
pub const JSONRPC_VERSION: &str = "2.0";

/// A JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest<'a> {
    /// Always `"2.0"`.
    pub jsonrpc: &'static str,
    /// Method name (e.g. `"pyde_chainId"`).
    pub method: &'a str,
    /// Positional params. The engine expects an array per method;
    /// `[]` for no-arg methods.
    pub params: Value,
    /// Request id. Echoed in the response.
    pub id: u64,
}

impl<'a> JsonRpcRequest<'a> {
    /// Build a request with a fresh id and the given method + params.
    #[must_use]
    pub fn new(id: u64, method: &'a str, params: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            method,
            params,
            id,
        }
    }
}

/// A JSON-RPC 2.0 response envelope.
///
/// Exactly one of `result` / `error` is populated on a well-formed
/// response.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    /// JSON-RPC version. Should always be `"2.0"`.
    #[serde(default)]
    pub jsonrpc: Option<String>,
    /// Echoed request id (when present).
    #[serde(default)]
    pub id: Option<Value>,
    /// Successful result payload.
    #[serde(default)]
    pub result: Option<Value>,
    /// Error envelope.
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// Take the result if present, or convert the error to
    /// [`SdkError`].
    ///
    /// A response with neither `result` nor `error` is treated as
    /// `Ok(Value::Null)` — serde's default `Option` handling
    /// collapses missing-field and explicit-`null` into `None`, so
    /// surfacing `Value::Null` is the only way to give callers the
    /// "this query has no result" signal (e.g.
    /// `pyde_resolveName` for an unregistered name returns `null`).
    ///
    /// # Errors
    /// - [`SdkError::Rpc`] when the response carries an error envelope.
    pub fn into_result(self) -> Result<Value, SdkError> {
        if let Some(err) = self.error {
            return Err(err.into_sdk_error());
        }
        Ok(self.result.unwrap_or(Value::Null))
    }
}

/// JSON-RPC error envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    /// Standard JSON-RPC error code (negative integer).
    pub code: i64,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured data attached by the server.
    #[serde(default)]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Convert to the appropriate [`SdkError`] variant.
    ///
    /// Mapping (see [JSON-RPC 2.0 spec][spec]):
    ///
    /// | Code            | Variant                       |
    /// |-----------------|-------------------------------|
    /// | `-32700`        | `Rpc` ("parse error")         |
    /// | `-32600..-32601`| `Rpc` ("invalid request")     |
    /// | `-32602`        | `InvalidArgument`             |
    /// | `-32603`        | `Rpc` ("internal error")      |
    /// | anything else   | `Rpc` (raw message)           |
    ///
    /// [spec]: https://www.jsonrpc.org/specification#error_object
    #[must_use]
    pub fn into_sdk_error(self) -> SdkError {
        match self.code {
            -32602 => SdkError::InvalidArgument(self.message),
            _ => SdkError::Rpc(format!("[{}] {}", self.code, self.message)),
        }
    }
}

/// JSON-RPC notification envelope — used by WebSocket subscriptions
/// for server-pushed events.
///
/// Notifications never carry an `id`; they're addressed by
/// `params.subscription` (the subscription id returned by
/// `pyde_subscribe`).
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    /// JSON-RPC version. Always `"2.0"`.
    pub jsonrpc: String,
    /// Notification method (e.g. `"pyde_subscription"`).
    pub method: String,
    /// Method-specific parameters.
    pub params: Value,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use serde_json::json;

    #[test]
    fn request_serialises_canonical_shape() {
        let req = JsonRpcRequest::new(7, "pyde_chainId", json!([]));
        let s = serde_json::to_string(&req).unwrap();
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["method"], "pyde_chainId");
        assert_eq!(v["id"], 7);
        assert_eq!(v["params"], json!([]));
    }

    #[test]
    fn response_into_result_unwraps_success() {
        let resp: JsonRpcResponse =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#).unwrap();
        let v = resp.into_result().unwrap();
        assert_eq!(v, json!("0x1"));
    }

    #[test]
    fn response_into_result_surfaces_error() {
        let resp: JsonRpcResponse = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad params"}}"#,
        )
        .unwrap();
        let err = resp.into_result().unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(_)));
    }

    #[test]
    fn response_into_result_null_when_no_result_or_error() {
        // No `result` field at all + no `error` → treat as null,
        // matching engine responses for "not found" queries.
        let resp: JsonRpcResponse = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1}"#).unwrap();
        let v = resp.into_result().unwrap();
        assert!(v.is_null());
    }

    #[test]
    fn response_into_result_null_for_explicit_null_result() {
        let resp: JsonRpcResponse =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":null}"#).unwrap();
        let v = resp.into_result().unwrap();
        assert!(v.is_null());
    }
}
