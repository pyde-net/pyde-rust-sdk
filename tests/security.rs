//! Security-regression tests for the v1 hardening pass.
//!
//! Every test pins a specific attack-surface fix. Adding a regression
//! here documents the assumption + makes it impossible to silently
//! remove the guard.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use pyde_rust_sdk::contract::{decode_value, encode_value, Value, MAX_DECODE_ELEMENTS};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ParamType, TxHash};
use pyde_rust_sdk::{PendingTx, Provider, SdkError};
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

// ── C1: Codec DoS via malicious length prefix ───────────────────

#[test]
fn codec_rejects_huge_vec_length_prefix() {
    // A malicious upstream might send a Vec<u8> field with a u32::MAX
    // length prefix and no payload. Before the guard, decode_value
    // allocated `count` items unbounded → instant OOM.
    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let err = decode_value(&ParamType::Vec(Box::new(ParamType::U64)), &payload).unwrap_err();
    assert!(matches!(err, SdkError::InvalidResponse(ref m) if m.contains("MAX_DECODE_ELEMENTS")));
}

#[test]
fn codec_rejects_huge_map_length_prefix() {
    let mut payload = Vec::new();
    payload.extend_from_slice(&u32::MAX.to_le_bytes());
    let err = decode_value(
        &ParamType::Map {
            key: Box::new(ParamType::U64),
            value: Box::new(ParamType::U64),
        },
        &payload,
    )
    .unwrap_err();
    assert!(matches!(err, SdkError::InvalidResponse(ref m) if m.contains("MAX_DECODE_ELEMENTS")));
}

#[test]
fn codec_accepts_count_at_the_limit() {
    // Sanity check: a legitimate `MAX_DECODE_ELEMENTS`-count Vec
    // decodes fine. Use empty elements (Bool) to keep memory cheap.
    let count = MAX_DECODE_ELEMENTS;
    let mut payload = Vec::with_capacity(4 + count);
    payload.extend_from_slice(&(count as u32).to_le_bytes());
    payload.extend(std::iter::repeat_n(0u8, count));
    let value = decode_value(&ParamType::Vec(Box::new(ParamType::Bool)), &payload).unwrap();
    if let Value::Vec(v) = value {
        assert_eq!(v.len(), count);
    } else {
        panic!("expected Vec");
    }
}

// ── C3: PendingTx hash-mismatch detection ───────────────────────

#[tokio::test]
async fn pending_tx_rejects_mismatched_tx_hash() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider = Arc::new(RootProvider::new(transport));

    let polled_hash = TxHash::new([0xAA; 32]);
    let wrong_hash_hex = format!("0x{}", hex::encode([0xBB; 32]));

    // Node serves a receipt for a DIFFERENT tx than we asked about.
    // PendingTx must reject rather than report success.
    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok(json!({
            "tx_hash": wrong_hash_hex,
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

    let dyn_provider: Arc<dyn Provider> = provider.clone();
    let pending = PendingTx::new(polled_hash, dyn_provider)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(1));
    let err = pending.wait_for_receipt().await.unwrap_err();
    assert!(
        matches!(err, SdkError::InvalidResponse(ref m) if m.contains("doesn't match")),
        "expected InvalidResponse, got {err:?}"
    );
}

#[tokio::test]
async fn pending_tx_accepts_case_mismatched_tx_hash() {
    // Upper-case 0xAA… vs lower-case 0xaa…. Different ASCII bytes but
    // semantically the same hash. PendingTx must accept this.
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider = Arc::new(RootProvider::new(transport));

    let polled_hash = TxHash::new([0xAA; 32]);
    let upper_hex = format!("0x{}", hex::encode([0xAA; 32]).to_uppercase());

    Mock::given(method("POST"))
        .and(match_method("pyde_getTransactionReceipt"))
        .respond_with(ok(json!({
            "tx_hash": upper_hex,
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

    let dyn_provider: Arc<dyn Provider> = provider.clone();
    let pending = PendingTx::new(polled_hash, dyn_provider)
        .with_poll_interval(Duration::from_millis(10))
        .with_timeout(Duration::from_secs(1));
    let receipt = pending.wait_for_receipt().await.unwrap();
    assert!(receipt.is_success());
}

// ── Codec round-trip — pinning the guard didn't break legitimate
//    paths.

#[test]
fn codec_legitimate_vec_still_round_trips() {
    let ty = ParamType::Vec(Box::new(ParamType::U128));
    let value = Value::Vec((0..256).map(|n| Value::U128(n as u128)).collect());
    let bytes = encode_value(&ty, &value).unwrap();
    let decoded = decode_value(&ty, &bytes).unwrap();
    assert_eq!(decoded, value);
}

#[test]
fn codec_legitimate_map_still_round_trips() {
    let ty = ParamType::Map {
        key: Box::new(ParamType::String),
        value: Box::new(ParamType::U64),
    };
    let value = Value::Map(vec![
        (Value::String("a".into()), Value::U64(1)),
        (Value::String("b".into()), Value::U64(2)),
        (Value::String("c".into()), Value::U64(3)),
    ]);
    let bytes = encode_value(&ty, &value).unwrap();
    let decoded = decode_value(&ty, &bytes).unwrap();
    assert_eq!(decoded, value);
}
