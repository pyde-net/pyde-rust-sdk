//! Feed random bytes into the JSON-RPC response decoder. The
//! decoder must reject malformed JSON without panicking — serde's
//! parser returns `Err`, and the SDK wraps that in an
//! `SdkError::InvalidResponse`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::provider::JsonRpcResponse;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(text) {
            let _ = resp.into_result();
        }
    }
});
