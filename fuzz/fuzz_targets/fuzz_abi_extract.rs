//! Feed random bytes into the WASM `pyde.abi` extractor. The
//! parser walks `wasmparser` payloads and Borsh-decodes the custom
//! section; both must reject malformed input cleanly.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::abi;

fuzz_target!(|data: &[u8]| {
    let _ = abi::extract_abi(data);
});
