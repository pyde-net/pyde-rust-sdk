//! Remaining audit gap closures — decimal edges, AuthKeys variants,
//! Receipt strict accessors, TxBuilder less-travelled paths.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pyde_rust_sdk::tx::{tx_hash, TxBuilder};
use pyde_rust_sdk::types::{
    AccessEntry, AccessType, AuthKeys, FalconPubkey, FeePayer, Receipt, ReceiptStatus,
    FALCON_PUBKEY_LEN, MAX_MULTISIG_SIGNERS,
};
use pyde_rust_sdk::util::{format_units, parse_units};
use pyde_rust_sdk::{Address, SdkError};

// ── util: decimal edges ────────────────────────────────────────

#[test]
fn parse_units_handles_18_decimals_eth_convention() {
    let raw = parse_units("1.5", 18).unwrap();
    assert_eq!(raw, 1_500_000_000_000_000_000u128);
}

#[test]
fn parse_units_handles_38_decimals_u128_max_precision() {
    let raw = parse_units("0.1", 38).unwrap();
    // 0.1 with 38 decimals = 10^37
    assert_eq!(raw, 10u128.pow(37));
}

#[test]
fn parse_units_rejects_decimals_above_38() {
    let err = parse_units("1", 39).unwrap_err();
    assert!(err.contains("u128"));
}

#[test]
fn parse_units_rejects_negative() {
    let err = parse_units("-1.0", 9).unwrap_err();
    assert!(err.contains("negative"));
}

#[test]
fn parse_units_rejects_extra_dots() {
    assert!(parse_units("1.0.0", 9).is_err());
}

#[test]
fn parse_units_rejects_too_many_fractional_digits() {
    let err = parse_units("1.123456789012", 9).unwrap_err();
    assert!(err.contains("decimal"));
}

#[test]
fn parse_units_rejects_non_digit() {
    assert!(parse_units("12abc", 9).is_err());
}

#[test]
fn format_units_zero_decimals() {
    assert_eq!(format_units(42, 0), "42.0");
}

#[test]
fn format_units_decimals_above_38_falls_back_to_raw() {
    assert_eq!(format_units(100, 39), "100");
}

#[test]
fn parse_units_overflow_rejected() {
    let huge = format!("{}.0", u128::MAX);
    let err = parse_units(&huge, 9).unwrap_err();
    // u128::MAX × 10^9 overflows u128 → parse_units catches it.
    assert!(err.to_lowercase().contains("overflow"));
}

// ── AuthKeys::validate edge cases ──────────────────────────────

fn pk(byte: u8) -> FalconPubkey {
    FalconPubkey::new([byte; FALCON_PUBKEY_LEN])
}

#[test]
fn auth_keys_validate_threshold_zero_fails() {
    let a = AuthKeys::MultiSig {
        threshold: 0,
        signers: vec![pk(1)],
    };
    assert!(a.validate().is_err());
}

#[test]
fn auth_keys_validate_threshold_greater_than_signers_fails() {
    let a = AuthKeys::MultiSig {
        threshold: 5,
        signers: vec![pk(1), pk(2)],
    };
    assert!(a.validate().is_err());
}

#[test]
fn auth_keys_validate_threshold_equals_signer_count_ok() {
    let a = AuthKeys::MultiSig {
        threshold: 2,
        signers: vec![pk(1), pk(2)],
    };
    assert!(a.validate().is_ok());
}

#[test]
fn auth_keys_validate_max_signer_set_ok() {
    let signers = (0..MAX_MULTISIG_SIGNERS as u8).map(pk).collect();
    let a = AuthKeys::MultiSig {
        threshold: MAX_MULTISIG_SIGNERS as u8,
        signers,
    };
    assert!(a.validate().is_ok());
}

#[test]
fn auth_keys_validate_overflow_signer_set_fails() {
    let signers = (0..(MAX_MULTISIG_SIGNERS + 1) as u8).map(pk).collect();
    let a = AuthKeys::MultiSig {
        threshold: 1,
        signers,
    };
    assert!(a.validate().is_err());
}

#[test]
fn auth_keys_validate_empty_signers_fails() {
    let a = AuthKeys::MultiSig {
        threshold: 1,
        signers: vec![],
    };
    assert!(a.validate().is_err());
}

#[test]
fn auth_keys_programmable_rejected_in_v1() {
    let a = AuthKeys::Programmable {
        policy: vec![0u8; 4],
    };
    assert!(!a.is_v1_supported());
}

// ── Receipt strict accessors ──────────────────────────────────

fn empty_receipt() -> Receipt {
    Receipt {
        tx_hash: "0x00".into(),
        wave_id: "0x10".into(),
        tx_index: "0x2".into(),
        nonce: "0x0".into(),
        commit_reveal: false,
        status: ReceiptStatus::Success,
        gas_used: "0x5208".into(),
        fee_paid: "0xABCD".into(),
        return_data: "0xDEADBEEF".into(),
        events: vec![],
        revert_reason: None,
    }
}

#[test]
fn receipt_try_gas_ok() {
    let r = empty_receipt();
    assert_eq!(r.try_gas().unwrap(), 0x5208);
}

#[test]
fn receipt_try_gas_rejects_garbage() {
    let mut r = empty_receipt();
    r.gas_used = "not-hex".into();
    let err = r.try_gas().unwrap_err();
    assert!(matches!(err, SdkError::InvalidResponse(_)));
}

#[test]
fn receipt_try_fee_paid_ok() {
    assert_eq!(empty_receipt().try_fee_paid_quanta().unwrap(), 0xABCD);
}

#[test]
fn receipt_try_wave_id_rejects_garbage() {
    let mut r = empty_receipt();
    r.wave_id = "0xZZZZ".into();
    assert!(r.try_wave_id_u64().is_err());
}

#[test]
fn receipt_try_tx_index_rejects_garbage() {
    let mut r = empty_receipt();
    r.tx_index = "0xZZZZ".into();
    assert!(r.try_tx_index_u32().is_err());
}

#[test]
fn receipt_try_return_bytes_ok() {
    assert_eq!(
        empty_receipt().try_return_bytes().unwrap(),
        vec![0xDE, 0xAD, 0xBE, 0xEF]
    );
}

#[test]
fn receipt_try_return_bytes_rejects_garbage() {
    let mut r = empty_receipt();
    r.return_data = "not-hex".into();
    assert!(r.try_return_bytes().is_err());
}

#[test]
fn receipt_lossy_gas_falls_back_to_zero() {
    // Sanity: the lossy `gas()` returns 0 on garbage so existing
    // callers don't break.
    let mut r = empty_receipt();
    r.gas_used = "not-hex".into();
    assert_eq!(r.gas(), 0);
}

// ── TxBuilder less-travelled paths ────────────────────────────

#[test]
fn tx_builder_paymaster_fee_payer_changes_hash() {
    let from = Address::new([0x10; 32]);
    let to = Address::new([0x20; 32]);
    let paymaster = Address::new([0x30; 32]);

    let a = TxBuilder::new()
        .from(from)
        .to(to)
        .chain_id(31337)
        .nonce(0)
        .build()
        .unwrap();
    let b = TxBuilder::new()
        .from(from)
        .to(to)
        .chain_id(31337)
        .nonce(0)
        .fee_payer(FeePayer::Paymaster(paymaster))
        .build()
        .unwrap();
    assert_ne!(tx_hash(&a), tx_hash(&b));
}

#[test]
fn tx_builder_gas_tank_fee_payer_changes_hash() {
    let from = Address::new([0x10; 32]);
    let to = Address::new([0x20; 32]);

    let a = TxBuilder::new()
        .from(from)
        .to(to)
        .chain_id(31337)
        .nonce(0)
        .build()
        .unwrap();
    let b = TxBuilder::new()
        .from(from)
        .to(to)
        .chain_id(31337)
        .nonce(0)
        .fee_payer(FeePayer::GasTank)
        .build()
        .unwrap();
    assert_ne!(tx_hash(&a), tx_hash(&b));
}

#[test]
fn tx_builder_access_list_propagates() {
    let from = Address::new([0x10; 32]);
    let entry = AccessEntry {
        address: Address::new([0x99; 32]),
        storage_keys: vec![[0u8; 32], [0xFFu8; 32]],
        access_type: AccessType::ReadWrite,
    };
    let tx = TxBuilder::new()
        .from(from)
        .chain_id(31337)
        .nonce(0)
        .access_list(vec![entry.clone()])
        .build()
        .unwrap();
    assert_eq!(tx.access_list, vec![entry]);
}

#[test]
fn tx_builder_deadline_can_be_cleared() {
    let from = Address::new([0x10; 32]);
    let tx = TxBuilder::new()
        .from(from)
        .chain_id(31337)
        .nonce(0)
        .deadline(500)
        .clear_deadline()
        .build()
        .unwrap();
    assert_eq!(tx.deadline, None);
}

#[test]
fn tx_builder_deadline_changes_hash() {
    let from = Address::new([0x10; 32]);
    let a = TxBuilder::new()
        .from(from)
        .chain_id(31337)
        .nonce(0)
        .build()
        .unwrap();
    let b = TxBuilder::new()
        .from(from)
        .chain_id(31337)
        .nonce(0)
        .deadline(500)
        .build()
        .unwrap();
    assert_ne!(tx_hash(&a), tx_hash(&b));
}
