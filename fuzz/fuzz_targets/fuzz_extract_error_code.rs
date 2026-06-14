//! Feed random UTF-8 strings into `error_code()` to ensure the
//! integer-parsing fallback handles every edge case (overflow,
//! malformed negatives, embedded numbers) without panicking.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::SdkError;

fuzz_target!(|data: &[u8]| {
    // The error_code() entry point lives on SdkError::Reverted —
    // run it through that surface so we exercise the public path.
    let err = SdkError::Reverted {
        gas_used: 1,
        data: data.to_vec(),
    };
    let _ = err.error_code();
});
