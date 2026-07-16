//! Live demonstration of every halt mechanism Pyde supports +
//! the SDK's structured error decoding.
//!
//! Deploys a Go-authored `access-guard` contract and walks
//! through each failure mode end-to-end:
//!
//! 1. **Authorization revert** — non-admin caller hits a
//!    `guardAdmin()` check inside the contract; revert payload
//!    is `"unauthorized: caller is not admin"`. SDK decodes via
//!    `SdkError::revert_reason()`.
//! 2. **Plain UTF-8 revert** — entry that always calls
//!    `pyde::revert("custom message …")`. SDK shows the message.
//! 3. **Named-token revert** — entry reverts with a payload
//!    containing `"ERR_FORBIDDEN"`. SDK's
//!    [`SdkError::error_code`] returns `Some(ErrorCode::Forbidden)`.
//! 4. **Integer-code revert** — entry reverts with `"-5"`
//!    embedded. SDK's integer-parse fallback also returns
//!    `Some(ErrorCode::Forbidden)`.
//! 5. **WASM trap** — entry hits an out-of-bounds slice access;
//!    the engine traps the instance and reports failure with no
//!    revert payload. SDK reports gas burned but no reason.
//!
//! ## Prereqs
//!
//! Build the access-guard contract via otigen:
//!
//! ```sh
//! otigen init --lang go access-guard
//! # replace main.go + otigen.toml with the SDK example fixture
//! # (see examples/contracts/access-guard/ once landed upstream)
//! cd access-guard && otigen build
//! ```
//!
//! Then point this example at the bundle:
//!
//! ```sh
//! PYDE_ACCESS_GUARD_WASM=/tmp/access-guard/artifacts/access-guard.bundle/contract.wasm \
//! cargo run --example halt_methods
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::constants::{GAS_DEPLOY, GAS_TOKEN_CALL};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ContractType, ErrorCode, RevertCategory, Tx};
use pyde_rust_sdk::{
    Address, CallPayload, CallRequest, PendingTx, Provider, Receipt, ReceiptStatus, SdkError,
    Signer, TxBuilder, Wallet,
};

#[path = "shared/common.rs"]
mod common;

// ── Helpers ─────────────────────────────────────────────────────

async fn send_tx(
    provider: &Arc<RootProvider<HttpTransport>>,
    dyn_provider: &Arc<dyn Provider>,
    wallet: &Wallet,
    chain_id: u64,
    build: impl FnOnce(TxBuilder) -> anyhow::Result<TxBuilder>,
) -> anyhow::Result<Receipt> {
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
        .with_timeout(Duration::from_secs(15))
        .wait_for_receipt()
        .await?;
    Ok(receipt)
}

fn call_payload_no_args(function: &str) -> Vec<u8> {
    let payload = CallPayload {
        function: function.to_string(),
        calldata: Vec::new(),
    };
    borsh::to_vec(&payload).expect("borsh CallPayload")
}

fn call_payload_u64_arg(function: &str, arg: u64) -> Vec<u8> {
    let payload = CallPayload {
        function: function.to_string(),
        calldata: arg.to_le_bytes().to_vec(),
    };
    borsh::to_vec(&payload).expect("borsh CallPayload")
}

/// Centralised diagnostic printer — pulls out the reason, the
/// structured `ErrorCode` (if any), and the gas cost in one place.
fn explain(label: &str, receipt: &Receipt) {
    println!("\n── {label} ──");
    let return_data = receipt.return_bytes();
    let status_str = match receipt.status {
        ReceiptStatus::Success => "Success",
        ReceiptStatus::Reverted => "Reverted",
        ReceiptStatus::OutOfGas => "OutOfGas",
    };
    println!("  status:        {status_str}");
    println!(
        "  gas charged:   {} (wave {} / tx_index {})",
        receipt.gas(),
        receipt.wave_id_u64(),
        receipt.tx_index_u32()
    );
    println!("  return bytes:  {} byte(s)", return_data.len());

    if matches!(receipt.status, ReceiptStatus::Success) {
        return;
    }

    // Build the SdkError shape the dapp would see, then ask the SDK
    // to decode it.
    let err = SdkError::Reverted {
        gas_used: receipt.gas(),
        data: return_data.clone(),
        reason: receipt.revert_reason.clone(),
    };
    let category_label = match err.revert_category() {
        Some(RevertCategory::EngineValidation) => "engine validation",
        Some(RevertCategory::Contract) => "contract revert",
        Some(RevertCategory::Vm) => "VM trap",
        Some(RevertCategory::Other(s)) => {
            println!("  category:      <unknown: {s}>");
            return;
        }
        None => "<no structured category>",
    };
    println!("  category:      {category_label}");
    match err.revert_reason() {
        Some(reason) => println!("  reason:        {reason:?}"),
        None => println!("  reason:        <no decodable payload — likely WASM trap>"),
    }
    match err.error_code() {
        Some(code) => println!("  error_code:    {code} ({})", code.name()),
        None => println!("  error_code:    none"),
    }
    println!("  SdkError.code: {}", err.code());
    println!("  → user-facing: {}", precise_user_message(&err));
}

/// Demonstrates one match on `RevertCategory`, with string heuristics
/// only as last-resort UX polish for contract-side reverts where the
/// engine doesn't supply a typed error code.
fn precise_user_message(err: &SdkError) -> String {
    if err.is_engine_validation_revert() {
        if let SdkError::Reverted {
            reason: Some(r), ..
        } = err
        {
            return format!("⚙ Rejected before execution: {}", r.message);
        }
    }
    if err.is_vm_trap() {
        return "❌ Contract trapped (panic / out-of-bounds).".into();
    }
    if let Some(code) = err.error_code() {
        return match code {
            ErrorCode::Forbidden => "❌ This action isn't permitted.".into(),
            ErrorCode::InsufficientBalance => "❌ Not enough balance.".into(),
            ErrorCode::ReentrancyBlocked => "❌ Re-entrant call detected.".into(),
            ErrorCode::ValueTransferNotPayable => {
                "❌ Target function doesn't accept value transfers.".into()
            }
            ErrorCode::SignatureInvalid => "❌ Bad signature.".into(),
            ErrorCode::OutOfGas => "❌ Ran out of gas.".into(),
            other => format!("❌ Chain returned {other}"),
        };
    }
    if err.is_contract_revert() {
        if let Some(reason) = err.revert_reason() {
            if reason.starts_with("unauthorized") {
                return "🔒 You're not authorised to do that.".into();
            }
            return format!("❌ Reverted: {reason}");
        }
    }
    "❌ Reverted (no decodable payload)".into()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = common::rpc_url();
    let wasm_path = std::env::var("PYDE_ACCESS_GUARD_WASM").map_err(|_| {
        anyhow::anyhow!(
            "missing PYDE_ACCESS_GUARD_WASM — build the contract first: \
             `cd /tmp/access-guard && otigen build`"
        )
    })?;
    let transport = HttpTransport::new(rpc_url.clone())?;
    let provider = Arc::new(RootProvider::new(transport));
    let dyn_provider: Arc<dyn Provider> = provider.clone();
    println!("connected: {rpc_url}");

    let chain_id = provider.chain_id().await?;
    let deployer = Wallet::from_seed(&common::devnet_secret(0))?;
    let stranger = Wallet::from_seed(&common::devnet_secret(1))?;
    println!("deployer (admin) = {}", deployer.address());
    println!("stranger         = {}", stranger.address());

    // ── Deploy ─────────────────────────────────────────────────
    let wasm = std::fs::read(&wasm_path)?;
    let name = format!(
        "access-guard-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    println!("\n[deploy] {name} ({} bytes)", wasm.len());
    let receipt = send_tx(&provider, &dyn_provider, &deployer, chain_id, |b| {
        Ok(
            b.deploy(name.clone(), wasm, ContractType::Contract, Vec::new())
                .map_err(|e| anyhow::anyhow!(e))?
                .gas_limit(GAS_DEPLOY),
        )
    })
    .await?;
    anyhow::ensure!(receipt.is_success(), "deploy reverted");
    let contract = Address::from_contract_name(&name);
    println!("  deployed at {contract}");

    // ── 1. Verify the admin was stored correctly ──────────────
    println!("\n[verify admin] view get_admin()");
    let req = CallRequest {
        to: contract.to_hex(),
        data: format!("0x{}", hex::encode(call_payload_no_args("get_admin"))),
        from: None,
        value: None,
        gas: None,
    };
    let bytes = provider.call(&req).await?;
    let mut arr = [0u8; 32];
    anyhow::ensure!(
        bytes.len() == 32,
        "expected 32-byte address; got {}",
        bytes.len()
    );
    arr.copy_from_slice(&bytes);
    let stored_admin = Address::new(arr);
    println!("  stored admin: {stored_admin}");
    anyhow::ensure!(stored_admin == deployer.address(), "admin mismatch");

    // ── 2. Happy path — deployer calls admin_bump(5) ─────────
    println!("\n[happy] deployer.admin_bump(5)");
    let receipt = send_tx(&provider, &dyn_provider, &deployer, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_u64_arg("admin_bump", 5))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("after admin_bump(5)", &receipt);
    anyhow::ensure!(receipt.is_success(), "admin call should succeed");

    // ── 3. Halt — non-admin calls admin_bump → unauthorized ──
    println!("\n[halt 1/5] stranger.admin_bump(99) — expect unauthorized revert");
    let receipt = send_tx(&provider, &dyn_provider, &stranger, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_u64_arg("admin_bump", 99))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("authorization revert", &receipt);
    anyhow::ensure!(!receipt.is_success(), "stranger call should revert");

    // ── 4. Halt — explicit plain-message revert ──────────────
    println!("\n[halt 2/5] stranger.cause_revert_with_message — expect plain UTF-8 revert");
    let receipt = send_tx(&provider, &dyn_provider, &stranger, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_no_args("cause_revert_with_message"))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("plain-message revert", &receipt);

    // ── 5. Halt — revert payload contains ERR_FORBIDDEN ──────
    println!("\n[halt 3/5] stranger.cause_revert_with_err_forbidden — expect named-token revert");
    let receipt = send_tx(&provider, &dyn_provider, &stranger, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_no_args("cause_revert_with_err_forbidden"))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("named-token revert (ERR_FORBIDDEN)", &receipt);
    assert!(
        receipt.is_contract_revert() || receipt.revert_reason.is_none(),
        "expected contract category or no structured reason"
    );

    // ── 6. Halt — revert payload contains "-5" ───────────────
    println!("\n[halt 4/5] stranger.cause_revert_with_negative_code — expect integer-code revert");
    let receipt = send_tx(&provider, &dyn_provider, &stranger, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_no_args("cause_revert_with_negative_code"))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("integer-code revert (-5)", &receipt);
    assert!(
        receipt.is_contract_revert() || receipt.revert_reason.is_none(),
        "expected contract category or no structured reason"
    );

    // ── 7. Halt — WASM trap (no revert payload) ──────────────
    println!("\n[halt 5/5] stranger.cause_panic — expect WASM trap, no payload");
    let receipt = send_tx(&provider, &dyn_provider, &stranger, chain_id, |b| {
        Ok(b.to(contract)
            .data(call_payload_no_args("cause_panic"))
            .gas_limit(GAS_TOKEN_CALL))
    })
    .await?;
    explain("WASM trap", &receipt);
    assert!(
        receipt.is_vm_trap() || receipt.revert_reason.is_none(),
        "expected vm trap category or no structured reason"
    );

    println!("\n✔ halt-method demonstration complete");
    Ok(())
}
