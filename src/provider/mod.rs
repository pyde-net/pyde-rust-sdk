//! JSON-RPC client surface — [`Provider`] trait, [`RootProvider`],
//! HTTP transport, and the [`PendingTx`] handle.
//!
//! Quick start:
//!
//! ```ignore
//! use std::sync::Arc;
//! use pyde_rust_sdk::provider::{HttpTransport, Provider, RootProvider};
//! use pyde_rust_sdk::types::Address;
//!
//! # async fn run() -> Result<(), pyde_rust_sdk::SdkError> {
//! let transport = HttpTransport::new("http://127.0.0.1:9933")?;
//! let provider = Arc::new(RootProvider::new(transport));
//!
//! let chain_id = provider.chain_id().await?;
//! let balance = provider
//!     .get_balance(&Address::from_hex("0xaaaa…")?)
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! Subscriptions live in [`crate::ws`]; both transports share the
//! same [`Provider`] trait.

pub mod json_rpc;
pub mod pending;
pub mod private;
pub mod provider_trait;
pub mod transport;

pub use json_rpc::{JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
pub use pending::{PendingTx, DEFAULT_POLL_INTERVAL, DEFAULT_TIMEOUT};
pub use private::PrivateSendHandle;
pub use provider_trait::{Provider, RootProvider};
pub use transport::{HttpTransport, RetryConfig, Transport};

/// Convenience alias for the HTTP-backed provider stack.
pub type HttpProvider = RootProvider<HttpTransport>;
