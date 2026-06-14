//! Open a WebSocket connection and subscribe to event logs. Prints
//! each incoming event as it arrives until the user hits Ctrl-C or
//! the connection drops.
//!
//! Env vars:
//! - `PYDE_WS_URL` — defaults to `ws://127.0.0.1:9933/ws`.
//! - `PYDE_CONTRACT_ADDRESS` — optional address to scope the
//!   subscription to a single contract.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example subscribe_logs
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use pyde_rust_sdk::types::LogFilter;
use pyde_rust_sdk::{Address, WsProvider};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("PYDE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9933/ws".to_string());

    let provider = WsProvider::connect_ws(&url).await?;
    println!("connected: {url}");

    let mut filter = LogFilter::default();
    if let Ok(addr_hex) = std::env::var("PYDE_CONTRACT_ADDRESS") {
        let addr = Address::from_hex(&addr_hex)?;
        filter.contracts.push(addr.to_hex());
        println!("scoped to contract: {addr}");
    } else {
        println!("subscribing to all contracts");
    }

    let mut sub = provider.subscribe_logs(filter).await?;
    println!("subscription id: {}", sub.id());
    println!("listening — Ctrl-C to exit");

    while let Some(result) = sub.recv().await {
        match result {
            Ok(event) => {
                println!(
                    "[wave {}, tx {}, ev {}] {} topics={} data={} bytes",
                    event.wave_id_u64(),
                    event.tx_index_u32(),
                    event.event_index_u32(),
                    event.contract_addr,
                    event.topics.len(),
                    event.data_bytes().len(),
                );
            }
            Err(e) => {
                eprintln!("decode error: {e}");
            }
        }
    }
    println!("subscription stream ended");
    Ok(())
}
