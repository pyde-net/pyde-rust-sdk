//! Use `pyde_abi!` to generate a typed contract wrapper at compile
//! time, then call its view function via the HTTP provider.
//!
//! Env vars:
//! - `PYDE_RPC_URL` — JSON-RPC endpoint (defaults to
//!   `http://127.0.0.1:9933`; `otigen devnet` picks a random port,
//!   so this almost always needs to be set).
//! - `PYDE_COUNTER_ADDRESS` — deployed Counter contract address
//!   (hex, with or without `0x` prefix).
//!
//! Run with:
//!
//! ```sh
//! PYDE_COUNTER_ADDRESS=0x… cargo run --example contract_typed
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;

use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::{Address, Provider};

// The ABI fixture ships in tests/fixtures/ — we reuse it here so
// the example builds out of the box. Real dapps would point this at
// their contract's exported ABI JSON.
pyde_rust_sdk::pyde_abi!(Counter, "tests/fixtures/counter_abi.json");

#[path = "shared/common.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let address = match std::env::var("PYDE_COUNTER_ADDRESS") {
        Ok(s) => Address::from_hex(&s)?,
        Err(_) => {
            println!(
                "no PYDE_COUNTER_ADDRESS set; printing typed ABI metadata only.\n\
                 Set PYDE_COUNTER_ADDRESS=<deployed-address> to issue a live call."
            );
            println!("contract: {} v{}", Counter::NAME, Counter::VERSION);
            return Ok(());
        }
    };

    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    println!("connected: {rpc_url}");

    let counter = Counter::new(address, provider);
    let count = counter.get_count().await?;
    let owner = counter.owner().await?;
    println!("Counter at {address}");
    println!("  count: {count}");
    println!("  owner: {owner}");

    Ok(())
}
