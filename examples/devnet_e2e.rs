//! Live end-to-end smoke test against an `otigen devnet`.
//!
//! Run the devnet first. The example reads `PYDE_RPC_URL` (and
//! falls back to `http://127.0.0.1:9933` with a warning, since
//! `otigen devnet` picks a random port by default):
//!
//! ```sh
//! otigen devnet --prefund-count 5
//! ```
//!
//! Then in another terminal, point the example at the RPC URL the
//! devnet advertised:
//!
//! ```sh
//! PYDE_RPC_URL=http://127.0.0.1:<port> cargo run --example devnet_e2e
//! ```
//!
//! Exercises every SDK layer end-to-end against a live node:
//!   1. `chain_id` + `wave_id` sanity
//!   2. Reproduce a devnet pre-funded wallet from its canonical seed
//!   3. Verify the balance the genesis pre-fund advertises
//!   4. Build + sign + submit a transfer
//!   5. Wait for the receipt
//!   6. Re-check balances on both sides
//!   7. Run a no-arg view call against `pyde_call` with a synthetic
//!      payload (should fail cleanly — proves the error envelope
//!      round-trips)

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::constants::{GAS_DEPLOY, GAS_TRANSFER};
use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::util::{format_quanta, parse_quanta};
use pyde_rust_sdk::{abi, Address, PendingTx, Provider, Signer, TxBuilder, Wallet};

#[path = "shared/common.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider = Arc::new(RootProvider::new(transport));
    let dyn_provider: Arc<dyn Provider> = provider.clone();
    println!("== connected: {rpc_url}\n");

    // ── Chain info ─────────────────────────────────────────────
    let chain_id = provider.chain_id().await?;
    let wave_id = provider.wave_id().await?;
    println!("chain_id = {chain_id} (0x{chain_id:x})");
    println!("wave_id  = {wave_id}");
    anyhow::ensure!(chain_id == 31337, "expected devnet chain_id 31337");

    // ── Reproduce devnet-0 + devnet-1 ─────────────────────────
    let seed_0 = common::devnet_secret(0);
    let seed_1 = common::devnet_secret(1);
    let sender = Wallet::from_seed(&seed_0)?;
    let recipient_existing = Wallet::from_seed(&seed_1)?;
    println!("\n== devnet-0 reproduced ==");
    println!("address: {}", sender.address());
    println!("pubkey:  0x{}…", &sender.pubkey().to_hex()[2..18]);

    // Verify pre-funded balance.
    let bal_sender = provider.get_balance(&sender.address()).await?;
    let bal_recipient = provider.get_balance(&recipient_existing.address()).await?;
    let bal_sender_pyde = format_quanta(bal_sender);
    let bal_recipient_pyde = format_quanta(bal_recipient);
    println!("balance: {bal_sender_pyde} PYDE  ({bal_sender} quanta)");
    // Devnet ships a 10-PYDE pre-fund per account by default
    // (10,000,000,000 quanta with PYDE_DECIMALS = 9). Use >= so
    // re-runs do not trip the check once a prior tx has reduced
    // the balance.
    anyhow::ensure!(
        bal_sender >= 1_000_000_000,
        "sender balance below 1 PYDE floor; got {bal_sender_pyde}"
    );
    println!("devnet-1 balance: {bal_recipient_pyde} PYDE");

    // ── Account record decode ─────────────────────────────────
    let account = provider.get_account(&sender.address()).await?;
    println!(
        "\n== account record ==\n  type: {}\n  nonce: {}\n  code_hash: {}",
        account.account_type, account.nonce, account.code_hash
    );

    // ── Transfer to a fresh random recipient ──────────────────
    let fresh_recipient = Wallet::generate()?;
    let amount = parse_quanta("1.234").map_err(|e| anyhow::anyhow!(e))?;
    let bal_before = provider.get_balance(&fresh_recipient.address()).await?;
    println!(
        "\n== transferring {} PYDE from devnet-0 → {} ==",
        format_quanta(amount),
        fresh_recipient.address()
    );

    let nonce = provider.get_nonce(&sender.address()).await?;
    let mut tx = TxBuilder::new()
        .from(sender.address())
        .chain_id(chain_id)
        .nonce(nonce)
        .transfer(fresh_recipient.address(), amount)
        .gas_limit(GAS_TRANSFER)
        .build()?;
    sender.sign_tx(&mut tx).await?;
    println!("signed tx: {} sig bytes", tx.signature.as_bytes().len());

    let hash = provider.send_raw_transaction(&tx).await?;
    println!("submitted: 0x{}", hex::encode(hash.as_bytes()));
    let pending = PendingTx::new(hash, dyn_provider.clone());

    let receipt = pending
        .with_poll_interval(Duration::from_millis(100))
        .with_timeout(Duration::from_secs(15))
        .wait_for_receipt()
        .await?;
    println!(
        "committed: wave {} / tx_index {} / status {:?} / gas {}",
        receipt.wave_id_u64(),
        receipt.tx_index_u32(),
        receipt.status,
        receipt.gas()
    );
    anyhow::ensure!(receipt.is_success(), "transfer reverted");

    let bal_recipient_after = provider.get_balance(&fresh_recipient.address()).await?;
    println!(
        "recipient balance: {} → {} ({}+{})",
        bal_before,
        bal_recipient_after,
        bal_before,
        bal_recipient_after - bal_before
    );
    anyhow::ensure!(
        bal_recipient_after == bal_before + amount,
        "recipient balance delta mismatch"
    );

    let bal_sender_after = provider.get_balance(&sender.address()).await?;
    let spent = bal_sender - bal_sender_after;
    println!(
        "sender   balance: {} → {} (spent {} quanta = {} gas + value)",
        bal_sender,
        bal_sender_after,
        spent,
        receipt.gas()
    );
    anyhow::ensure!(
        spent >= amount,
        "sender should have spent at least the transferred amount"
    );

    // ── RPC error handling round-trip ──────────────────────────
    // `pyde_call` against a non-contract address should fail
    // cleanly with `INVALID_PARAMS` — exercises the error envelope
    // decoder.
    println!("\n== error-envelope test ==");
    let req = pyde_rust_sdk::CallRequest {
        to: sender.address().to_hex(),
        data: "0x00".into(),
        from: None,
        value: None,
        gas: None,
    };
    match provider.call(&req).await {
        Ok(out) => println!("(unexpectedly ok) returned {} bytes", out.len()),
        Err(e) => println!("(expected error) {e}"),
    }

    // ── Contract deploy + call ─────────────────────────────────
    // Set PYDE_CONTRACT_WASM to a built WASM bundle (e.g. the
    // counter example: `otigen new counter-e2e --from counter &&
    // cd counter-e2e && otigen build`). Skipped silently otherwise.
    if let Ok(wasm_path) = std::env::var("PYDE_CONTRACT_WASM") {
        println!("\n== contract deploy + call ({wasm_path}) ==");
        let wasm = std::fs::read(&wasm_path)?;
        let parsed_abi = abi::extract_abi(&wasm)?;
        println!(
            "ABI: {} v{} ({} functions, {} events)",
            parsed_abi.name,
            parsed_abi.version,
            parsed_abi.functions.len(),
            parsed_abi.events.len()
        );

        // Pick a unique contract name so reruns against the same
        // devnet don't collide with a previous deploy.
        let contract_name = format!(
            "counter-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        println!("deploying as: {contract_name}");

        let deploy_nonce = provider.get_nonce(&sender.address()).await?;
        let mut deploy_tx = TxBuilder::new()
            .from(sender.address())
            .chain_id(chain_id)
            .nonce(deploy_nonce)
            .deploy(
                contract_name.clone(),
                wasm,
                pyde_rust_sdk::types::ContractType::Contract,
                Vec::new(),
            )
            .map_err(|e| anyhow::anyhow!(e))?
            .gas_limit(GAS_DEPLOY)
            .build()?;
        sender.sign_tx(&mut deploy_tx).await?;
        let deploy_hash = provider.send_raw_transaction(&deploy_tx).await?;
        println!("deploy tx: 0x{}", hex::encode(deploy_hash.as_bytes()));
        let deploy_pending = PendingTx::new(deploy_hash, dyn_provider.clone())
            .with_poll_interval(Duration::from_millis(100))
            .with_timeout(Duration::from_secs(15));
        let deploy_receipt = deploy_pending.wait_for_receipt().await?;
        anyhow::ensure!(
            deploy_receipt.is_success(),
            "deploy reverted: {:?}",
            String::from_utf8_lossy(&deploy_receipt.return_bytes())
        );
        // Address is deterministic from the name.
        let contract_addr = Address::from_contract_name(&contract_name);
        println!(
            "deployed: {} (wave {}, gas {})",
            contract_addr,
            deploy_receipt.wave_id_u64(),
            deploy_receipt.gas()
        );

        // Wrap in a Contract handle.
        let contract = Contract::new(contract_addr, parsed_abi, dyn_provider.clone());

        // View call — get the current counter value (expect 0).
        if contract.abi().function_by_name("get").is_some() {
            let result = contract.call("get", vec![]).await?;
            println!("view get() => {result:?}");
        }

        // Mutating call — increment the counter.
        if contract.abi().function_by_name("increment").is_some() {
            let pending = contract
                .send(&sender, "increment", vec![], 500_000, 0)
                .await?;
            let inc_receipt = pending
                .with_poll_interval(Duration::from_millis(100))
                .with_timeout(Duration::from_secs(15))
                .wait_for_receipt()
                .await?;
            anyhow::ensure!(inc_receipt.is_success(), "increment reverted");
            println!(
                "increment() committed in wave {} (gas {})",
                inc_receipt.wave_id_u64(),
                inc_receipt.gas()
            );
            // Decode the u64 return value.
            let return_bytes = inc_receipt.return_bytes();
            if return_bytes.len() == 8 {
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&return_bytes);
                println!("  return: {}", u64::from_le_bytes(arr));
            }
        }

        // View call again — expect counter has incremented.
        if contract.abi().function_by_name("get").is_some() {
            let result = contract.call("get", vec![]).await?;
            match result {
                Some(Value::U64(v)) => println!("view get() => {v}"),
                other => println!("view get() => {other:?}"),
            }
        }
    } else {
        println!("\n(set PYDE_CONTRACT_WASM=<path-to-counter.wasm> to exercise deploy/call)");
    }

    println!("\n✔ devnet E2E smoke test passed");
    Ok(())
}
