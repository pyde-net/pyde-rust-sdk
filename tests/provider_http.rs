//! HTTP provider integration tests against a wiremock-hosted JSON-RPC
//! server. Each test stamps a response for the method under test +
//! asserts the SDK decodes it the way the engine ships it.
//!
//! These tests are the closest thing the SDK has to a contract test
//! against the live engine — wiremock plays the role of the node, and
//! the SDK exercises its full HTTP transport / decode stack.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pyde_rust_sdk::provider::{HttpTransport, Provider, RootProvider};
use pyde_rust_sdk::types::{Address, EventFilter, LogFilter, TxHash, FALCON_PUBKEY_LEN};
use pyde_rust_sdk::Signer;
use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Build a wiremock-backed provider and return it alongside the
/// server so the test can stamp additional responses.
async fn provider_with_server() -> (Arc<RootProvider<HttpTransport>>, MockServer) {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    (Arc::new(RootProvider::new(transport)), server)
}

/// Build a JSON-RPC response wrapping `result`.
fn ok_response(result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    }))
}

/// Build a wiremock matcher that requires the incoming JSON body to
/// have `method == method_name` (other fields free).
fn match_method(method_name: &str) -> wiremock::matchers::BodyPartialJsonMatcher {
    body_partial_json(json!({ "method": method_name }))
}

// ── Chain info ──────────────────────────────────────────────────

#[tokio::test]
async fn chain_id_decodes_hex() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok_response(json!("0x1")))
        .mount(&server)
        .await;
    assert_eq!(provider.chain_id().await.unwrap(), 1);
}

#[tokio::test]
async fn wave_id_decodes_hex() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_waveId"))
        .respond_with(ok_response(json!("0x2a")))
        .mount(&server)
        .await;
    assert_eq!(provider.wave_id().await.unwrap(), 42);
}

#[tokio::test]
async fn get_node_info_decodes() {
    let (provider, server) = provider_with_server().await;
    let fake_pk = format!("0x{}", "ab".repeat(FALCON_PUBKEY_LEN));
    Mock::given(method("POST"))
        .and(match_method("pyde_getNodeInfo"))
        .respond_with(ok_response(json!({
            "peer_id": "deadbeef",
            "falcon_pubkey": fake_pk,
            "listen_addrs": ["/ip4/127.0.0.1/tcp/9000"],
            "agent_version": "pyde-node/0.1.0",
            "protocol_version": "pyde/1"
        })))
        .mount(&server)
        .await;
    let info = provider.get_node_info().await.unwrap();
    assert_eq!(info.peer_id, "deadbeef");
    assert_eq!(info.protocol_version, "pyde/1");
    assert_eq!(info.listen_addrs.len(), 1);
}

// ── Account state ───────────────────────────────────────────────

#[tokio::test]
async fn get_balance_decodes_quanta() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getBalance"))
        .respond_with(ok_response(json!("0x3b9aca00")))
        .mount(&server)
        .await;
    let addr = Address::new([0x42; 32]);
    let balance = provider.get_balance(&addr).await.unwrap();
    assert_eq!(balance, 1_000_000_000);
}

#[tokio::test]
async fn get_nonce_decodes_via_canonical_name() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getNonce"))
        .respond_with(ok_response(json!("0x7")))
        .mount(&server)
        .await;
    let addr = Address::new([0xAA; 32]);
    assert_eq!(provider.get_nonce(&addr).await.unwrap(), 7);
}

#[tokio::test]
async fn get_account_decodes_engine_shape() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getAccount"))
        .respond_with(ok_response(json!({
            "address": format!("0x{}", "aa".repeat(32)),
            "account_type": "eoa",
            "balance": "0x3b9aca00",
            "nonce": 5,
            "code_hash": format!("0x{}", "00".repeat(32)),
            "state_root": format!("0x{}", "00".repeat(32)),
        })))
        .mount(&server)
        .await;
    let addr = Address::new([0xAA; 32]);
    let account = provider.get_account(&addr).await.unwrap();
    assert_eq!(account.balance_quanta(), 1_000_000_000);
    assert_eq!(account.nonce, 5);
    assert!(!account.is_contract());
}

#[tokio::test]
async fn get_contract_code_decodes_bytes() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getContractCode"))
        .respond_with(ok_response(json!("0xdeadbeef")))
        .mount(&server)
        .await;
    let addr = Address::new([0xCC; 32]);
    assert_eq!(
        provider.get_contract_code(&addr).await.unwrap(),
        vec![0xDE, 0xAD, 0xBE, 0xEF]
    );
}

#[tokio::test]
async fn get_storage_slot_handles_none() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getStorageSlot"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    let slot = [0x77; 32];
    assert!(provider.get_storage_slot(&slot).await.unwrap().is_none());
}

#[tokio::test]
async fn get_storage_slot_decodes_bytes() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getStorageSlot"))
        .respond_with(ok_response(json!("0xcafe")))
        .mount(&server)
        .await;
    let slot = [0x77; 32];
    assert_eq!(
        provider.get_storage_slot(&slot).await.unwrap(),
        Some(vec![0xCA, 0xFE])
    );
}

#[tokio::test]
async fn resolve_name_handles_registered_and_unregistered() {
    let (provider, server) = provider_with_server().await;
    let registered = format!("0x{}", "11".repeat(32));
    Mock::given(method("POST"))
        .and(match_method("pyde_resolveName"))
        .respond_with(ok_response(json!(registered.clone())))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let resolved = provider.resolve_name("alice").await.unwrap().unwrap();
    assert_eq!(resolved.to_hex(), registered);

    // Next stub returns null → unregistered.
    Mock::given(method("POST"))
        .and(match_method("pyde_resolveName"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    assert!(provider.resolve_name("nobody").await.unwrap().is_none());
}

// ── Receipt ─────────────────────────────────────────────────────

#[tokio::test]
async fn get_receipt_decodes_hex_string_shape() {
    use pyde_rust_sdk::types::ReceiptStatus;
    let (provider, server) = provider_with_server().await;
    // Engine #335 aligned `pyde_getReceipt`'s wire shape to match
    // `pyde_getTransactionReceipt` — hex strings throughout. The
    // two endpoints now share the `Receipt` deserialiser.
    Mock::given(method("POST"))
        .and(match_method("pyde_getReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0xabababababababababababababababababababababababababababababababab",
            "wave_id": "0x5",
            "tx_index": "0x2",
            "status": "success",
            "gas_used": "0x5208",
            "fee_paid": "0x12345",
            "return_data": "0x",
            "events": []
        })))
        .mount(&server)
        .await;
    let hash = TxHash::new([0xAB; 32]);
    let receipt = provider.get_receipt(&hash).await.unwrap().unwrap();
    assert!(matches!(receipt.status, ReceiptStatus::Success));
    assert!(receipt.is_success());
    assert_eq!(receipt.gas(), 0x5208);
    assert_eq!(receipt.fee_paid_quanta(), 0x12345);
    assert_eq!(receipt.wave_id_u64(), 5);
    assert_eq!(receipt.tx_index_u32(), 2);
}

#[tokio::test]
async fn get_receipt_handles_null() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getReceipt"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    let hash = TxHash::new([0xAB; 32]);
    assert!(provider.get_receipt(&hash).await.unwrap().is_none());
}

// ── Structured revert_reason (engine #349) ──────────────────────

#[tokio::test]
async fn receipt_decodes_structured_engine_validation_revert_reason() {
    use pyde_rust_sdk::types::{ReceiptStatus, RevertCategory};
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0xabababababababababababababababababababababababababababababababab",
            "wave_id": "0x5",
            "tx_index": "0x0",
            "status": "reverted",
            "gas_used": "0x5208",
            "fee_paid": "0x5208",
            "return_data": "0x",
            "events": [],
            "revert_reason": {
                "category": "EngineValidation",
                "message": "nonce out of window: provided=17, window_start=18"
            }
        })))
        .mount(&server)
        .await;
    let hash = TxHash::new([0xAB; 32]);
    let r = provider
        .get_transaction_receipt(&hash)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(r.status, ReceiptStatus::Reverted));
    let reason = r.revert_reason.as_ref().expect("structured reason present");
    assert!(matches!(reason.category, RevertCategory::EngineValidation));
    assert!(reason.message.contains("nonce out of window"));
    assert!(r.is_engine_validation_revert());
    assert!(!r.is_contract_revert());
    assert!(!r.is_vm_trap());
}

#[tokio::test]
async fn receipt_decodes_structured_contract_revert_reason() {
    use pyde_rust_sdk::types::RevertCategory;
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0x".to_string() + &"c0".repeat(32),
            "wave_id": "0x1", "tx_index": "0x0",
            "status": "reverted", "gas_used": "0x100",
            "fee_paid": "0x100", "return_data": "0x", "events": [],
            "revert_reason": { "category": "Contract", "message": "ERR_FORBIDDEN" }
        })))
        .mount(&server)
        .await;
    let r = provider
        .get_transaction_receipt(&TxHash::new([0xC0; 32]))
        .await
        .unwrap()
        .unwrap();
    let reason = r.revert_reason.as_ref().unwrap();
    assert!(matches!(reason.category, RevertCategory::Contract));
    assert_eq!(reason.message, "ERR_FORBIDDEN");
    assert!(r.is_contract_revert());
}

#[tokio::test]
async fn receipt_decodes_structured_vm_revert_reason() {
    use pyde_rust_sdk::types::RevertCategory;
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0x".to_string() + &"f0".repeat(32),
            "wave_id": "0x1", "tx_index": "0x0",
            "status": "reverted", "gas_used": "0x100",
            "fee_paid": "0x100", "return_data": "0x", "events": [],
            "revert_reason": { "category": "Vm", "message": "Trap(MemoryOutOfBounds)" }
        })))
        .mount(&server)
        .await;
    let r = provider
        .get_transaction_receipt(&TxHash::new([0xF0; 32]))
        .await
        .unwrap()
        .unwrap();
    let reason = r.revert_reason.as_ref().unwrap();
    assert!(matches!(reason.category, RevertCategory::Vm));
    assert!(reason.message.contains("MemoryOutOfBounds"));
    assert!(r.is_vm_trap());
}

#[tokio::test]
async fn receipt_tolerates_unknown_category_for_forward_compat() {
    use pyde_rust_sdk::types::RevertCategory;
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0xff".repeat(32),
            "wave_id": "0x1", "tx_index": "0x0",
            "status": "reverted", "gas_used": "0x100",
            "fee_paid": "0x100", "return_data": "0x", "events": [],
            "revert_reason": { "category": "BrandNewCategoryEngineShippedThisMorning", "message": "..." }
        })))
        .mount(&server)
        .await;
    let r = provider
        .get_transaction_receipt(&TxHash::new([0xFF; 32]))
        .await
        .unwrap()
        .unwrap();
    let reason = r.revert_reason.as_ref().unwrap();
    match &reason.category {
        RevertCategory::Other(s) => {
            assert_eq!(s, "BrandNewCategoryEngineShippedThisMorning");
        }
        other => panic!("expected Other, got {other:?}"),
    }
    assert!(!reason.category.is_known());
}

#[tokio::test]
async fn receipt_without_revert_reason_field_deserialises_with_none() {
    // Pre-#349 engines (and post-#349 success receipts) omit the
    // field. #[serde(default)] makes the SDK deserialise it as None
    // without erroring.
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": "0xab".repeat(32),
            "wave_id": "0x5", "tx_index": "0x0",
            "status": "reverted", "gas_used": "0x100",
            "fee_paid": "0x100", "return_data": "0x", "events": []
        })))
        .mount(&server)
        .await;
    let r = provider
        .get_transaction_receipt(&TxHash::new([0xAB; 32]))
        .await
        .unwrap()
        .unwrap();
    assert!(r.revert_reason.is_none());
    assert!(!r.is_engine_validation_revert());
}

// ── send_raw_transaction ────────────────────────────────────────

#[tokio::test]
async fn send_raw_transaction_round_trips() {
    use pyde_rust_sdk::{tx::TxBuilder, wallet::Wallet};

    let (provider, server) = provider_with_server().await;
    let wallet = Wallet::generate().unwrap();
    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .transfer(Address::new([0x99; 32]), 1_000)
        .build()
        .unwrap();
    wallet.sign_tx(&mut tx).await.unwrap();
    let expected = pyde_rust_sdk::tx::tx_hash(&tx);

    let expected_hex = format!("0x{}", hex::encode(expected.as_bytes()));
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok_response(json!(expected_hex)))
        .mount(&server)
        .await;
    let returned = provider.send_raw_transaction(&tx).await.unwrap();
    assert_eq!(returned, expected);
}

// ── get_wave_head + get_fee_data (engine #333 / #337) ───────────

#[tokio::test]
async fn get_wave_head_sends_empty_params() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getWave"))
        .and(body_partial_json(json!({ "params": [] })))
        .respond_with(ok_response(json!({
            "wave_id": "0x42",
            "state_root": "0xab"
        })))
        .mount(&server)
        .await;
    let v = provider.get_wave_head().await.unwrap().unwrap();
    assert_eq!(v["wave_id"], "0x42");
}

#[tokio::test]
async fn get_wave_head_handles_null() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getWave"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    assert!(provider.get_wave_head().await.unwrap().is_none());
}

#[tokio::test]
async fn get_fee_data_decodes_full_shape() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getFeeData"))
        .respond_with(ok_response(json!({
            "base_fee": "0x174876e800",
            "suggested_tip": "0x0",
            "wave_id": "0x2a",
            "recent_waves": [
                {
                    "wave_id": "0x29",
                    "gas_used": "0x5208",
                    "gas_limit": "0x2faf080",
                    "utilisation": "0.0001"
                },
                {
                    "wave_id": "0x28",
                    "gas_used": "0x186a0",
                    "gas_limit": "0x2faf080",
                    "utilisation": "0.0003"
                }
            ]
        })))
        .mount(&server)
        .await;
    let fd = provider.get_fee_data().await.unwrap();
    assert_eq!(fd.base_fee, 0x174876e800u128);
    assert_eq!(fd.suggested_tip, 0);
    assert_eq!(fd.wave_id, 42);
    assert_eq!(fd.recent_waves.len(), 2);
    assert_eq!(fd.recent_waves[0].wave_id, 0x29);
    assert_eq!(fd.recent_waves[0].gas_used, 0x5208);
    assert_eq!(fd.recent_waves[0].gas_limit, 0x2faf080);
    assert!((fd.recent_waves[0].utilisation - 0.0001).abs() < 1e-9);
}

#[tokio::test]
async fn get_fee_data_rejects_missing_base_fee() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getFeeData"))
        .respond_with(ok_response(json!({
            "suggested_tip": "0x0",
            "wave_id": "0x0",
            "recent_waves": []
        })))
        .mount(&server)
        .await;
    let err = provider.get_fee_data().await.unwrap_err();
    assert!(format!("{err}").contains("missing base_fee"));
}

// ── Errors ──────────────────────────────────────────────────────

#[tokio::test]
async fn rpc_error_surfaces() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32603, "message": "internal error" }
        })))
        .mount(&server)
        .await;
    let err = provider.chain_id().await.unwrap_err();
    assert!(matches!(err, pyde_rust_sdk::SdkError::Rpc(_)));
}

#[tokio::test]
async fn http_5xx_surfaces() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server boom"))
        .mount(&server)
        .await;
    let err = provider.chain_id().await.unwrap_err();
    assert!(matches!(err, pyde_rust_sdk::SdkError::Rpc(_)));
}

// ── Logs ────────────────────────────────────────────────────────

#[tokio::test]
async fn get_logs_decodes_page() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getLogs"))
        .respond_with(ok_response(json!({
            "entries": [{
                "wave_id": "0x1",
                "tx_index": "0x0",
                "event_index": "0x0",
                "contract_addr": format!("0x{}", "ab".repeat(32)),
                "topics": [format!("0x{}", "11".repeat(32))],
                "data": "0xbeef"
            }],
            "next_cursor": null
        })))
        .mount(&server)
        .await;
    let filter = LogFilter::default();
    let page = provider.get_logs(&filter).await.unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn get_events_decodes_array() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getEvents"))
        .respond_with(ok_response(json!([
            {
                "wave_id": "0x1",
                "tx_index": "0x0",
                "event_index": "0x0",
                "contract_addr": format!("0x{}", "ab".repeat(32)),
                "topics": [],
                "data": "0x"
            }
        ])))
        .mount(&server)
        .await;
    let events = provider
        .get_events(&EventFilter {
            from_wave: Some("0x0".into()),
            to_wave: None,
            contract: None,
        })
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
}

// ── PendingTx ───────────────────────────────────────────────────

#[tokio::test]
async fn pending_tx_waits_for_receipt() {
    use std::time::Duration;

    let (provider, server) = provider_with_server().await;
    let hash = TxHash::new([0xCD; 32]);

    // First two polls return null (mempool); third returns the
    // receipt. Mount the null stub first with up_to_n_times(2) so
    // the success stub fires last. PendingTx polls
    // `pyde_getTransactionReceipt` (hot state map), not
    // `pyde_getReceipt` (consensus archive).
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(Value::Null))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(json!({
            "tx_hash": format!("0x{}", hex::encode(hash.as_bytes())),
            "wave_id": "0x1",
            "tx_index": "0x0",
            "status": "success",
            "gas_used": "0x5208",
            "fee_paid": "0x1",
            "return_data": "0x",
            "events": []
        })))
        .mount(&server)
        .await;
    let dyn_provider: Arc<dyn pyde_rust_sdk::Provider> = provider.clone();
    let pending = pyde_rust_sdk::PendingTx::new(hash, dyn_provider)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(2));
    let receipt = pending.wait_for_receipt().await.unwrap();
    assert!(receipt.is_success());
}

#[tokio::test]
async fn pending_tx_times_out() {
    use std::time::Duration;

    let (provider, server) = provider_with_server().await;
    let hash = TxHash::new([0xEF; 32]);
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    let dyn_provider: Arc<dyn pyde_rust_sdk::Provider> = provider.clone();
    let pending = pyde_rust_sdk::PendingTx::new(hash, dyn_provider)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_millis(50));
    let err = pending.wait_for_receipt().await.unwrap_err();
    assert!(matches!(err, pyde_rust_sdk::SdkError::Timeout(_)));
}

// ── Coverage gap: 8 RPC methods the audit flagged as untested. ─

#[tokio::test]
async fn get_metrics_returns_value() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getMetrics"))
        .respond_with(ok_response(json!({
            "waves_committed_total": 42,
            "mempool_txs_received_total": 1000
        })))
        .mount(&server)
        .await;
    let v = provider.get_metrics().await.unwrap();
    assert_eq!(v["waves_committed_total"], 42);
}

#[tokio::test]
async fn call_decodes_hex_return() {
    use pyde_rust_sdk::CallRequest;
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .respond_with(ok_response(json!("0xdeadbeef")))
        .mount(&server)
        .await;
    let req = CallRequest {
        to: format!("0x{}", "11".repeat(32)),
        data: "0x".into(),
        from: None,
        value: None,
        gas: None,
    };
    let out = provider.call(&req).await.unwrap();
    assert_eq!(out, vec![0xDE, 0xAD, 0xBE, 0xEF]);
}

#[tokio::test]
async fn simulate_transaction_decodes() {
    use pyde_rust_sdk::TxBuilder;
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_simulateTransaction"))
        .respond_with(ok_response(json!({
            "receipt": {
                "status": "Success",
                "gas_used": "0x5208",
                "fee_paid": "0x1",
                "return_data": "0x"
            },
            "access_list": {
                "reads": [{ "slot": "0xaa", "observed_version": null }],
                "writes": ["0xbb"]
            }
        })))
        .mount(&server)
        .await;
    let tx = TxBuilder::new()
        .from(Address::new([0x42; 32]))
        .build()
        .unwrap();
    let sim = provider.simulate_transaction(&tx).await.unwrap();
    let receipt = sim.receipt.unwrap();
    assert_eq!(receipt.status, "Success");
    assert_eq!(sim.access_list.reads.len(), 1);
    assert_eq!(sim.access_list.writes.len(), 1);
}

#[tokio::test]
async fn get_tx_decodes_some_and_none() {
    let (provider, server) = provider_with_server().await;
    let hash = TxHash::new([0xAA; 32]);

    // Stub null first.
    Mock::given(method("POST"))
        .and(match_method("pyde_getTx"))
        .respond_with(ok_response(Value::Null))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert!(provider.get_tx(&hash).await.unwrap().is_none());
}

#[tokio::test]
async fn get_wave_decodes_some_and_none() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getWave"))
        .respond_with(ok_response(Value::Null))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert!(provider.get_wave(5).await.unwrap().is_none());

    Mock::given(method("POST"))
        .and(match_method("pyde_getWave"))
        .respond_with(ok_response(json!({"wave_id": "0x5"})))
        .mount(&server)
        .await;
    let v = provider.get_wave(5).await.unwrap();
    assert!(v.is_some());
}

#[tokio::test]
async fn get_validator_decodes() {
    let (provider, server) = provider_with_server().await;
    let addr = Address::new([0x77; 32]);
    Mock::given(method("POST"))
        .and(match_method("pyde_getValidator"))
        .respond_with(ok_response(Value::Null))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    assert!(provider.get_validator(&addr).await.unwrap().is_none());

    Mock::given(method("POST"))
        .and(match_method("pyde_getValidator"))
        .respond_with(ok_response(json!({
            "address": addr.to_hex(),
            "status": "Active",
            "stake": "0x1234"
        })))
        .mount(&server)
        .await;
    let v = provider.get_validator(&addr).await.unwrap().unwrap();
    assert_eq!(v["status"], "Active");
}

#[tokio::test]
async fn get_operator_validators_decodes() {
    let (provider, server) = provider_with_server().await;
    let operator = Address::new([0x88; 32]);
    Mock::given(method("POST"))
        .and(match_method("pyde_getOperatorValidators"))
        .respond_with(ok_response(json!([
            { "stake": "0x100" },
            { "stake": "0x200" }
        ])))
        .mount(&server)
        .await;
    let validators = provider.get_operator_validators(&operator).await.unwrap();
    assert_eq!(validators.len(), 2);
}

#[tokio::test]
async fn get_snapshot_returns_value() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getSnapshot"))
        .respond_with(ok_response(json!({
            "wave_id": 100,
            "state_root": "0xabcd"
        })))
        .mount(&server)
        .await;
    let v = provider.get_snapshot().await.unwrap();
    assert_eq!(v["wave_id"], 100);
}

#[tokio::test]
async fn get_snapshot_manifest_returns_value() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getSnapshotManifest"))
        .respond_with(ok_response(json!({
            "wave_id": 100,
            "chunk_count": 4
        })))
        .mount(&server)
        .await;
    let v = provider.get_snapshot_manifest().await.unwrap();
    assert_eq!(v["chunk_count"], 4);
}

// ── T27: encrypted-mempool + finality + threshold-pk ─────────────

#[tokio::test]
async fn get_threshold_public_key_decodes_engine_shape() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getThresholdPublicKey"))
        .respond_with(ok_response(json!({
            "epoch": "0x0",
            "scheme": "mock",
            "public_key": "0xdeadbeef"
        })))
        .mount(&server)
        .await;
    let pk = provider.get_threshold_public_key().await.unwrap().unwrap();
    assert_eq!(pk.epoch, "0x0");
    assert_eq!(pk.scheme, "mock");
    assert_eq!(pk.public_key, "0xdeadbeef");
}

#[tokio::test]
async fn get_threshold_public_key_handles_null() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getThresholdPublicKey"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    assert!(provider.get_threshold_public_key().await.unwrap().is_none());
}

#[tokio::test]
async fn send_raw_encrypted_transaction_decodes_tx_hash() {
    let (provider, server) = provider_with_server().await;
    let expected = format!("0x{}", "cc".repeat(32));
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawEncryptedTransaction"))
        .respond_with(ok_response(json!(expected)))
        .mount(&server)
        .await;
    let h = provider
        .send_raw_encrypted_transaction("0xaabbccddeeff")
        .await
        .unwrap();
    assert_eq!(h.as_bytes(), &[0xCC; 32]);
}

#[tokio::test]
async fn send_raw_encrypted_transaction_accepts_bare_hex() {
    // No 0x prefix on the envelope arg — handler should add it.
    let (provider, server) = provider_with_server().await;
    let expected = format!("0x{}", "dd".repeat(32));
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawEncryptedTransaction"))
        .respond_with(ok_response(json!(expected)))
        .mount(&server)
        .await;
    let h = provider
        .send_raw_encrypted_transaction("aabbccddeeff")
        .await
        .unwrap();
    assert_eq!(h.as_bytes(), &[0xDD; 32]);
}

#[tokio::test]
async fn get_hard_finality_cert_returns_value() {
    let (provider, server) = provider_with_server().await;
    let anchor_hash: Vec<u8> = vec![1; 32];
    let sig_bytes: Vec<u8> = vec![99; 666];
    Mock::given(method("POST"))
        .and(match_method("pyde_getHardFinalityCert"))
        .respond_with(ok_response(json!({
            "commit": { "wave_id": 5, "anchor_hash": anchor_hash },
            "signatures": [[0, sig_bytes]]
        })))
        .mount(&server)
        .await;
    let cert = provider.get_hard_finality_cert(5).await.unwrap().unwrap();
    assert_eq!(cert["commit"]["wave_id"], 5);
    assert!(cert["signatures"].is_array());
}

#[tokio::test]
async fn get_hard_finality_cert_handles_null() {
    let (provider, server) = provider_with_server().await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getHardFinalityCert"))
        .respond_with(ok_response(Value::Null))
        .mount(&server)
        .await;
    assert!(provider
        .get_hard_finality_cert(99999)
        .await
        .unwrap()
        .is_none());
}
