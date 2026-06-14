//! Keystore corruption + edge-case tests. Audit M5 closure: pin
//! every rejection path so a future refactor can't silently widen
//! what the decrypt accepts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pyde_rust_sdk::wallet::Keystore;
use pyde_rust_sdk::{SdkError, Wallet};

fn fresh_keystore(password: &str) -> (Wallet, Keystore) {
    let wallet = Wallet::generate().unwrap();
    let ks = wallet.to_keystore(password).unwrap();
    (wallet, ks)
}

#[test]
fn wrong_password_rejected() {
    let (_w, ks) = fresh_keystore("correct horse battery staple");
    let err = Wallet::from_keystore(&ks, "WRONG password").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn unknown_version_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.version = 999;
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("version")));
}

#[test]
fn unknown_kdf_name_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.kdf.name = "scrypt".into();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("KDF")));
}

#[test]
fn unknown_cipher_name_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.cipher.name = "chacha20-poly1305".into();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("cipher")));
}

#[test]
fn salt_with_wrong_length_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.kdf.params.salt = "0xdeadbeef".into(); // 4 bytes, expected 16
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("salt")));
}

#[test]
fn nonce_with_wrong_length_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.cipher.nonce = "0xdeadbeef".into(); // 4 bytes, expected 12
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("nonce")));
}

#[test]
fn malformed_hex_in_salt_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.kdf.params.salt = "0xZZZZ".into();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn truncated_ciphertext_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    // Truncate the ciphertext so the GCM tag is invalid.
    let mut hex_str = ks.cipher.ciphertext.trim_start_matches("0x").to_string();
    hex_str.truncate(hex_str.len() - 4);
    ks.cipher.ciphertext = format!("0x{hex_str}");
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_ciphertext_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    // Flip one byte in the ciphertext — GCM tag fails.
    let mut chars: Vec<char> = ks.cipher.ciphertext.chars().collect();
    let idx = chars.len() / 2;
    chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
    ks.cipher.ciphertext = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_address_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    let mut chars: Vec<char> = ks.address.chars().collect();
    let idx = chars.len() - 4;
    chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
    ks.address = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    // The recomputed address mismatch path OR the AAD failure path
    // — both surface as InvalidArgument.
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_pubkey_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    let mut chars: Vec<char> = ks.pubkey.chars().collect();
    let idx = 100;
    chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
    ks.pubkey = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn pubkey_address_mismatch_rejected() {
    // Generate two wallets; mix their pubkey + address — the
    // derivation check rejects.
    let (a, _) = fresh_keystore("pw");
    let (_b, mut ks) = fresh_keystore("pw");
    ks.pubkey = a.pubkey().to_hex();
    let err = Wallet::from_keystore(&ks, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("address")));
}

#[test]
fn round_trip_through_json() {
    let (wallet, ks) = fresh_keystore("pw");
    let json = serde_json::to_string_pretty(&ks).unwrap();
    let parsed: Keystore = serde_json::from_str(&json).unwrap();
    let restored = Wallet::from_keystore(&parsed, "pw").unwrap();
    assert_eq!(restored.address(), wallet.address());
}

#[test]
fn each_write_produces_distinct_salt_and_nonce() {
    let (wallet, _) = fresh_keystore("pw");
    let a = wallet.to_keystore("pw").unwrap();
    let b = wallet.to_keystore("pw").unwrap();
    assert_ne!(a.kdf.params.salt, b.kdf.params.salt);
    assert_ne!(a.cipher.nonce, b.cipher.nonce);
    assert_ne!(a.cipher.ciphertext, b.cipher.ciphertext);
}
