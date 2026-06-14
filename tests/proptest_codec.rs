//! Property-based tests for the calldata codec + tx round-trip.
//!
//! Generates random `Value` payloads against every `ParamType`,
//! encodes + decodes, and asserts byte-for-byte recovery. Catches
//! the corner cases hand-written round-trip tests miss — zero-length
//! Vecs, single-element Options, max/min ints, empty strings, etc.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use pyde_rust_sdk::contract::{decode_value, encode_value, Value};
use pyde_rust_sdk::tx::{decode, encode, tx_hash, TxBuilder};
use pyde_rust_sdk::types::{Address, ParamType};
use pyde_rust_sdk::Wallet;

// ── Value strategies ────────────────────────────────────────────

fn arb_address() -> impl Strategy<Value = Address> {
    proptest::array::uniform32(any::<u8>()).prop_map(Address::new)
}

fn arb_simple_value() -> impl Strategy<Value = (ParamType, Value)> {
    prop_oneof![
        any::<u8>().prop_map(|x| (ParamType::U8, Value::U8(x))),
        any::<u16>().prop_map(|x| (ParamType::U16, Value::U16(x))),
        any::<u32>().prop_map(|x| (ParamType::U32, Value::U32(x))),
        any::<u64>().prop_map(|x| (ParamType::U64, Value::U64(x))),
        any::<u128>().prop_map(|x| (ParamType::U128, Value::U128(x))),
        any::<i8>().prop_map(|x| (ParamType::I8, Value::I8(x))),
        any::<i16>().prop_map(|x| (ParamType::I16, Value::I16(x))),
        any::<i32>().prop_map(|x| (ParamType::I32, Value::I32(x))),
        any::<i64>().prop_map(|x| (ParamType::I64, Value::I64(x))),
        any::<i128>().prop_map(|x| (ParamType::I128, Value::I128(x))),
        any::<bool>().prop_map(|x| (ParamType::Bool, Value::Bool(x))),
        arb_address().prop_map(|a| (ParamType::Address, Value::Address(a))),
        prop::collection::vec(any::<u8>(), 0..256)
            .prop_map(|b| (ParamType::Bytes, Value::Bytes(b))),
        "[a-zA-Z0-9 ]{0,128}".prop_map(|s| (ParamType::String, Value::String(s))),
    ]
}

fn arb_fixed_bytes() -> impl Strategy<Value = (ParamType, Value)> {
    (1u32..=64).prop_flat_map(|n| {
        prop::collection::vec(any::<u8>(), n as usize..=n as usize)
            .prop_map(move |b| (ParamType::FixedBytes(n), Value::FixedBytes(b)))
    })
}

fn arb_vec_value() -> impl Strategy<Value = (ParamType, Value)> {
    prop::collection::vec(any::<u64>(), 0..32).prop_map(|items| {
        (
            ParamType::Vec(Box::new(ParamType::U64)),
            Value::Vec(items.into_iter().map(Value::U64).collect()),
        )
    })
}

fn arb_option_value() -> impl Strategy<Value = (ParamType, Value)> {
    prop::option::of(any::<u128>()).prop_map(|opt| {
        (
            ParamType::Option(Box::new(ParamType::U128)),
            Value::Option(opt.map(|x| Box::new(Value::U128(x)))),
        )
    })
}

// ── Properties ─────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn simple_value_round_trips((ty, value) in arb_simple_value()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn fixed_bytes_round_trips((ty, value) in arb_fixed_bytes()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn vec_round_trips((ty, value) in arb_vec_value()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn option_round_trips((ty, value) in arb_option_value()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }
}

// ── Tx round-trip ──────────────────────────────────────────────

fn arb_tx_skeleton() -> impl Strategy<Value = (u64, u64, u128, u64)> {
    (
        1u64..=10_000_000,              // chain_id
        0u64..=u64::MAX,                // nonce
        0u128..=10_000_000_000_000u128, // value (quanta, capped)
        21_000u64..=1_000_000u64,       // gas_limit
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn tx_wire_round_trip_preserves_hash(
        (chain_id, nonce, value, gas_limit) in arb_tx_skeleton(),
        recipient_bytes in proptest::array::uniform32(any::<u8>()),
        data in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        // Use a fresh wallet per case so randomized FALCON sigs vary.
        let wallet = Wallet::generate().unwrap();
        let to = Address::new(recipient_bytes);
        let mut tx = TxBuilder::new()
            .from(wallet.address())
            .chain_id(chain_id)
            .nonce(nonce)
            .to(to)
            .value(value)
            .data(data)
            .gas_limit(gas_limit)
            .build()
            .unwrap();

        // sign synchronously by entering the tokio runtime
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            use pyde_rust_sdk::Signer;
            wallet.sign_tx(&mut tx).await.unwrap();
        });
        let hash_before = tx_hash(&tx);

        let bytes = encode(&tx).unwrap();
        let decoded = decode(&bytes).unwrap();
        let hash_after = tx_hash(&decoded);

        prop_assert_eq!(decoded, tx);
        prop_assert_eq!(hash_before, hash_after);
    }
}

// ── Nested composites — close the proptest gap for arbitrary depth.

fn arb_vec_of_string() -> impl Strategy<Value = (ParamType, Value)> {
    prop::collection::vec("[a-z]{0,32}".prop_map(String::from), 0..16).prop_map(|items| {
        (
            ParamType::Vec(Box::new(ParamType::String)),
            Value::Vec(items.into_iter().map(Value::String).collect()),
        )
    })
}

fn arb_map_string_to_u128() -> impl Strategy<Value = (ParamType, Value)> {
    prop::collection::vec(("[a-z]{0,16}", any::<u128>()), 0..16).prop_map(|pairs| {
        let mut entries: Vec<(Value, Value)> = pairs
            .into_iter()
            .map(|(k, v)| (Value::String(k), Value::U128(v)))
            .collect();
        // Dedupe by key — borsh map encoding is order-preserving but
        // not deduping, and our codec is happy either way; but the
        // hand-written round-trip easier if we keep keys unique.
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.dedup_by(|a, b| a.0 == b.0);
        (
            ParamType::Map {
                key: Box::new(ParamType::String),
                value: Box::new(ParamType::U128),
            },
            Value::Map(entries),
        )
    })
}

fn arb_option_address() -> impl Strategy<Value = (ParamType, Value)> {
    prop::option::of(proptest::array::uniform32(any::<u8>()).prop_map(Address::new)).prop_map(
        |opt| {
            (
                ParamType::Option(Box::new(ParamType::Address)),
                Value::Option(opt.map(|a| Box::new(Value::Address(a)))),
            )
        },
    )
}

fn arb_vec_of_vec_u64() -> impl Strategy<Value = (ParamType, Value)> {
    prop::collection::vec(prop::collection::vec(any::<u64>(), 0..8), 0..8).prop_map(|outer| {
        let mut top = Vec::with_capacity(outer.len());
        for inner in outer {
            top.push(Value::Vec(inner.into_iter().map(Value::U64).collect()));
        }
        (
            ParamType::Vec(Box::new(ParamType::Vec(Box::new(ParamType::U64)))),
            Value::Vec(top),
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn vec_string_round_trips((ty, value) in arb_vec_of_string()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn map_string_u128_round_trips((ty, value) in arb_map_string_to_u128()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn option_address_round_trips((ty, value) in arb_option_address()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }

    #[test]
    fn nested_vec_of_vec_u64_round_trips((ty, value) in arb_vec_of_vec_u64()) {
        let bytes = encode_value(&ty, &value).unwrap();
        let decoded = decode_value(&ty, &bytes).unwrap();
        prop_assert_eq!(decoded, value);
    }
}
