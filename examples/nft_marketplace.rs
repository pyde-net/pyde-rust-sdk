//! Multi-account, multi-contract end-to-end demo: a PTS-F token + a PTS-N NFT +
//! a custom NFT marketplace, all deployed and orchestrated through
//! the SDK.
//!
//! ## Scenario
//!
//! Three accounts:
//!
//! - **deployer** (devnet-0) — deploys all three contracts; holds
//!   the initial token supply minted by the token's constructor.
//! - **seller** (devnet-1) — receives an NFT (minted by deployer
//!   into seller's address), lists it for sale on the marketplace.
//! - **buyer** (devnet-2) — receives tokens (transferred from
//!   deployer), approves the marketplace, calls `buy` to atomically
//!   swap payment for ownership.
//!
//! ## Pipeline
//!
//! 1. Deploy the fungible token — constructor mints 1_000_000 tokens to deployer.
//! 2. Deploy the NFT with `init(name, symbol, max_supply)`.
//! 3. Deploy marketplace with `init(token_addr, nft_addr)`.
//! 4. Deployer transfers 1000 tokens to buyer.
//! 5. Deployer mints NFT into seller's address.
//! 6. Seller approves marketplace as the NFT's spender.
//! 7. Seller `list_item(token_id, price)`.
//! 8. Buyer approves marketplace to pull `price` tokens.
//! 9. Buyer `buy(listing_id)` — marketplace cross-calls into both
//!    contracts to atomically swap.
//! 10. Verify final state: NFT owned by buyer, token balances
//!     shifted, listing inactive.
//!
//! ## Prereqs
//!
//! Build the three contracts via otigen:
//!
//! ```sh
//! otigen new token-mkt    --from fungible-token
//! otigen new nft-mkt      --from nft-token
//! # marketplace lives at otigen/examples/marketplace (or build your own)
//! cd token-mkt   && otigen build
//! cd nft-mkt  && otigen build
//! cd marketplace && otigen build
//! ```
//!
//! Then point the example at the three bundle paths:
//!
//! ```sh
//! PYDE_the token_WASM=/tmp/token-mkt/artifacts/token-mkt.bundle/contract.wasm \
//! PYDE_the NFT_WASM=/tmp/nft-mkt/artifacts/nft-mkt.bundle/contract.wasm \
//! PYDE_MARKETPLACE_WASM=/tmp/nft-marketplace/artifacts/nft-marketplace.bundle/contract.wasm \
//! cargo run --example nft_marketplace
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::constants::{
    GAS_CROSS_CALL_ORCHESTRATOR, GAS_DEPLOY, GAS_NFT_CALL, GAS_TOKEN_CALL,
};
use pyde_rust_sdk::contract::{decode_value, Contract};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ContractType, ParamType, Tx};
use pyde_rust_sdk::{abi, Address, PendingTx, Provider, Signer, TxBuilder, Wallet};

#[path = "shared/common.rs"]
mod common;

fn require_env(key: &str) -> anyhow::Result<String> {
    std::env::var(key).map_err(|_| anyhow::anyhow!("missing required env: {key}"))
}

// ── Borsh-arg helpers ──────────────────────────────────────────

fn enc_address(a: &Address) -> Vec<u8> {
    a.as_bytes().to_vec()
}

fn enc_u64(n: u64) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

fn enc_u128(n: u128) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

fn enc_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(4 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    let total: usize = parts.iter().map(|p| p.len()).sum();
    let mut out = Vec::with_capacity(total);
    for p in parts {
        out.extend_from_slice(p);
    }
    out
}

// ── Helpers driving the chain ──────────────────────────────────

/// Build, sign, and submit a tx; wait for the receipt.
async fn send_tx(
    provider: &Arc<RootProvider<HttpTransport>>,
    dyn_provider: &Arc<dyn Provider>,
    wallet: &Wallet,
    chain_id: u64,
    build: impl FnOnce(TxBuilder) -> anyhow::Result<TxBuilder>,
) -> anyhow::Result<pyde_rust_sdk::Receipt> {
    let nonce = provider.get_nonce(&wallet.address()).await?;
    let mut tx: Tx = build(
        TxBuilder::new()
            .from(wallet.address())
            .chain_id(chain_id)
            .nonce(nonce),
    )?
    .build()?;
    wallet.sign_tx(&mut tx).await?;
    let hash = provider.send_raw_transaction(&tx).await?;
    let receipt = PendingTx::new(hash, dyn_provider.clone())
        .with_poll_interval(Duration::from_millis(100))
        .with_timeout(Duration::from_secs(20))
        .wait_for_receipt()
        .await?;
    anyhow::ensure!(
        receipt.is_success(),
        "tx reverted: {:?}",
        String::from_utf8_lossy(&receipt.return_bytes())
    );
    Ok(receipt)
}

/// Deploy a contract: builds DeployData with `name`, `wasm`, `init_calldata`,
/// submits the tx as `deployer`, waits for the receipt, returns the
/// deployed address.
async fn deploy_contract(
    provider: &Arc<RootProvider<HttpTransport>>,
    dyn_provider: &Arc<dyn Provider>,
    deployer: &Wallet,
    chain_id: u64,
    name: &str,
    wasm: Vec<u8>,
    init_calldata: Vec<u8>,
) -> anyhow::Result<Address> {
    println!("  deploying {name} ({} bytes)…", wasm.len());
    let receipt = send_tx(provider, dyn_provider, deployer, chain_id, |b| {
        Ok(b.deploy(name, wasm, ContractType::Contract, init_calldata)
            .map_err(|e| anyhow::anyhow!(e))?
            .gas_limit(GAS_DEPLOY))
    })
    .await?;
    let address = Address::from_contract_name(name);
    println!(
        "    → {address} (wave {}, gas {})",
        receipt.wave_id_u64(),
        receipt.gas()
    );
    Ok(address)
}

/// Helper for typed view calls. Doesn't go through `Contract::call`'s
/// dynamic Value layer — bypasses that so we can target arbitrary
/// return types.
async fn view_call<T: borsh::BorshDeserialize>(
    contract: &Contract,
    function: &str,
    calldata: Vec<u8>,
    return_type: &ParamType,
) -> anyhow::Result<T> {
    let payload = pyde_rust_sdk::types::CallPayload {
        function: function.to_string(),
        calldata,
    };
    let payload_bytes = borsh::to_vec(&payload)?;
    let req = pyde_rust_sdk::CallRequest {
        to: contract.address().to_hex(),
        data: format!("0x{}", hex::encode(&payload_bytes)),
        from: None,
        value: None,
        gas: None,
    };
    let return_bytes = contract.provider().call(&req).await?;
    let _ = decode_value(return_type, &return_bytes)?;
    Ok(borsh::from_slice::<T>(&return_bytes)?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let token_wasm_path = require_env("PYDE_the token_WASM")?;
    let nft_wasm_path = require_env("PYDE_the NFT_WASM")?;
    let marketplace_wasm_path = require_env("PYDE_MARKETPLACE_WASM")?;

    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider = Arc::new(RootProvider::new(transport));
    let dyn_provider: Arc<dyn Provider> = provider.clone();
    println!("connected: {rpc_url}\n");

    let chain_id = provider.chain_id().await?;
    anyhow::ensure!(chain_id == 31337, "expected devnet chain_id 31337");

    // ── Cast three accounts ───────────────────────────────────
    let deployer = Wallet::from_seed(&common::devnet_secret(0))?;
    let seller = Wallet::from_seed(&common::devnet_secret(1))?;
    let buyer = Wallet::from_seed(&common::devnet_secret(2))?;
    println!("cast:");
    println!("  deployer = {}", deployer.address());
    println!("  seller   = {}", seller.address());
    println!("  buyer    = {}", buyer.address());

    // ── 1. Deploy the token ──────────────────────────────────────
    println!("\n[1] deploy the token");
    let token_wasm = std::fs::read(&token_wasm_path)?;
    let token_name = format!(
        "token-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let token_addr = deploy_contract(
        &provider,
        &dyn_provider,
        &deployer,
        chain_id,
        &token_name,
        token_wasm.clone(),
        Vec::new(), // the token.init() takes no args
    )
    .await?;
    let token = Contract::new(
        token_addr,
        abi::extract_abi(&token_wasm)?,
        dyn_provider.clone(),
    );

    // ── 2. Deploy the NFT ─────────────────────────────────────
    println!("\n[2] deploy the NFT");
    let nft_wasm = std::fs::read(&nft_wasm_path)?;
    let nft_name = format!(
        "nft-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let nft_addr = deploy_contract(
        &provider,
        &dyn_provider,
        &deployer,
        chain_id,
        &nft_name,
        nft_wasm.clone(),
        // init(name: String, symbol: String) — calldata = borsh(name) || borsh(symbol)
        concat(&[&enc_string("PydeNFT"), &enc_string("PYD")]),
    )
    .await?;
    let nft = Contract::new(nft_addr, abi::extract_abi(&nft_wasm)?, dyn_provider.clone());

    // ── 3. Deploy Marketplace ────────────────────────────────
    println!("\n[3] deploy marketplace");
    let marketplace_wasm = std::fs::read(&marketplace_wasm_path)?;
    let marketplace_name = format!(
        "marketplace-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let marketplace_addr = deploy_contract(
        &provider,
        &dyn_provider,
        &deployer,
        chain_id,
        &marketplace_name,
        marketplace_wasm.clone(),
        concat(&[&enc_address(&token_addr), &enc_address(&nft_addr)]),
    )
    .await?;
    let marketplace = Contract::new(
        marketplace_addr,
        abi::extract_abi(&marketplace_wasm)?,
        dyn_provider.clone(),
    );

    // ── 4. Deployer → Buyer: 1000 the token ─────────────────────
    println!("\n[4] deployer transfers 1000 the token → buyer");
    let _r = send_tx(&provider, &dyn_provider, &deployer, chain_id, |b| {
        let calldata = concat(&[&enc_address(&buyer.address()), &enc_u128(1000)]);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "transfer".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(token.address()).data(data).gas_limit(GAS_TOKEN_CALL))
    })
    .await?;

    let buyer_balance_initial: u128 = view_call(
        &token,
        "balance_of",
        enc_address(&buyer.address()),
        &ParamType::U128,
    )
    .await?;
    println!("    buyer the token balance: {buyer_balance_initial}");
    anyhow::ensure!(buyer_balance_initial == 1000, "buyer should have 1000");

    // ── 5. Mint NFT into seller's address ───────────────────
    println!("\n[5] deployer mints NFT → seller");
    let mint_receipt = send_tx(&provider, &dyn_provider, &deployer, chain_id, |b| {
        let calldata = concat(&[&enc_address(&seller.address()), &enc_string("ipfs://demo")]);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "mint".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(nft.address()).data(data).gas_limit(GAS_NFT_CALL))
    })
    .await?;
    // mint() returns the new token_id as a u64.
    let token_id = {
        let mut arr = [0u8; 8];
        let bytes = mint_receipt.return_bytes();
        anyhow::ensure!(bytes.len() == 8, "expected 8 bytes for u64 token_id");
        arr.copy_from_slice(&bytes);
        u64::from_le_bytes(arr)
    };
    println!("    minted token_id = {token_id}");

    let owner: Address =
        view_call(&nft, "owner_of", enc_u64(token_id), &ParamType::Address).await?;
    anyhow::ensure!(owner == seller.address(), "seller should own the NFT");

    // ── 6. Seller approves marketplace for the NFT ──────────
    println!("\n[6] seller approves marketplace for NFT");
    let _r = send_tx(&provider, &dyn_provider, &seller, chain_id, |b| {
        let calldata = concat(&[&enc_address(&marketplace_addr), &enc_u64(token_id)]);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "approve".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(nft.address()).data(data).gas_limit(GAS_TOKEN_CALL))
    })
    .await?;

    // ── 7. Seller lists the NFT ─────────────────────────────
    let price: u128 = 250;
    println!("\n[7] seller lists NFT for {price} the token");
    let list_receipt = send_tx(&provider, &dyn_provider, &seller, chain_id, |b| {
        let calldata = concat(&[&enc_u64(token_id), &enc_u128(price)]);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "list_item".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(marketplace.address())
            .data(data)
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    let listing_id = {
        let mut arr = [0u8; 8];
        let bytes = list_receipt.return_bytes();
        anyhow::ensure!(bytes.len() == 8, "expected 8 bytes for u64 listing_id");
        arr.copy_from_slice(&bytes);
        u64::from_le_bytes(arr)
    };
    println!("    listing_id = {listing_id}");

    // ── 8. Buyer approves marketplace for the token ─────────────
    println!("\n[8] buyer approves marketplace for {price} the token");
    let _r = send_tx(&provider, &dyn_provider, &buyer, chain_id, |b| {
        let calldata = concat(&[&enc_address(&marketplace_addr), &enc_u128(price)]);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "approve".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(token.address()).data(data).gas_limit(GAS_TOKEN_CALL))
    })
    .await?;

    // ── 9. Buyer calls buy ──────────────────────────────────
    println!("\n[9] buyer calls marketplace.buy({listing_id})");
    let buy_receipt = send_tx(&provider, &dyn_provider, &buyer, chain_id, |b| {
        let calldata = enc_u64(listing_id);
        let payload = pyde_rust_sdk::types::CallPayload {
            function: "buy".to_string(),
            calldata,
        };
        let data = borsh::to_vec(&payload)?;
        Ok(b.to(marketplace.address())
            .data(data)
            .gas_limit(GAS_CROSS_CALL_ORCHESTRATOR))
    })
    .await?;
    println!(
        "    buy committed in wave {} (gas {})",
        buy_receipt.wave_id_u64(),
        buy_receipt.gas()
    );

    // ── 10. Verify final state ──────────────────────────────
    println!("\n[10] final state");

    let new_owner: Address =
        view_call(&nft, "owner_of", enc_u64(token_id), &ParamType::Address).await?;
    println!("    NFT owner: {new_owner}");
    anyhow::ensure!(new_owner == buyer.address(), "buyer should own the NFT");

    let buyer_balance_after: u128 = view_call(
        &token,
        "balance_of",
        enc_address(&buyer.address()),
        &ParamType::U128,
    )
    .await?;
    let seller_balance: u128 = view_call(
        &token,
        "balance_of",
        enc_address(&seller.address()),
        &ParamType::U128,
    )
    .await?;
    println!("    buyer the token: {buyer_balance_initial} → {buyer_balance_after}");
    println!("    seller the token: 0 → {seller_balance}");
    anyhow::ensure!(
        buyer_balance_after == buyer_balance_initial - price,
        "buyer balance delta mismatch"
    );
    anyhow::ensure!(seller_balance == price, "seller balance delta mismatch");

    let active: bool = view_call(
        &marketplace,
        "is_active",
        enc_u64(listing_id),
        &ParamType::Bool,
    )
    .await?;
    println!("    listing active: {active}");
    anyhow::ensure!(!active, "listing should be inactive");

    println!("\n✔ NFT marketplace E2E passed");
    Ok(())
}
