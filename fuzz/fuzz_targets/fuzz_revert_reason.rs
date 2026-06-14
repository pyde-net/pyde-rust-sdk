//! Feed random bytes into the revert-reason decoder. The decoder
//! must always return `Some(s)` or `None`, never panic, even when
//! the input would be valid UTF-8 of bizarre length or shape.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::SdkError;

fuzz_target!(|data: &[u8]| {
    let err = SdkError::Reverted {
        gas_used: 1,
        data: data.to_vec(),
    };
    let _ = err.revert_reason();
    let _ = err.error_code();
});
