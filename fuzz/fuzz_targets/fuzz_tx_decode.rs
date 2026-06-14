//! Feed random bytes into the `Tx` Borsh decoder. The decoder must
//! reject malformed wire bytes without panicking — borsh's
//! `from_slice` returns `Err`, and the SDK wraps that in an
//! `SdkError`.

#![no_main]

use libfuzzer_sys::fuzz_target;
use pyde_rust_sdk::tx::decode;

fuzz_target!(|data: &[u8]| {
    let _ = decode(data);
});
