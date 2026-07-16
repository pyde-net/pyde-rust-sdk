//! Coverage closure for the `Contract` runtime + event-decode +
//! filter-construction paths the audit flagged as untested.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use pyde_rust_sdk::abi;
use pyde_rust_sdk::contract::{event_signature_topic, Contract, Value};
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::{
    ContractAbi, ContractType, Event, EventAbi, EventFilter, FunctionAbi, FunctionAttrs, LogFilter,
    ParamAbi, ParamType, StateSchema,
};
use pyde_rust_sdk::{Address, Provider};
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

fn build_abi(name: &str, events: Vec<EventAbi>) -> ContractAbi {
    ContractAbi {
        pyde_abi_version: ContractAbi::V1_2,
        contract_type: ContractType::Contract,
        functions: vec![FunctionAbi {
            selector: [0, 0, 0, 0],
            name: "noop".into(),
            attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY),
            params: vec![],
            returns: None,
        }],
        state_schema_hash: [0; 32],
        constructor_index: None,
        fallback_index: None,
        receive_index: None,
        name: name.into(),
        version: "0.1.0".into(),
        events,
        parachain_imports: vec![],
        state_schema: StateSchema::empty(),
        types: vec![],
    }
}

// ── event_signature_topic — pin against a hand-computed Blake3 vector

#[test]
fn event_signature_topic_matches_canonical_blake3() {
    let ev = EventAbi {
        name: "Transfer".into(),
        params: vec![
            ParamAbi {
                name: "from".into(),
                ty: ParamType::Address,
            },
            ParamAbi {
                name: "to".into(),
                ty: ParamType::Address,
            },
            ParamAbi {
                name: "amount".into(),
                ty: ParamType::U128,
            },
        ],
        indexed_mask: 0b011,
    };
    let topic = event_signature_topic(&ev);
    let expected: [u8; 32] = *blake3::hash(b"Transfer(address,address,uint128)").as_bytes();
    assert_eq!(topic, expected);
}

#[test]
fn event_signature_topic_changes_with_name() {
    let mut ev = EventAbi {
        name: "Transfer".into(),
        params: vec![],
        indexed_mask: 0,
    };
    let topic_a = event_signature_topic(&ev);
    ev.name = "Approval".into();
    let topic_b = event_signature_topic(&ev);
    assert_ne!(topic_a, topic_b);
}

#[test]
fn event_signature_topic_changes_with_params() {
    let mut ev = EventAbi {
        name: "X".into(),
        params: vec![ParamAbi {
            name: "a".into(),
            ty: ParamType::U64,
        }],
        indexed_mask: 0,
    };
    let topic_a = event_signature_topic(&ev);
    ev.params[0].ty = ParamType::U128;
    let topic_b = event_signature_topic(&ev);
    assert_ne!(topic_a, topic_b);
}

// ── Contract::decode_event — happy + malformed paths

#[tokio::test]
async fn decode_event_unpacks_topics_and_data() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x11; 32]);

    let ev_abi = EventAbi {
        name: "Transfer".into(),
        params: vec![
            ParamAbi {
                name: "from".into(),
                ty: ParamType::Address,
            },
            ParamAbi {
                name: "to".into(),
                ty: ParamType::Address,
            },
            ParamAbi {
                name: "amount".into(),
                ty: ParamType::U128,
            },
        ],
        indexed_mask: 0b011, // from + to indexed; amount not
    };
    let topic0 = event_signature_topic(&ev_abi);
    let contract = Contract::new(address, build_abi("token", vec![ev_abi]), provider);

    let from_topic = [0xAA; 32];
    let to_topic = [0xBB; 32];

    // Non-indexed param `amount: u128` goes in `data`.
    let amount: u128 = 1234;
    let data_hex = format!("0x{}", hex::encode(amount.to_le_bytes()));

    let log = Event {
        wave_id: "0x5".into(),
        tx_index: "0x0".into(),
        event_index: "0x0".into(),
        contract_addr: address.to_hex(),
        topics: vec![
            format!("0x{}", hex::encode(topic0)),
            format!("0x{}", hex::encode(from_topic)),
            format!("0x{}", hex::encode(to_topic)),
        ],
        data: data_hex,
    };
    let decoded = contract.decode_event(&log).unwrap();
    assert_eq!(decoded.name, "Transfer");
    assert_eq!(decoded.indexed.len(), 2);
    assert_eq!(decoded.indexed[0], from_topic);
    assert_eq!(decoded.indexed[1], to_topic);
    assert_eq!(decoded.data.len(), 1);
    assert_eq!(decoded.data[0], Value::U128(amount));
}

#[tokio::test]
async fn decode_event_rejects_wrong_contract() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));

    let our_address = Address::new([0x11; 32]);
    let other_address = Address::new([0x22; 32]);
    let contract = Contract::new(our_address, build_abi("x", vec![]), provider);

    let log = Event {
        wave_id: "0x0".into(),
        tx_index: "0x0".into(),
        event_index: "0x0".into(),
        contract_addr: other_address.to_hex(),
        topics: vec![format!("0x{}", hex::encode([0u8; 32]))],
        data: "0x".into(),
    };
    let err = contract.decode_event(&log).unwrap_err();
    assert!(
        format!("{err}").to_lowercase().contains("expected"),
        "unexpected err: {err}"
    );
}

#[tokio::test]
async fn decode_event_rejects_missing_topic0() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x11; 32]);
    let contract = Contract::new(address, build_abi("x", vec![]), provider);

    let log = Event {
        wave_id: "0x0".into(),
        tx_index: "0x0".into(),
        event_index: "0x0".into(),
        contract_addr: address.to_hex(),
        topics: vec![],
        data: "0x".into(),
    };
    let err = contract.decode_event(&log).unwrap_err();
    assert!(format!("{err}").contains("topic 0"));
}

#[tokio::test]
async fn decode_event_rejects_unknown_signature() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x11; 32]);
    let contract = Contract::new(address, build_abi("x", vec![]), provider);

    let log = Event {
        wave_id: "0x0".into(),
        tx_index: "0x0".into(),
        event_index: "0x0".into(),
        contract_addr: address.to_hex(),
        topics: vec![format!("0x{}", hex::encode([0xCC; 32]))],
        data: "0x".into(),
    };
    let err = contract.decode_event(&log).unwrap_err();
    assert!(format!("{err}").contains("no event in ABI"));
}

// ── Contract::event_filter / event_filter_for

#[tokio::test]
async fn event_filter_pins_contract_address() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x42; 32]);
    let contract = Contract::new(address, build_abi("x", vec![]), provider);

    let filter = contract.event_filter();
    assert_eq!(filter.contracts.len(), 1);
    assert_eq!(filter.contracts[0], address.to_hex());
    assert!(filter.topics.is_empty());
}

#[tokio::test]
async fn event_filter_for_pre_populates_topic_0() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x42; 32]);
    let ev = EventAbi {
        name: "Mint".into(),
        params: vec![],
        indexed_mask: 0,
    };
    let contract = Contract::new(address, build_abi("x", vec![ev.clone()]), provider);

    let filter = contract.event_filter_for("Mint").unwrap();
    assert_eq!(filter.contracts.len(), 1);
    let topic_pos_0 = filter.topics.first().expect("topic[0]").as_ref().unwrap();
    assert_eq!(topic_pos_0.len(), 1);
    let expected = format!("0x{}", hex::encode(event_signature_topic(&ev)));
    assert_eq!(topic_pos_0[0], expected);
}

#[tokio::test]
async fn event_filter_for_rejects_unknown_event() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));
    let address = Address::new([0x42; 32]);
    let contract = Contract::new(address, build_abi("x", vec![]), provider);
    assert!(contract.event_filter_for("Nope").is_err());
}

// ── Contract::load — name resolution + bytecode fetch

#[tokio::test]
async fn contract_load_unresolved_name_errors() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));

    Mock::given(method("POST"))
        .and(match_method("pyde_resolveName"))
        .respond_with(ok(JsonValue::Null))
        .mount(&server)
        .await;
    let err = Contract::load("does-not-exist", provider)
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("not registered"));
}

#[tokio::test]
async fn contract_load_at_empty_bytecode_errors() {
    let server = MockServer::start().await;
    let transport = HttpTransport::new(server.uri()).unwrap();
    let provider: Arc<dyn Provider> = Arc::new(RootProvider::new(transport));

    Mock::given(method("POST"))
        .and(match_method("pyde_getContractCode"))
        .respond_with(ok(json!("0x")))
        .mount(&server)
        .await;
    let err = Contract::load_at(Address::new([0xAA; 32]), provider)
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("no contract code"));
}

// ── LogFilter + EventFilter serialization shape

#[test]
fn log_filter_serializes_to_snake_case() {
    let f = LogFilter {
        from_wave: Some("0x1".into()),
        to_wave: Some("0xff".into()),
        contracts: vec!["0xaa".into()],
        topics: vec![Some(vec!["0xbb".into()])],
        cursor: None,
        limit: Some(50),
    };
    let json = serde_json::to_string(&f).unwrap();
    assert!(json.contains("from_wave"));
    assert!(json.contains("to_wave"));
    assert!(json.contains("contracts"));
    assert!(json.contains("topics"));
    assert!(json.contains("limit"));
}

#[test]
fn event_filter_serializes_to_camel_case() {
    let f = EventFilter {
        from_wave: Some("0x0".into()),
        to_wave: Some("0x10".into()),
        contract: Some("0xaa".into()),
    };
    let json = serde_json::to_string(&f).unwrap();
    assert!(json.contains("fromWave"));
    assert!(json.contains("toWave"));
    assert!(json.contains("contract"));
}

// ── ABI extract via pre-built bundle

#[test]
fn extract_abi_against_a_handcrafted_wasm() {
    // Build a minimal WASM + Borsh-encoded ContractAbi custom section.
    let abi = build_abi("handcrafted", vec![]);
    let abi_bytes = borsh::to_vec(&abi).unwrap();
    let mut wasm = Vec::new();
    wasm.extend_from_slice(b"\0asm");
    wasm.extend_from_slice(&1u32.to_le_bytes());
    let name = b"pyde.abi";
    let mut payload = Vec::new();
    payload.push(name.len() as u8);
    payload.extend_from_slice(name);
    payload.extend_from_slice(&abi_bytes);
    wasm.push(0x00);
    wasm.push(payload.len() as u8);
    wasm.extend_from_slice(&payload);
    let parsed = abi::extract_abi(&wasm).unwrap();
    assert_eq!(parsed.name, "handcrafted");
}
