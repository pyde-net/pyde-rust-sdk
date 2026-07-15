//! Cross-repo wire parity for the commit-reveal private mempool.
//!
//! Pins the SDK's commit/reveal encoding to the canonical vectors in
//! `otigen/crates/otigen-tx-codec/fixtures/otigen_commit_reveal_vectors_v1.json`
//! (same `FixtureFile` schema + pinned FALCON key as the shared
//! `cross_repo_tx_vectors_v1.json`). For each vector we:
//!
//! 1. rebuild the outer `Tx` from the fixture fields and assert its
//!    canonical `tx_hash` matches `expected_tx_hash_hex` — this covers
//!    `TxType::Commit`/`Reveal` tagging and the whole outer encoding;
//! 2. (reveal) borsh-decode the `RevealPayload` and recompute
//!    `commitment_hash(inner_tx, nonce)`, asserting it matches the
//!    embedded commitment — this covers the Blake3 commitment;
//! 3. (commit) borsh-decode the `CommitPayload` and assert the tx's
//!    `value` equals `required_bond(value_ceiling)`.
//!
//! The fixture is repo-local (not shipped with the published crate), so
//! the test skips gracefully when it isn't present.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stderr
)]

use pyde_rust_sdk::tx::{commitment_hash, required_bond, tx_hash};
use pyde_rust_sdk::types::{
    Address, CommitPayload, FalconSignature, FeePayer, RevealPayload, Tx, TxType,
};

fn fixture_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../otigen/crates/otigen-tx-codec/fixtures/otigen_commit_reveal_vectors_v1.json")
}

fn addr_from_hex(s: &str) -> Address {
    let bytes = hex::decode(s.trim_start_matches("0x")).unwrap();
    let mut a = [0u8; 32];
    a.copy_from_slice(&bytes);
    Address::new(a)
}

fn tx_type_from_str(s: &str) -> TxType {
    match s {
        "commit" => TxType::Commit,
        "reveal" => TxType::Reveal,
        other => panic!("unexpected tx_type in commit-reveal fixture: {other}"),
    }
}

/// Rebuild the unsigned outer `Tx` from a fixture `input` object. The
/// signature is excluded from the canonical hash, so an empty signature
/// is fine for `tx_hash` parity.
fn tx_from_input(input: &serde_json::Value) -> Tx {
    let fee_payer = match input["fee_payer"]["kind"].as_str().unwrap() {
        "sender" => FeePayer::Sender,
        "gas_tank" => FeePayer::GasTank,
        other => panic!("commit-reveal fixture uses unexpected fee_payer: {other}"),
    };
    assert!(
        input["access_list"].as_array().unwrap().is_empty(),
        "commit-reveal fixtures declare an empty access list",
    );
    let deadline = match &input["deadline"] {
        serde_json::Value::Null => None,
        v => Some(v.as_u64().unwrap()),
    };
    Tx {
        from: addr_from_hex(input["from_hex"].as_str().unwrap()),
        to: addr_from_hex(input["to_hex"].as_str().unwrap()),
        value: input["value_dec"]
            .as_str()
            .unwrap()
            .parse::<u128>()
            .unwrap(),
        data: hex::decode(input["data_hex"].as_str().unwrap()).unwrap(),
        gas_limit: input["gas_limit"].as_u64().unwrap(),
        nonce: input["nonce"].as_u64().unwrap(),
        signature: FalconSignature::new(Vec::new()),
        fee_payer,
        access_list: Vec::new(),
        deadline,
        chain_id: input["chain_id"].as_u64().unwrap(),
        tx_type: tx_type_from_str(input["tx_type"].as_str().unwrap()),
    }
}

#[test]
fn commit_reveal_vectors_match_canonical_wire() {
    let path = fixture_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        eprintln!(
            "skipping commit-reveal parity: fixture not found at {}",
            path.display()
        );
        return;
    };
    let file: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let vectors = file["vectors"].as_array().expect("fixture has vectors[]");
    assert!(
        !vectors.is_empty(),
        "fixture must carry at least one vector"
    );

    let mut saw_commit = false;
    let mut saw_reveal = false;

    for vec in vectors {
        let input = &vec["input"];
        let name = input["name"].as_str().unwrap_or("<unnamed>");
        let tx = tx_from_input(input);

        // (1) Outer canonical tx_hash parity — the load-bearing check.
        let got = hex::encode(tx_hash(&tx).as_bytes());
        let expected = vec["expected_tx_hash_hex"].as_str().unwrap();
        assert_eq!(
            got, expected,
            "tx_hash mismatch for vector {name}: SDK computed {got}, fixture expects {expected}"
        );

        match tx.tx_type {
            TxType::Commit => {
                saw_commit = true;
                // (3) required_bond parity: value == required_bond(ceiling).
                let payload: CommitPayload = borsh::from_slice(&tx.data).unwrap();
                assert_eq!(
                    tx.value,
                    required_bond(payload.value_ceiling),
                    "commit vector {name}: tx.value must equal required_bond(value_ceiling)"
                );
            }
            TxType::Reveal => {
                saw_reveal = true;
                // (2) commitment_hash parity over the embedded inner tx.
                let payload: RevealPayload = borsh::from_slice(&tx.data).unwrap();
                let recomputed = commitment_hash(&payload.inner_tx, &payload.nonce);
                assert_eq!(
                    recomputed, payload.commitment,
                    "reveal vector {name}: commitment_hash(inner_tx, nonce) must match the \
                     committed hash"
                );
            }
            other => panic!("unexpected tx_type {other:?} in commit-reveal fixture"),
        }
    }

    assert!(saw_commit, "fixture should contain a commit vector");
    assert!(saw_reveal, "fixture should contain a reveal vector");
}
