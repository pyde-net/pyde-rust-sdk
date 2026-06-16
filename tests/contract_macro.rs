//! Integration test for the `pyde_abi!` proc-macro.
//!
//! Generates a typed `Counter` wrapper from `tests/fixtures/counter_abi.json`,
//! then exercises view + non-view calls against a wiremock-hosted
//! JSON-RPC server. Confirms the macro produces compilable code and
//! wires through the runtime correctly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pyde_rust_sdk::contract::Value;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{ParamType, ReceiptStatus};
use pyde_rust_sdk::{Address, Provider, Wallet};
use serde_json::{json, Value as JsonValue};
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

pyde_rust_sdk::pyde_abi!(Counter, "tests/fixtures/counter_abi.json");

fn match_method(method_name: &str) -> wiremock::matchers::BodyPartialJsonMatcher {
    body_partial_json(json!({ "method": method_name }))
}

fn ok_response(result: JsonValue) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    }))
}

async fn provider() -> (Arc<RootProvider<HttpTransport>>, MockServer) {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    (Arc::new(RootProvider::new(transport)), server)
}

#[test]
fn macro_publishes_constants_and_struct() {
    assert_eq!(Counter::NAME, "Counter");
    assert_eq!(Counter::VERSION, "0.1.0");
    assert!(Counter::EVENTS.is_empty());
}

#[tokio::test]
async fn view_call_decodes_typed_return() {
    let (provider, server) = provider().await;
    // pyde_call returns a Borsh-encoded u64 (hex).
    let payload = pyde_rust_sdk::contract::encode_value(&ParamType::U64, &Value::U64(42)).unwrap();
    let hex_payload = format!("0x{}", hex::encode(&payload));
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .respond_with(ok_response(json!(hex_payload)))
        .mount(&server)
        .await;
    let counter = Counter::new(Address::new([0x11; 32]), provider as Arc<dyn Provider>);
    let count = counter.get_count().await.unwrap();
    assert_eq!(count, 42);
}

#[tokio::test]
async fn view_call_decodes_address_return() {
    let (provider, server) = provider().await;
    let owner_addr = Address::new([0xAB; 32]);
    let payload =
        pyde_rust_sdk::contract::encode_value(&ParamType::Address, &Value::Address(owner_addr))
            .unwrap();
    let hex_payload = format!("0x{}", hex::encode(&payload));
    Mock::given(method("POST"))
        .and(match_method("pyde_call"))
        .respond_with(ok_response(json!(hex_payload)))
        .mount(&server)
        .await;
    let counter = Counter::new(Address::new([0x11; 32]), provider as Arc<dyn Provider>);
    let owner = counter.owner().await.unwrap();
    assert_eq!(owner, owner_addr);
}

#[tokio::test]
async fn non_view_call_returns_pending_tx() {
    let (provider, server) = provider().await;
    let wallet = Wallet::generate().unwrap();

    // Mock the chain_id + nonce calls the send path makes.
    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok_response(json!("0x1")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getNonce"))
        .respond_with(ok_response(json!("0x0")))
        .mount(&server)
        .await;
    // sendRawTransaction returns a tx hash; mock with a fixed value.
    let fake_hash = format!("0x{}", "cd".repeat(32));
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok_response(json!(fake_hash.clone())))
        .mount(&server)
        .await;

    let counter = Counter::new(Address::new([0x22; 32]), provider as Arc<dyn Provider>);
    let pending = counter.add(&wallet, 5, 500_000, 0).await.unwrap();
    assert_eq!(
        format!("0x{}", hex::encode(pending.hash().as_bytes())),
        fake_hash
    );
}

#[tokio::test]
async fn non_view_with_no_args_works() {
    let (provider, server) = provider().await;
    let wallet = Wallet::generate().unwrap();

    Mock::given(method("POST"))
        .and(match_method("pyde_chainId"))
        .respond_with(ok_response(json!("0x1")))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(match_method("pyde_getNonce"))
        .respond_with(ok_response(json!("0x3")))
        .mount(&server)
        .await;
    let fake_hash = format!("0x{}", "11".repeat(32));
    Mock::given(method("POST"))
        .and(match_method("pyde_sendRawTransaction"))
        .respond_with(ok_response(json!(fake_hash.clone())))
        .mount(&server)
        .await;

    let counter = Counter::new(Address::new([0x33; 32]), provider as Arc<dyn Provider>);
    let pending = counter.increment(&wallet, 250_000, 0).await.unwrap();
    let _ = wallet.address(); // silence unused-wallet warning if the
                              // assertion below is removed in future
    assert_eq!(
        format!("0x{}", hex::encode(pending.hash().as_bytes())),
        fake_hash
    );
    // Receipt-poll path matches the basic provider tests already.
    let _ = ReceiptStatus::Success; // exercise the re-export
}
