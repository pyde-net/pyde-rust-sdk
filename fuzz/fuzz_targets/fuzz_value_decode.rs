//! Feed random bytes into `decode_value` against a rotating set of
//! `ParamType`s. The decoder must reject malformed input cleanly
//! (return `Err`) without panicking, allocating unbounded memory,
//! or leaking secrets.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::contract::decode_value;
use pyde_rust_sdk::types::ParamType;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let types: [ParamType; 12] = [
        ParamType::U8,
        ParamType::U64,
        ParamType::U128,
        ParamType::I64,
        ParamType::Bool,
        ParamType::Address,
        ParamType::Bytes,
        ParamType::String,
        ParamType::FixedBytes(32),
        ParamType::Vec(Box::new(ParamType::U64)),
        ParamType::Map {
            key: Box::new(ParamType::String),
            value: Box::new(ParamType::U128),
        },
        ParamType::Option(Box::new(ParamType::Address)),
    ];
    // Pick a type from the first byte; rest is the payload.
    let idx = (data[0] as usize) % types.len();
    let _ = decode_value(&types[idx], &data[1..]);
});
