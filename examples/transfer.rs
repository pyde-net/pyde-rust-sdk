//! Sign and submit a PYDE transfer via the HTTP provider.
//!
//! Environment variables:
//! - `PYDE_RPC_URL` — JSON-RPC endpoint (defaults to
//!   `http://127.0.0.1:9933`; `otigen devnet` picks a random port,
//!   so this almost always needs to be set).
//! - `PYDE_SENDER_SEED` — 32-byte hex seed for the sender wallet.
//!   When unset, the example reproduces `devnet-0`, which `otigen
//!   devnet` pre-funds by default.
//! - `PYDE_RECIPIENT` — recipient address (defaults to all-zeros
//!   for a self-burn).
//!
//! Run with:
//!
//! ```sh
//! cargo run --example transfer
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use pyde_rust_sdk::constants::GAS_TRANSFER;
use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::util::{format_quanta, parse_quanta};
use pyde_rust_sdk::{Address, PendingTx, Provider, Signer, TxBuilder};

#[path = "shared/common.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider = Arc::new(RootProvider::new(transport));
    println!("connected: {rpc_url}");

    let wallet = common::seeded_wallet()?;
    println!("sender: {}", wallet.address());

    // Recipient defaults to zero — replace via PYDE_RECIPIENT.
    let recipient = match std::env::var("PYDE_RECIPIENT") {
        Ok(s) => Address::from_hex(&s)?,
        Err(_) => Address::ZERO,
    };
    let amount = parse_quanta("1.0").map_err(|e| anyhow::anyhow!(e))?;
    println!("recipient: {recipient}");
    println!("amount: {} PYDE ({} quanta)", format_quanta(amount), amount);

    let chain_id = provider.chain_id().await?;
    let nonce = provider.get_nonce(&wallet.address()).await?;
    println!("chain_id: {chain_id}, nonce: {nonce}");

    let mut tx = TxBuilder::new()
        .gas_limit(GAS_TRANSFER)
        .from(wallet.address())
        .chain_id(chain_id)
        .nonce(nonce)
        .transfer(recipient, amount)
        .build()?;
    wallet.sign_tx(&mut tx).await?;
    println!("signed tx: {} bytes", tx.signature.as_bytes().len());

    let hash = provider.send_raw_transaction(&tx).await?;
    println!("submitted: 0x{}", hex::encode(hash.as_bytes()));

    let dyn_provider: Arc<dyn Provider> = provider.clone();
    let pending = PendingTx::new(hash, dyn_provider).with_timeout(Duration::from_secs(30));
    let receipt = pending.wait_for_receipt().await?;
    println!(
        "committed in wave {} (tx_index {}): {:?}",
        receipt.wave_id_u64(),
        receipt.tx_index_u32(),
        receipt.status
    );

    Ok(())
}
