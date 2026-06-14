# Fuzz targets

Coverage-guided `cargo-fuzz` (libFuzzer) harnesses for the SDK's
attacker-facing deserialization paths.

## Run

`cargo-fuzz` requires a nightly toolchain:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
```

Then from the repo root:

```sh
cargo +nightly fuzz run fuzz_value_decode
cargo +nightly fuzz run fuzz_tx_decode
cargo +nightly fuzz run fuzz_jsonrpc_response
cargo +nightly fuzz run fuzz_revert_reason
cargo +nightly fuzz run fuzz_extract_error_code
cargo +nightly fuzz run fuzz_abi_extract
```

Each target runs until you Ctrl-C or libFuzzer finds a crash. A
crashing input is written to
`fuzz/artifacts/<target>/crash-<hash>`; re-run the target with that
file as a positional arg to reproduce.

## Surfaces covered

| Target | What |
|---|---|
| `fuzz_value_decode` | `Value::decode_value` against 12 rotating `ParamType`s — primitives, composites, address, options |
| `fuzz_tx_decode` | `Tx` Borsh decoder over random bytes |
| `fuzz_jsonrpc_response` | `JsonRpcResponse` decode + `into_result()` |
| `fuzz_revert_reason` | `SdkError::Reverted::revert_reason()` — Borsh string / u64-length / raw UTF-8 paths |
| `fuzz_extract_error_code` | `SdkError::Reverted::error_code()` — named-token scan + negative-int parser |
| `fuzz_abi_extract` | `abi::extract_abi(&wasm)` — wasmparser walk + Borsh decode of the `pyde.abi` custom section |

## Invariants

Every target asserts the same contract: **the parser never
panics on attacker-controlled input.** It can return an error.
It cannot:
- Trigger an `unwrap`/`expect`/`panic!()` in library code.
- Allocate unbounded memory (codec has `MAX_DECODE_ELEMENTS`).
- Read out-of-bounds.
- Hang.

A crash here is a real bug. File an issue; pin the seed in
`tests/security.rs` so the regression doesn't reopen.
