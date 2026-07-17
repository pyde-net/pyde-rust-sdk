//! Persist a wallet to disk encrypted under a password, then load it back.
//!
//! Pure-local example — no RPC connection needed. Writes a temp file
//! that the example cleans up at the end.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example keystore
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::unwrap_used
)]

use std::path::Path;

use pyde_rust_sdk::wallet::Keystore;
use pyde_rust_sdk::Wallet;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let password = "correct horse battery staple";
    let path = Path::new("/tmp/pyde-keystore-example.json");

    // Save into a canonical keystore vault under a named account.
    let wallet = Wallet::generate()?;
    println!("created wallet: {}", wallet.address());
    let keystore = wallet.to_keystore("my-account", password)?;
    std::fs::write(path, serde_json::to_vec_pretty(&keystore)?)?;
    println!("wrote keystore: {}", path.display());

    // Load.
    let bytes = std::fs::read(path)?;
    let keystore: Keystore = serde_json::from_slice(&bytes)?;
    let restored = Wallet::from_keystore(&keystore, "my-account", password)?;
    println!("restored wallet: {}", restored.address());
    assert_eq!(restored.address(), wallet.address(), "address mismatch");

    // Demonstrate wrong-password rejection.
    let err = Wallet::from_keystore(&keystore, "my-account", "wrong password").unwrap_err();
    println!("wrong-password error: {err}");

    // Cleanup.
    let _ = std::fs::remove_file(path);
    Ok(())
}
