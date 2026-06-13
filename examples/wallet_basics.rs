//! Generate a wallet, print its address and pubkey, sign a hash, verify.
//!
//! Pure-local example — no RPC connection needed.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example wallet_basics
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use pyde_rust_sdk::{Signer, TxHash, Wallet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Generate a fresh wallet from OS entropy.
    let wallet = Wallet::generate()?;
    println!("address: {}", wallet.address());
    println!(
        "pubkey:  {} ({} bytes)",
        wallet.pubkey().to_hex(),
        wallet.pubkey().as_bytes().len()
    );

    // Sign an arbitrary 32-byte hash.
    let hash = TxHash::new([0x42; 32]);
    let sig = wallet.sign_hash(&hash).await?;
    println!("signature length: {} bytes", sig.as_bytes().len());

    // Verify via pyde-crypto directly.
    let crypto_pk: pyde_crypto::falcon::FalconPublicKey = (&wallet.pubkey()).into();
    let crypto_sig =
        pyde_crypto::falcon::FalconSignature::from_bytes(sig.as_bytes()).expect("sig bytes");
    let ok = pyde_crypto::falcon::falcon_verify(&crypto_pk, hash.as_bytes(), &crypto_sig);
    println!("verification: {}", if ok { "OK" } else { "FAILED" });

    Ok(())
}
