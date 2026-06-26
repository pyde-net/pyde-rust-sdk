//! Load a deployed contract by name and exercise it via the dynamic
//! `Contract` runtime — no compile-time ABI.
//!
//! Env vars:
//! - `PYDE_RPC_URL` — JSON-RPC endpoint (defaults to
//!   `http://127.0.0.1:9933`; `otigen devnet` picks a random port,
//!   so this almost always needs to be set).
//! - `PYDE_CONTRACT_NAME` — registered contract name (e.g. `"counter"`).
//! - `PYDE_FUNCTION` — view function to call (must take no args).
//!
//! Run with:
//!
//! ```sh
//! PYDE_CONTRACT_NAME=counter PYDE_FUNCTION=get_count cargo run --example contract_dynamic
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;

use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::Provider;

#[path = "shared/common.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let contract_name = std::env::var("PYDE_CONTRACT_NAME").unwrap_or_else(|_| "counter".into());
    let function = std::env::var("PYDE_FUNCTION").unwrap_or_else(|_| "get_count".into());

    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    println!("connected: {rpc_url}");

    let contract = Contract::load(&contract_name, provider).await?;
    println!("loaded {contract_name} at {}", contract.address());
    println!(
        "  v{} — {} functions",
        contract.abi().version,
        contract.abi().functions.len()
    );
    for f in &contract.abi().functions {
        let kind = if f.attrs.is_view() { "view" } else { "send" };
        println!("    {kind} {}({} params)", f.name, f.params.len());
    }

    let result: Option<Value> = contract.call(&function, Vec::new()).await?;
    println!("{function}() => {result:?}");

    Ok(())
}
