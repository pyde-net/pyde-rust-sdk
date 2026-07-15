//! Send a PYDE transfer *privately* via the commit-reveal flow.
//!
//! Pyde's front-running protection is commit-reveal: the transaction's
//! ordering position is fixed before its contents are visible, with no
//! decryption key anywhere. [`RootProvider::send_private`] hides the
//! two round-trips (commit → wait → reveal) behind a single call that
//! auto-reveals, so it feels like one send.
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
//! cargo run --example private_transfer
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;

use pyde_rust_sdk::constants::GAS_TRANSFER;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::util::{format_quanta, parse_quanta};
use pyde_rust_sdk::{Address, Provider, TxBuilder};

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
    println!("chain_id: {chain_id}");

    // Build the hidden inner transaction UNSIGNED. `send_private`
    // assigns its nonce, signs it, and drives the commit-reveal dance.
    // A refundable bond scaled off the transfer value is posted with
    // the commit and returned when the reveal lands.
    let inner = TxBuilder::new()
        .gas_limit(GAS_TRANSFER)
        .from(wallet.address())
        .chain_id(chain_id)
        .transfer(recipient, amount)
        .build()?;

    println!("sending privately (commit → wait → reveal)…");
    let handle = provider.send_private(&wallet, inner).await?;
    println!(
        "  commit: 0x{}",
        hex::encode(handle.commit_hash().as_bytes())
    );
    println!(
        "  reveal: 0x{}",
        hex::encode(handle.reveal_hash().as_bytes())
    );
    println!(
        "  inner:  0x{}",
        hex::encode(handle.inner_hash().as_bytes())
    );

    // The inner tx executes in the reveal wave's resolution pass, in
    // commit order. Its receipt is the real outcome.
    let receipt = handle.await_receipt().await?;
    println!(
        "committed in wave {} (tx_index {}): {:?}",
        receipt.wave_id_u64(),
        receipt.tx_index_u32(),
        receipt.status
    );

    Ok(())
}
