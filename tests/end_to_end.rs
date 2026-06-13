//! End-to-end SDK flow — wallet generation → tx build + sign → wire
//! encode → wire decode → hash unchanged → mock-RPC broadcast →
//! PendingTx receipt.
//!
//! Exercises every layer of the SDK in one chain. If any single
//! layer drifts from the engine's wire format, the round-trip
//! comparison here breaks.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::contract::Value;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ParamType, ReceiptStatus};
use pyde_rust_sdk::{tx, Address, Provider, Signer, TxBuilder, Wallet};
use serde_json::{json, Value as JsonValue};
use wiremock::matchers::{body_partial_json, method};
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

#[tokio::test]
async fn tx_round_trip_through_wire_preserves_hash() {
    let wallet = Wallet::generate().unwrap();
    let recipient = Address::new([0xCC; 32]);

    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .chain_id(31337)
        .nonce(0)
        .transfer(recipient, 1_500_000_000)
        .gas_limit(50_000)
        .build()
        .unwrap();
    wallet.sign_tx(&mut tx).await.unwrap();
    let hash_before = tx::tx_hash(&tx);

    // Encode → decode (what `pyde_sendRawTransaction` consumes).
    let bytes = tx::encode(&tx).unwrap();
    let decoded = tx::decode(&bytes).unwrap();
    let hash_after = tx::tx_hash(&decoded);

    assert_eq!(decoded, tx, "tx round-trip mismatch");
    assert_eq!(
        hash_before, hash_after,
        "tx_hash changed across encode/decode"
    );
}

#[tokio::test]
async fn full_pipeline_end_to_end_against_mock_node() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider = Arc::new(RootProvider::new(transport));

    // chain_id + nonce + send_raw_transaction + get_receipt.
    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok(json!("0x7A69"))) // 31337
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionCount"))
        .respond_with(ok(json!("0x0")))
        .mount(&server)
        .await;

    // Build the same tx the client will build so we can pin the
    // hash the mock returns.
    let wallet = Wallet::generate().unwrap();
    let recipient = Address::new([0xCC; 32]);
    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .chain_id(31337)
        .nonce(0)
        .transfer(recipient, 1_500_000_000)
        .gas_limit(50_000)
        .build()
        .unwrap();
    wallet.sign_tx(&mut tx).await.unwrap();
    let expected_hash = tx::tx_hash(&tx);
    let expected_hash_hex = format!("0x{}", hex::encode(expected_hash.as_bytes()));

    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok(json!(expected_hash_hex.clone())))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getReceipt"))
        .respond_with(ok(json!({
            "tx_hash": expected_hash_hex,
            "wave_id": "0x10",
            "tx_index": "0x2",
            "status": "success",
            "gas_used": "0xC350",
            "fee_paid": "0x1234",
            "return_data": "0x",
            "events": []
        })))
        .mount(&server)
        .await;

    // Driver path: chain_id + nonce + sign + broadcast + wait.
    let chain_id = provider.chain_id().await.unwrap();
    let nonce = provider.get_nonce(&wallet.address()).await.unwrap();
    assert_eq!(chain_id, 31337);
    assert_eq!(nonce, 0);

    let pending = provider.send_transaction(&tx).await.unwrap();
    assert_eq!(pending.hash(), expected_hash);

    let receipt = pending
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(2))
        .wait_for_receipt()
        .await
        .unwrap();
    assert!(matches!(receipt.status, ReceiptStatus::Success));
    assert_eq!(receipt.wave_id_u64(), 0x10);
    assert_eq!(receipt.tx_index_u32(), 2);
    assert_eq!(receipt.gas(), 0xC350);
}

#[test]
fn value_codec_preserves_borsh_for_all_primitives() {
    use pyde_rust_sdk::contract::{decode_value, encode_value};

    let cases: Vec<(ParamType, Value)> = vec![
        (ParamType::U8, Value::U8(255)),
        (ParamType::U64, Value::U64(u64::MAX)),
        (ParamType::U128, Value::U128(u128::MAX)),
        (ParamType::I128, Value::I128(i128::MIN)),
        (ParamType::Bool, Value::Bool(true)),
        (ParamType::Bool, Value::Bool(false)),
        (ParamType::Address, Value::Address(Address::new([0xDE; 32]))),
        (ParamType::Bytes, Value::Bytes(vec![0; 1024])),
        (ParamType::String, Value::String("unicode 🎩".into())),
        (ParamType::FixedBytes(16), Value::FixedBytes(vec![0xCC; 16])),
        (
            ParamType::Vec(Box::new(ParamType::U64)),
            Value::Vec((0..10).map(Value::U64).collect()),
        ),
    ];

    for (ty, value) in cases {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        assert_eq!(decoded, value, "round-trip mismatch for {ty:?}");
    }
}
