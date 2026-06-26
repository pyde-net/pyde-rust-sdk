#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]
//! 2-of-3 treasury multisig spend.
//!
//! Walks the full bundle-construction flow: generate three FALCON
//! signers, have two of them sign the canonical message, assemble
//! the bundle, build an envelope-style `MultisigTx`, and verify the
//! produced wire bytes round-trip. Submission to a live devnet is
//! commented out — the chain's treasury account is initialised at
//! genesis with a specific signer set, so an arbitrary 3-key set
//! generated here wouldn't verify. The example focuses on the
//! construction primitive a wallet would use against a real
//! treasury, where the signers and indices come from the on-chain
//! `MultisigState`.

use pyde_rust_sdk::multisig::{sign_action, MultisigTxPayload, SigBundle};
use pyde_rust_sdk::signer::LocalSigner;
use pyde_rust_sdk::tx::{decode, encode};
use pyde_rust_sdk::types::{Address, TxType};
use pyde_rust_sdk::{Signer, TxBuilder};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Setup ─────────────────────────────────────────────────────────
    // In production these come from the chain — `MultisigState::signers`
    // is the authoritative list; the local signer must hold a secret
    // key matching one of those pubkeys at a known index.
    let signers = [
        LocalSigner::random()?,
        LocalSigner::random()?,
        LocalSigner::random()?,
    ];
    println!("== Treasury signer set (3 FALCON-512 keys) ==");
    for (i, s) in signers.iter().enumerate() {
        println!("  signer[{i}]: {}", s.address());
    }

    // The on-chain treasury nonce — bumped by every successful
    // multisig-driven tx. Wallets read this via
    // `provider.get_account(&treasury_address())`.
    let treasury_nonce: u64 = 0;

    // Spend target + amount.
    let target =
        Address::from_hex("0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899")?;
    let amount: u128 = 1_500_000_000; // 1.5 PYDE

    // ── Sign ──────────────────────────────────────────────────────────
    // Each authorised signer signs the same canonical message
    // independently. With threshold=2, only two of the three need
    // to participate — signers[0] and signers[2] here.
    let payload_bytes = MultisigTxPayload::canonical_bytes(target, amount)?;
    let mut bundle: SigBundle = Vec::new();
    for idx in [0u32, 2] {
        let entry = sign_action(
            &signers[idx as usize],
            idx,
            TxType::MultisigTx,
            treasury_nonce,
            &payload_bytes,
        )
        .await?;
        println!(
            "  bundle entry {idx}: {} sig bytes",
            entry.signature.as_bytes().len()
        );
        bundle.push(entry);
    }
    println!("\n== Bundle: {}-of-3 ==", bundle.len());

    // ── Build envelope-style tx ──────────────────────────────────────
    // tx.from = ZERO (the bundle authorises, not a tx-level signer)
    // tx.to   = ZERO (envelope shape — the handler reads target from
    //                 the borsh-encoded payload, not tx.to)
    // tx.signature is left empty — multisig auth lives in the payload.
    let tx = TxBuilder::new()
        .from(Address::ZERO)
        .chain_id(31337)
        .nonce(0)
        .gas_limit(0)
        .multisig_treasury_spend(target, amount, bundle.clone())?
        .build()?;

    assert_eq!(tx.tx_type, TxType::MultisigTx);
    assert_eq!(tx.to, Address::ZERO);
    assert!(
        tx.signature.as_bytes().is_empty(),
        "envelope tx is unsigned"
    );
    println!(
        "\n== Built MultisigTx ==\n  data: {} bytes (borsh MultisigTxPayload)",
        tx.data.len()
    );

    // ── Wire round-trip ──────────────────────────────────────────────
    // Prove the borsh encoding is reversible — same bytes the
    // engine sees when it re-decodes the payload during verification.
    let wire = encode(&tx)?;
    let decoded = decode(&wire)?;
    assert_eq!(decoded, tx);

    let decoded_payload: MultisigTxPayload = borsh::from_slice(&decoded.data)?;
    assert_eq!(decoded_payload.target, target);
    assert_eq!(decoded_payload.amount, amount);
    assert_eq!(decoded_payload.bundle, bundle);

    println!("  wire bytes: {} total", wire.len());
    println!("  ✓ borsh round-trip preserves target + amount + bundle");

    // ── Submission (commented — needs the live chain's treasury set) ──
    //
    // let url = std::env::var("PYDE_RPC_URL")
    //     .unwrap_or_else(|_| "http://127.0.0.1:9933".to_string());
    // let transport = HttpTransport::new(&url)?;
    // let provider = Arc::new(RootProvider::new(transport));
    // let pending = provider.send_transaction(&tx).await?;
    // let receipt = pending.wait_for_receipt().await?;
    // println!("treasury spend committed in wave {}", receipt.wave_id_u64());

    println!("\n✔ Multisig treasury spend constructed end-to-end");
    Ok(())
}
