//! Keystore corruption + edge-case tests. Pins every rejection path
//! for the canonical multi-account keystore so a future refactor
//! can't silently widen what the decrypt accepts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pyde_rust_sdk::wallet::{Keystore, KeystoreEntry};
use pyde_rust_sdk::{SdkError, Wallet};

const ACCT: &str = "acct";

fn fresh_keystore(password: &str) -> (Wallet, Keystore) {
    let wallet = Wallet::generate().unwrap();
    let ks = wallet.to_keystore(ACCT, password).unwrap();
    (wallet, ks)
}

fn entry(ks: &mut Keystore) -> &mut KeystoreEntry {
    ks.accounts.get_mut(ACCT).unwrap()
}

#[test]
fn wrong_password_rejected() {
    let (_w, ks) = fresh_keystore("correct horse battery staple");
    let err = Wallet::from_keystore(&ks, ACCT, "WRONG password").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn unknown_version_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    ks.version = 999;
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("version")));
}

#[test]
fn unknown_account_rejected() {
    let (_w, ks) = fresh_keystore("pw");
    assert!(Wallet::from_keystore(&ks, "nope", "pw").is_err());
}

#[test]
fn unknown_kdf_name_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).kdf.name = "scrypt".into();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("KDF")));
}

#[test]
fn kdf_above_clamp_rejected() {
    // Anti-DoS: a crafted memory_kb above the 1 GiB upper bound must be
    // rejected before any allocation. (There is deliberately no lower
    // floor reject.)
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).kdf.memory_kb = 2_097_152; // 2 GiB, above the clamp
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("upper bound")));
}

#[test]
fn unsupported_cipher_rejected() {
    // Only aes-256-gcm is accepted; anything else (including the old
    // ts-sdk chacha20-poly1305) is rejected.
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).cipher = "chacha20-poly1305".into();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("cipher")));
}

#[test]
fn salt_with_wrong_length_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).salt = "0xdeadbeef".into(); // 4 bytes, expected 16
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("salt")));
}

#[test]
fn nonce_with_wrong_length_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).nonce = "0xdeadbeef".into(); // 4 bytes, expected 12
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("nonce")));
}

#[test]
fn malformed_hex_in_salt_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    entry(&mut ks).salt = "0xZZZZ".into();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn truncated_ciphertext_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    let mut hex_str = entry(&mut ks)
        .ciphertext
        .trim_start_matches("0x")
        .to_string();
    hex_str.truncate(hex_str.len() - 4);
    entry(&mut ks).ciphertext = format!("0x{hex_str}");
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_ciphertext_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    let mut chars: Vec<char> = entry(&mut ks).ciphertext.chars().collect();
    let idx = chars.len() / 2;
    chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
    entry(&mut ks).ciphertext = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_address_rejected() {
    // No AAD in the canonical format, but the stored address must still
    // equal the address derived from the stored pubkey.
    let (_w, mut ks) = fresh_keystore("pw");
    let mut chars: Vec<char> = entry(&mut ks).address.chars().collect();
    let idx = chars.len() - 4;
    chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
    entry(&mut ks).address = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn tampered_pubkey_rejected() {
    let (_w, mut ks) = fresh_keystore("pw");
    let mut chars: Vec<char> = entry(&mut ks).pubkey.chars().collect();
    chars[100] = if chars[100] == '0' { '1' } else { '0' };
    entry(&mut ks).pubkey = chars.into_iter().collect();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(_)));
}

#[test]
fn pubkey_address_mismatch_rejected() {
    let (a, _) = fresh_keystore("pw");
    let (_b, mut ks) = fresh_keystore("pw");
    entry(&mut ks).pubkey = a.pubkey().to_hex();
    let err = Wallet::from_keystore(&ks, ACCT, "pw").unwrap_err();
    assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("address")));
}

#[test]
fn round_trip_through_json() {
    let (wallet, ks) = fresh_keystore("pw");
    let json = serde_json::to_string_pretty(&ks).unwrap();
    let parsed: Keystore = serde_json::from_str(&json).unwrap();
    let restored = Wallet::from_keystore(&parsed, ACCT, "pw").unwrap();
    assert_eq!(restored.address(), wallet.address());
}

#[test]
fn absent_cipher_field_defaults_to_aes() {
    // A canonical entry that omits `cipher` must read as aes-256-gcm.
    let (wallet, ks) = fresh_keystore("pw");
    let mut value = serde_json::to_value(&ks).unwrap();
    value["accounts"][ACCT]
        .as_object_mut()
        .unwrap()
        .remove("cipher");
    let json = value.to_string();
    let restored = Wallet::from_keystore_json(&json, ACCT, "pw").unwrap();
    assert_eq!(restored.address(), wallet.address());
}

#[test]
fn each_write_produces_distinct_salt_and_nonce() {
    let (wallet, _) = fresh_keystore("pw");
    let a = wallet.to_keystore(ACCT, "pw").unwrap();
    let b = wallet.to_keystore(ACCT, "pw").unwrap();
    let (ea, eb) = (a.accounts.get(ACCT).unwrap(), b.accounts.get(ACCT).unwrap());
    assert_ne!(ea.salt, eb.salt);
    assert_ne!(ea.nonce, eb.nonce);
    assert_ne!(ea.ciphertext, eb.ciphertext);
}
