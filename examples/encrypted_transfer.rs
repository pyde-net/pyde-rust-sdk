//! Round-trip an encrypted transfer through Pyde's MEV-protected
//! mempool path.
//!
//! The flow:
//! 1. Fetch the current epoch's threshold pubkey via
//!    [`Provider::get_threshold_public_key`] and verify
//!    `scheme == "kyber-768-goldilocks"` (the real-crypto path).
//! 2. Build + sign a plaintext `Tx` as usual.
//! 3. Borsh-encode the `Tx`, threshold-encrypt under the epoch
//!    pubkey via `pyde_crypto::threshold::threshold_encrypt`, then
//!    call `.to_wire_bytes()` (NOT `.to_bytes()` — the engine's
//!    admit-side borsh decoder expects the wire form).
//! 4. Wrap in [`EncryptedTxEnvelope`], borsh-serialize, hex-encode,
//!    submit via [`Provider::send_raw_encrypted_transaction`].
//! 5. Poll [`Provider::get_transaction_receipt`] for the *plaintext*
//!    hash — that's the receipt the wave-commit decryption ceremony
//!    publishes when the inner Tx executes.
//!
//! On a single-validator devnet the ceremony runs with `committee=1,
//! threshold=1`; the lone validator self-inserts its local share
//! (engine #334) and the plaintext receipt lands within a few waves.
//!
//! Requires a Pyde devnet at `PYDE_RPC_URL` (default
//! `http://127.0.0.1:9933`).
//!
//! Run:
//!
//! ```sh
//! cargo run --example encrypted_transfer
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::sync::Arc;
use std::time::Duration;

use pyde_crypto::threshold::{threshold_encrypt, ThresholdPublicKey};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::EncryptedTxEnvelope;
use pyde_rust_sdk::{Address, Provider, Signer, TxBuilder, Wallet};

/// Devnet's canonical pre-funded seed: `blake3("pyde-devnet-v1/" || u64_le(i))`.
fn devnet_seed(i: u64) -> [u8; 32] {
    let mut input = Vec::with_capacity(b"pyde-devnet-v1/".len() + 8);
    input.extend_from_slice(b"pyde-devnet-v1/");
    input.extend_from_slice(&i.to_le_bytes());
    *blake3::hash(&input).as_bytes()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("PYDE_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:9933".to_string());
    let provider = Arc::new(RootProvider::new(HttpTransport::new(&url)?));
    println!("connected: {url}");

    // 1. Fetch the threshold pubkey for the current epoch.
    let tpk_record = provider
        .get_threshold_public_key()
        .await?
        .ok_or_else(|| anyhow::anyhow!("no threshold pubkey published yet"))?;
    println!(
        "threshold pubkey: epoch={} scheme={}",
        tpk_record.epoch, tpk_record.scheme
    );
    if tpk_record.scheme != "kyber-768-goldilocks" {
        eprintln!(
            "warning: scheme is {:?}; encrypted submits expect 'kyber-768-goldilocks'. \
             The chain may be running on v1 mock-DKG; plaintext receipt \
             will not land until real-crypto ships.",
            tpk_record.scheme
        );
    }
    let pk_bytes = hex::decode(tpk_record.public_key.trim_start_matches("0x"))?;
    let tpk = ThresholdPublicKey::from_bytes(&pk_bytes)
        .ok_or_else(|| anyhow::anyhow!("ThresholdPublicKey::from_bytes failed"))?;

    // 2. Build + sign a plaintext transfer from devnet-0 to a sink.
    let wallet = Wallet::from_seed(&devnet_seed(0))?;
    let nonce = provider.get_nonce(&wallet.address()).await?;
    let chain_id = provider.chain_id().await?;
    let recipient = Address([0xCEu8; 32]);
    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .chain_id(chain_id)
        .nonce(nonce)
        .gas_limit(100_000)
        .transfer(recipient, 1_000_000)
        .build()?;
    wallet.sign_tx(&mut tx).await?;
    let plaintext_hash = pyde_rust_sdk::tx::tx_hash(&tx);
    let plaintext_bytes = pyde_rust_sdk::tx::encode(&tx)?;
    println!(
        "signed tx: {} bytes, plaintext_hash={plaintext_hash}",
        plaintext_bytes.len()
    );

    // 3. Threshold-encrypt under the epoch pubkey — use to_wire_bytes
    //    so the engine's admit-side borsh decoder accepts the
    //    ciphertext field.
    let ct = threshold_encrypt(&tpk, &plaintext_bytes)
        .map_err(|e| anyhow::anyhow!("threshold_encrypt: {e}"))?;
    let envelope = EncryptedTxEnvelope {
        version: EncryptedTxEnvelope::VERSION,
        ciphertext: ct.to_wire_bytes(),
    };
    println!(
        "ciphertext: {} bytes (min={}, max={})",
        envelope.ciphertext.len(),
        EncryptedTxEnvelope::MIN_CIPHERTEXT_LEN,
        EncryptedTxEnvelope::MAX_CIPHERTEXT_LEN
    );
    let envelope_hash_local = envelope.envelope_hash();
    let envelope_hex = format!("0x{}", hex::encode(borsh::to_vec(&envelope)?));

    // 4. Submit via the SDK's encrypted-mempool endpoint.
    let envelope_hash_returned = provider
        .send_raw_encrypted_transaction(&envelope_hex)
        .await?;
    println!("submitted envelope. hash returned: {envelope_hash_returned}");
    println!("  local hash for verification:   {envelope_hash_local}");
    assert_eq!(
        envelope_hash_local, envelope_hash_returned,
        "envelope hash mismatch — SDK and engine disagree on Blake3(version || len || ciphertext)"
    );
    println!("  ✓ envelope hashes match");

    // 5. Poll for the *plaintext* receipt — the inner Tx's hash, not
    //    the envelope hash. Wave-commit decryption fires within a
    //    few waves on the single-validator devnet.
    println!("waiting for plaintext receipt under {plaintext_hash}...");
    for i in 0..120 {
        if let Some(receipt) = provider.get_transaction_receipt(&plaintext_hash).await? {
            println!(
                "✓ committed: status={:?} wave={} gas_used={}",
                receipt.status,
                receipt.wave_id_u64(),
                receipt.gas()
            );
            return Ok(());
        }
        if i % 10 == 9 {
            let head = provider.wave_id().await.unwrap_or(0);
            println!("  still waiting ({}s, head wave: {head})", i + 1);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    Err(anyhow::anyhow!(
        "plaintext receipt didn't land within 120s — either the chain is paused or \
         the decryption ceremony isn't firing (check engine logs for MEV signals)"
    ))
}
