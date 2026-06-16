//! Multi-contract orchestration regression test.
//!
//! Existing tests cover Provider methods in isolation
//! (`provider_http.rs`) and Contract behavior on a single instance
//! (`contract_runtime.rs`). Neither pins the full SDK code path that
//! a real dapp drives: deploy/load → state-mutating send → poll for
//! receipt → read view returns, repeated across several contract
//! instances with the same signer threading nonces.
//!
//! This file mocks the JSON-RPC surface for that sequential flow
//! across two counter-shaped contracts and asserts the SDK
//! - dispatches to the right contract via `to` address,
//! - threads send → wait_for_receipt → call cleanly across both,
//! - cross-checks receipt `tx_hash` against the polled hash,
//! - decodes return data via the ABI on each view call.
//!
//! Wiremock dispatches via `body_string_contains` on the contract's
//! 32-byte address hex — both `pyde_sendRawTransaction` (whole tx
//! body hex-encoded; `to` bytes appear inline) and
//! `pyde_call` / `pyde_getTransactionReceipt` (address / hash
//! present as hex string in `params`) carry enough to disambiguate.
//!
//! ⚠️ Keep this test free of contract-deployment mocks. Deploy
//! exercises a different code path (`Contract::load_at` +
//! `extract_abi`) already covered by `contract_runtime.rs`. The
//! orchestration value here is in the *send → wait → call* cycle.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::provider::{HttpTransport, Provider, RootProvider};
use pyde_rust_sdk::signer::LocalSigner;
use pyde_rust_sdk::types::{
    Address, ContractAbi, ContractType, EventAbi, FunctionAbi, FunctionAttrs, ParamAbi, ParamType,
    ReceiptStatus, StateSchema,
};
use serde_json::{json, Value as JsonValue};
use wiremock::matchers::{body_partial_json, body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn match_method(name: &str) -> wiremock::matchers::BodyPartialJsonMatcher {
    body_partial_json(json!({ "method": name }))
}

fn ok(result: JsonValue) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    }))
}

/// Minimal counter ABI: `increment()` (entry, state-mutating) and
/// `get() -> u64` (entry+view). One event `Incremented(by: address)`.
fn counter_abi() -> ContractAbi {
    ContractAbi {
        pyde_abi_version: ContractAbi::V1_2,
        contract_type: ContractType::Contract,
        name: "counter".into(),
        version: "0.1.0".into(),
        functions: vec![
            FunctionAbi {
                selector: [1, 0, 0, 0],
                name: "increment".into(),
                attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY),
                params: vec![],
                returns: None,
            },
            FunctionAbi {
                selector: [2, 0, 0, 0],
                name: "get".into(),
                attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY | FunctionAttrs::VIEW),
                params: vec![],
                returns: Some(ParamType::U64),
            },
        ],
        events: vec![EventAbi {
            name: "Incremented".into(),
            params: vec![ParamAbi {
                name: "by".into(),
                ty: ParamType::Address,
            }],
            indexed_mask: 0b1,
        }],
        state_schema: StateSchema::empty(),
        state_schema_hash: [0; 32],
        constructor_index: None,
        fallback_index: None,
        receive_index: None,
        parachain_imports: vec![],
        types: vec![],
    }
}

/// Hex of a contract address without the `0x` prefix — used for
/// substring-based wiremock dispatch.
fn addr_hex(a: &Address) -> String {
    hex::encode(a.as_bytes())
}

/// Build the success-receipt JSON the engine ships from
/// `pyde_getTransactionReceipt` for a given tx hash.
fn success_receipt_json(tx_hash_no_prefix: &str, wave_id: u64, tx_index: u32) -> JsonValue {
    json!({
        "tx_hash": format!("0x{tx_hash_no_prefix}"),
        "wave_id": format!("0x{wave_id:x}"),
        "tx_index": format!("0x{tx_index:x}"),
        "status": "success",
        "gas_used": "0x186a0",
        "fee_paid": "0x186a0",
        "return_data": "0x",
        "events": [],
    })
}

#[tokio::test]
async fn orchestration_two_contracts_send_wait_call_cycle() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));

    let signer = LocalSigner::from_seed(&[7u8; 32]).unwrap();

    let counter_a_addr = Address::new([0xAA; 32]);
    let counter_b_addr = Address::new([0xBB; 32]);
    let counter_a = Contract::new(counter_a_addr, counter_abi(), Arc::clone(&provider));
    let counter_b = Contract::new(counter_b_addr, counter_abi(), Arc::clone(&provider));

    // Hashes the mock-engine "assigns" to each contract's send.
    // The first byte echoes the contract (0xAA / 0xBB) so any
    // mismatch surfaces as a cross-check failure in PendingTx.
    let send_hash_a = "aa".to_string() + &"01".repeat(31);
    let send_hash_b = "bb".to_string() + &"02".repeat(31);

    // ── Chain info + nonce ──────────────────────────────────────
    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok(json!("0x1")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionCount"))
        .respond_with(ok(json!("0x0")))
        .mount(&server)
        .await;

    // ── send_raw_transaction: dispatched per-contract ───────────
    let a_hex = addr_hex(&counter_a_addr);
    let b_hex = addr_hex(&counter_b_addr);
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .and(body_string_contains(a_hex.clone()))
        .respond_with(ok(json!(format!("0x{send_hash_a}"))))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .and(body_string_contains(b_hex.clone()))
        .respond_with(ok(json!(format!("0x{send_hash_b}"))))
        .expect(1)
        .mount(&server)
        .await;

    // ── get_transaction_receipt: dispatched per-hash ────────────
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .and(body_string_contains(send_hash_a.clone()))
        .respond_with(ok(success_receipt_json(&send_hash_a, 1, 0)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .and(body_string_contains(send_hash_b.clone()))
        .respond_with(ok(success_receipt_json(&send_hash_b, 2, 0)))
        .mount(&server)
        .await;

    // ── pyde_call: each returns u64(1) for `get()` ──────────────
    // Borsh u64 LE = 8 bytes; "1" → "0100000000000000".
    let u64_one_hex = format!("0x{}", hex::encode(1u64.to_le_bytes()));
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .and(body_string_contains(a_hex))
        .respond_with(ok(json!(u64_one_hex.clone())))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .and(body_string_contains(b_hex))
        .respond_with(ok(json!(u64_one_hex)))
        .mount(&server)
        .await;

    // ── Drive the flow ──────────────────────────────────────────

    // A: send + wait + read
    let pending_a = counter_a
        .send(&signer, "increment", vec![], 100_000, 0)
        .await
        .unwrap();
    let receipt_a = pending_a.wait_for_receipt().await.unwrap();
    assert!(matches!(receipt_a.status, ReceiptStatus::Success));
    assert_eq!(receipt_a.tx_hash, format!("0x{send_hash_a}"));
    assert_eq!(receipt_a.wave_id_u64(), 1);

    let value_a = counter_a.call("get", vec![]).await.unwrap();
    assert_eq!(value_a, Some(Value::U64(1)));

    // B: send + wait + read — same signer, distinct contract
    let pending_b = counter_b
        .send(&signer, "increment", vec![], 100_000, 0)
        .await
        .unwrap();
    let receipt_b = pending_b.wait_for_receipt().await.unwrap();
    assert!(matches!(receipt_b.status, ReceiptStatus::Success));
    assert_eq!(receipt_b.tx_hash, format!("0x{send_hash_b}"));
    assert_eq!(receipt_b.wave_id_u64(), 2);

    let value_b = counter_b.call("get", vec![]).await.unwrap();
    assert_eq!(value_b, Some(Value::U64(1)));

    // ── Orchestration invariants ────────────────────────────────

    // Distinct contracts produced distinct send hashes.
    assert_ne!(receipt_a.tx_hash, receipt_b.tx_hash);

    // The PendingTx handles returned the same hash the receipts
    // came back under (cross-check survives both contracts).
    assert_eq!(
        format!("0x{}", hex::encode(pending_a.hash().as_bytes())),
        receipt_a.tx_hash
    );
    assert_eq!(
        format!("0x{}", hex::encode(pending_b.hash().as_bytes())),
        receipt_b.tx_hash
    );

    // The two send mocks each matched exactly once — wiremock
    // would panic on drop otherwise via .expect(1).
}

#[tokio::test]
async fn orchestration_pending_tx_rejects_mismatched_receipt() {
    // Regression test for the PendingTx cross-check: if the engine
    // (or a malicious peer) serves a receipt under the wrong
    // tx_hash, wait_for_receipt must surface InvalidResponse and
    // not silently succeed.
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let signer = LocalSigner::from_seed(&[8u8; 32]).unwrap();
    let addr = Address::new([0xCC; 32]);
    let contract = Contract::new(addr, counter_abi(), Arc::clone(&provider));

    let real_hash = "cc".to_string() + &"01".repeat(31);
    let wrong_hash = "ff".to_string() + &"99".repeat(31);

    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok(json!("0x1")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionCount"))
        .respond_with(ok(json!("0x0")))
        .mount(&server)
        .await;
    // send_raw_transaction returns real_hash, but the receipt
    // claims a different hash — should error, not succeed.
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok(json!(format!("0x{real_hash}"))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok(success_receipt_json(&wrong_hash, 1, 0)))
        .mount(&server)
        .await;

    let pending = contract
        .send(&signer, "increment", vec![], 100_000, 0)
        .await
        .unwrap();
    let err = pending.wait_for_receipt().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("doesn't match polled hash"),
        "expected cross-check error, got: {msg}"
    );
}

#[tokio::test]
async fn orchestration_call_after_send_uses_fresh_provider_state() {
    // Variation: a single contract drives send → wait → call →
    // send → wait → call. Verifies the SDK doesn't accidentally
    // cache state from the first cycle into the second's view
    // call (return data should come fresh from the second
    // pyde_call, not be reused from any prior decode).
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let signer = LocalSigner::from_seed(&[9u8; 32]).unwrap();
    let addr = Address::new([0xDD; 32]);
    let contract = Contract::new(addr, counter_abi(), Arc::clone(&provider));

    let hash1 = "dd".to_string() + &"01".repeat(31);
    let hash2 = "dd".to_string() + &"02".repeat(31);

    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok(json!("0x1")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionCount"))
        .respond_with(ok(json!("0x0")))
        .mount(&server)
        .await;

    // Two sequential sends. wiremock matches FIFO when multiple
    // mocks would match — give the second send a non-overlapping
    // hash by mounting an `up_to_n_times(1)` matcher first, then
    // a fallback that catches the second.
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok(json!(format!("0x{hash1}"))))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok(json!(format!("0x{hash2}"))))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .and(body_string_contains(hash1.clone()))
        .respond_with(ok(success_receipt_json(&hash1, 1, 0)))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .and(body_string_contains(hash2.clone()))
        .respond_with(ok(success_receipt_json(&hash2, 2, 0)))
        .mount(&server)
        .await;

    // First view call returns 1, second returns 2 — distinct
    // values, distinct mocks, FIFO-ordered.
    let one_hex = format!("0x{}", hex::encode(1u64.to_le_bytes()));
    let two_hex = format!("0x{}", hex::encode(2u64.to_le_bytes()));
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .respond_with(ok(json!(one_hex)))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .respond_with(ok(json!(two_hex)))
        .mount(&server)
        .await;

    // First cycle
    let p1 = contract
        .send(&signer, "increment", vec![], 100_000, 0)
        .await
        .unwrap();
    let r1 = p1.wait_for_receipt().await.unwrap();
    assert_eq!(r1.tx_hash, format!("0x{hash1}"));
    let v1 = contract.call("get", vec![]).await.unwrap();
    assert_eq!(v1, Some(Value::U64(1)));

    // Second cycle — fresh send, fresh receipt, fresh view
    let p2 = contract
        .send(&signer, "increment", vec![], 100_000, 0)
        .await
        .unwrap();
    let r2 = p2.wait_for_receipt().await.unwrap();
    assert_eq!(r2.tx_hash, format!("0x{hash2}"));
    let v2 = contract.call("get", vec![]).await.unwrap();
    assert_eq!(v2, Some(Value::U64(2)));

    assert_ne!(p1.hash(), p2.hash());
}
