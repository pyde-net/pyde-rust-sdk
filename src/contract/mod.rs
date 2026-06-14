//! Typed contract interaction — [`Contract`] runtime + calldata
//! codec + the `pyde_abi!` proc-macro re-export.
//!
//! Two ways to call a deployed contract:
//!
//! 1. **Dynamic** — build a [`Contract`] directly from a parsed
//!    [`crate::types::ContractAbi`] and pass [`codec::Value`]
//!    arguments. Best for tools, indexers, explorers that handle
//!    arbitrary contracts.
//! 2. **Typed** — invoke `pyde_abi!(MyContract, "path/to/abi.json")`
//!    at the top of your file. The macro generates a strongly-typed
//!    `MyContract<P>` struct with methods per ABI function. Best
//!    for dapps targeting a specific contract.
//!
//! ## Dynamic example
//!
//! ```ignore
//! use std::sync::Arc;
//! use pyde_rust_sdk::{abi::extract_abi, contract::{Contract, Value}, Provider};
//!
//! # async fn run(provider: Arc<dyn Provider>, wasm: Vec<u8>, address: pyde_rust_sdk::Address) -> pyde_rust_sdk::Result<()> {
//! let abi = extract_abi(&wasm)?;
//! let contract = Contract::new(address, abi, provider);
//! let balance = contract
//!     .call("get_balance", vec![Value::Address(address)])
//!     .await?;
//! # Ok(()) }
//! ```

pub mod codec;
pub mod runtime;

pub use codec::{
    decode_return, decode_value, encode_calldata, encode_value, Value, MAX_DECODE_ELEMENTS,
};
pub use runtime::{event_signature_topic, Contract, DecodedEvent};

/// Re-export of the `pyde_abi!` proc-macro (defined in the
/// sibling `pyde-sdk-macros` crate).
pub use pyde_sdk_macros::pyde_abi;
