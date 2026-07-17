//! Cross-implementation keystore parity — the acceptance gate from the
//! canonical keystore spec §8.
//!
//! A keystore minted by the `otigen` CLI (`otigen wallet new`) MUST
//! decrypt in this SDK to the exact FALCON secret key, and a wrong
//! password MUST be rejected with the single error variant. Because
//! AES-256-GCM is authenticated, a correct decrypt proves the
//! Argon2id-derived key matched the CLI's byte-for-byte.
//!
//! The golden fixture (`tests/fixtures/otigen-keystore.golden.json`)
//! was minted by `otigen wallet new golden` under the password below.
//! It notably OMITS the `cipher` field, exercising the spec's
//! "absent ⇒ aes-256-gcm" default.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pyde_rust_sdk::wallet::Keystore;
use pyde_rust_sdk::{SdkError, Wallet};

const GOLDEN_PASSWORD: &str = "test-password-12345";
const GOLDEN_ACCOUNT: &str = "golden";

fn golden_json() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/otigen-keystore.golden.json");
    std::fs::read_to_string(path).expect("golden fixture present")
}

/// The address stored in cleartext inside the fixture — the decrypted
/// key must derive back to exactly this.
fn golden_expected_address() -> String {
    let ks: Keystore = serde_json::from_str(&golden_json()).unwrap();
    ks.accounts.get(GOLDEN_ACCOUNT).unwrap().address.clone()
}

#[test]
fn decrypts_otigen_cli_keystore() {
    let expected = golden_expected_address();
    // Via the typed struct path.
    let ks: Keystore = serde_json::from_str(&golden_json()).unwrap();
    let wallet = Wallet::from_keystore(&ks, GOLDEN_ACCOUNT, GOLDEN_PASSWORD)
        .expect("otigen keystore must decrypt in the rust SDK");
    assert_eq!(
        wallet.address().to_hex(),
        expected,
        "decrypted key must derive back to the CLI-stored address",
    );
}

#[test]
fn decrypts_otigen_cli_keystore_via_json() {
    let expected = golden_expected_address();
    let wallet = Wallet::from_keystore_json(&golden_json(), GOLDEN_ACCOUNT, GOLDEN_PASSWORD)
        .expect("otigen keystore must decrypt via from_keystore_json");
    assert_eq!(wallet.address().to_hex(), expected);
}

#[test]
fn otigen_keystore_omits_cipher_and_uses_canonical_kdf() {
    // Guards the parity conditions: the CLI writes p=4 and may omit the
    // cipher field (which must default to aes-256-gcm).
    let ks: Keystore = serde_json::from_str(&golden_json()).unwrap();
    let entry = ks.accounts.get(GOLDEN_ACCOUNT).unwrap();
    assert_eq!(entry.kdf.name, "argon2id");
    assert_eq!(entry.kdf.memory_kb, 65_536);
    assert_eq!(entry.kdf.iterations, 3);
    assert_eq!(entry.kdf.parallelism, 4);
    // Absent in the raw JSON, defaulted on read.
    let raw: serde_json::Value = serde_json::from_str(&golden_json()).unwrap();
    assert!(raw["accounts"][GOLDEN_ACCOUNT].get("cipher").is_none());
    assert_eq!(entry.cipher, "aes-256-gcm");
}

#[test]
fn wrong_password_rejected_single_variant() {
    let ks: Keystore = serde_json::from_str(&golden_json()).unwrap();
    let err = Wallet::from_keystore(&ks, GOLDEN_ACCOUNT, "not the password").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}
