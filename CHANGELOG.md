# Changelog

All notable changes to `pyde-rust-sdk`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning
follows [SemVer](https://semver.org/), with the caveat that everything
pre-1.0 may have breaking changes at any minor bump (see
[docs/12-compatibility.md](docs/12-compatibility.md#msrv--semver-intent)).

## [Unreleased]

### Added
- **Two new `Provider` methods** matching engine #333 / #337:
  - `get_wave_head()` → `pyde_getWave` (no-arg form). Returns the
    latest committed wave in one round-trip; pairs with the
    engine's light-client head query. `None` only on a chain
    that has never committed a wave.
  - `get_fee_data()` → `pyde_getFeeData`. Current base-fee +
    suggested-tip snapshot plus the last 10 committed waves'
    gas utilisation (driving a wallet's gas-price slider or
    network-load chart in one round-trip).
- Reworked `FeeData` struct: dropped the `gas_price` field
  (was an unused alias for `base_fee`); added `wave_id`,
  `suggested_tip` (always `0` in v1, future-proof), and
  `recent_waves: Vec<RecentWaveSummary>`. New
  `RecentWaveSummary { wave_id, gas_used, gas_limit, utilisation }`
  type for per-wave entries.
- **Three new `Provider` methods** matching engine PR-326's RPC
  catalog additions:
  - `send_raw_encrypted_transaction(envelope_hex)` →
    `pyde_sendRawEncryptedTransaction`. Submits a borsh-encoded
    `EncryptedTxEnvelope` for the MEV-protected mempool path.
    Returns the 32-byte Blake3 envelope hash. Engine v1 size
    limits enforced: min 1213 bytes, max 128 KiB.
  - `get_threshold_public_key()` →
    `pyde_getThresholdPublicKey`. Returns the current DKG-epoch
    pubkey wallets encrypt under (`{epoch, scheme, public_key}`).
    Callers should check
    `scheme == "kyber-768-goldilocks"` (the real-crypto path);
    a `"mock"` scheme indicates v1 mock-DKG and encrypted submits
    will sit unprocessed until real-crypto ships.
  - `get_hard_finality_cert(wave_id)` →
    `pyde_getHardFinalityCert`. Returns the wave's hard-finality
    cert (bundle of ≥85 validator signatures) for light-client +
    cross-chain-bridge use. `wave_id` is sent as a bare JSON
    number on the wire (engine quirk shared with `get_wave`).
- New types `ThresholdPublicKey` `{epoch, scheme, public_key}` and
  `EncryptedTxEnvelope` `{version, ciphertext}` — the latter mirrors
  `pyde_engine_types::EncryptedTxEnvelope` byte-for-byte so callers
  don't need a direct dep on the engine's types crate. Carries
  `VERSION = 1`, `MIN_CIPHERTEXT_LEN = 1213`,
  `MAX_CIPHERTEXT_LEN = MAX_TX_SIZE`, and an `envelope_hash()`
  matching the engine's `Blake3(version || len_le || ciphertext)`.
- `examples/encrypted_transfer.rs` — end-to-end round-trip of the
  MEV-protected mempool path. Fetches the threshold pubkey,
  encrypts a signed Tx via `pyde_crypto::threshold::threshold_encrypt`
  + `.to_wire_bytes()`, wraps in `EncryptedTxEnvelope`, submits,
  and waits for the plaintext receipt under the inner tx hash.
  Live-verified on `otigen devnet` post engine #334 / #335 /
  #336 + pyde-crypto-wasm #5: plaintext receipt lands in ~12 waves
  on the single-validator devnet.
- `RetryConfig` for `HttpTransport` — exponential backoff with jitter
  on transient failures (connection refused, TCP/TLS errors, HTTP 5xx,
  HTTP 429). Default: 3 retries, 100 ms base, 5 s cap, ±25% jitter.
  Disable via `HttpTransport::new(url)?.with_retry_config(RetryConfig::no_retry())`.
- HTTP 429 (rate-limited) is now classified as transient + retried.
  The engine's rate limiter ships its retry-after hint in the
  response body (`"Wait for Ns"`); `HttpTransport` parses it and
  uses it as the next sleep duration (capped at 60 s to bound
  worst-case latency from a hostile server) instead of the default
  exponential backoff. Before T30, 429 surfaced to the caller
  immediately and broke any test/dapp doing a burst of RPC calls
  against the default 100-rps engine limiter.
- Memory-safety pin test (`wallet_drop_wipes_secret`) verifying the
  `Wallet` → `LocalSigner` → `FalconSecret` Drop chain stays intact.
  The chain wipes the 1281-byte FALCON secret-key buffer when a
  `Wallet` goes out of scope.
- Multi-contract orchestration regression test
  (`tests/contract_orchestration_mock.rs`, 3 cases). Pins the
  send → wait_for_receipt → call cycle across two distinct contract
  instances under the same signer; also pins `PendingTx`'s
  receipt-vs-polled-hash cross-check and a sequential
  send→read→send→read flow on a single contract. Wiremock dispatches
  per-contract via `body_string_contains` on the address hex.
  Caught contract-side code-path regressions wouldn't otherwise
  surface until the next live test sweep.
- `rust-toolchain.toml` pinning the floating-stable channel + MSRV
  declaration (`rust-version = "1.75"`) in `Cargo.toml`.
- `docs/13-utilities.md` + `docs/14-constants.md` — reference chapters
  for every public utility function and every public constant.
- Per-API expansion across all 12 existing doc chapters: args + returns
  + errors + example code + expected output for every public function.
- Crates.io metadata: `homepage`, `documentation`, `readme`,
  `keywords` (`blockchain`, `pyde`, `post-quantum`, `falcon`,
  `rpc-client`), `categories` (`cryptography`, `api-bindings`,
  `asynchronous`). Path-deps (`pyde-crypto`, `pyde-sdk-macros`)
  now carry paired `version` requirements so `cargo publish`
  accepts the manifest — actual publish still blocked on the
  upstream deps shipping to crates.io.

### Changed
- `Provider::get_nonce` now sends `pyde_getNonce` (the canonical
  Chapter 17.4 name) instead of `pyde_getTransactionCount`. The
  engine accepts both — the swap is wire-equivalent for users on
  any engine ≥ #337; older engines that don't dispatch the alias
  would error, but those don't exist in the deployed network.
- All docs + examples now point at `otigen devnet` (port `9933`) for
  the local chain runtime. The previous `pyde devnet` (port `8545`)
  references assumed users had cloned `pyde-net/engine`; the new path
  uses the otigen binary, which dev users already need for contracts.
  WebSocket endpoints now use `ws://127.0.0.1:9933/ws` (same port,
  `/ws` path).
- `HttpTransport::send` now retries transient failures automatically;
  this is a behaviour change that's strictly additive (a failing call
  that would have errored on the first attempt now errors after up to
  3 retries with the same error type). Disable via
  `RetryConfig::no_retry()` if your dapp needs first-try-wins semantics.

### Fixed
- Broken intra-doc link `[\`wallet_drop_wipes_secret\`]` in
  `src/wallet/mod.rs` — the target lives in `tests/` (not visible
  to rustdoc). Now a plain code-ref `wallet_drop_wipes_secret` +
  file pointer; unblocks `cargo doc --workspace --no-deps`
  with `-D warnings` (the docs.rs build invocation).
- **Event-signature canonical type names** — `write_canonical_type`
  now emits Solidity-style `uintN` / `intN` (matching the otigen
  `declare_events!` macro + ts-sdk + downstream explorers / indexers)
  instead of Rust-style `uN` / `iN`. Previously the SDK's
  reconstructed topic-0 didn't match the engine-emitted topic-0
  for any event whose author wrote a Solidity-style signature
  in `otigen.toml` — so `Contract::event_filter_for(name)` returned
  zero matches from `get_logs` and `Contract::decode_event` failed
  with "no event in ABI matches topic 0". Caught by the rust-sdk
  coverage pass against `otigen/examples/state-and-emit`
  (`Incremented(uint64,uint64,uint64)`).
- Doc-comment in `src/types/tx_types.rs` for `EmergencyPause` no
  longer says "halt block production" (wave-not-block terminology).
- Removed the false claim in README + `src/wallet/mod.rs` that the
  keystore is wire-compatible with `pyde-ts-sdk` — the two SDKs use
  different ciphers (AES-256-GCM vs ChaCha20-Poly1305) and different
  envelope shapes; cross-SDK import is documented as a future
  convergence in `docs/12-compatibility.md`.

## [0.1.0] — 2026-06-14

Initial public alpha. Surface lock — every wire type byte-pinned to
the chain engine.

### Added
- Core types: `Address`, `TxHash`, `Blake3Hash`, `Poseidon2Hash`,
  `FalconPubkey`, `FalconSignature`, `FalconSecret`, `Tx`, `TxType`,
  `AuthKeys`, `FeePayer`, `AccessEntry`, `AccessType`, `DeployData`,
  `Receipt`, `ReceiptStatus`, `Event`, `EventFilter`, `LogFilter`,
  `LogPage`, `LogCursor`, `NodeInfo`, `AccountInfo`, `WaveHeader`,
  `BlockHeader` (alias), `SimulationResult`, `CallRequest`,
  `CallOverrides`, `FeeData`, `Log` (alias), `ContractAbi`,
  `ContractType`, `FunctionAbi`, `ParamAbi`, `ParamType`,
  `FunctionAttrs`, `StateSchema`, `EventAbi`, `EnumVariant`.
- `Signer` trait + `LocalSigner` (FALCON-512 via `pyde-crypto`).
- `Wallet` with `to_keystore` / `from_keystore` (Argon2id +
  AES-256-GCM, SDK-specific envelope; not interchangeable with
  `pyde-ts-sdk`'s ChaCha20-Poly1305 keystore).
- `Provider` trait with 23 methods covering account reads, transaction
  submission + receipt polling, wave + event queries, validators + state
  snapshots. `HttpProvider` (reqwest + rustls) and `WsProvider`
  (tokio-tungstenite) implementations.
- `PendingTx` for receipt polling with configurable interval + timeout.
- `Subscription<T>` for WebSocket subscriptions; `subscribe_logs` is
  the only kind currently wired (others return `SdkError` immediately
  until the engine ships them).
- `TxBuilder` with fluent helpers: `transfer`, `deploy`, `call`,
  `multisig_treasury_spend`, plus raw setters for every field.
- `tx::tx_hash` (canonical Poseidon2 pre-image, signature excluded),
  `tx::encode`, `tx::decode`.
- ABI surface: `extract_abi(wasm)` parses the `pyde.abi` custom
  section; supports ABI versions up to `ContractAbi::V1_2`.
- `Contract` dynamic runtime — `load`, `load_at`, `new`, `call`,
  `call_with`, `send`, `build_tx`, `event_filter`,
  `event_filter_for`, `decode_event`.
- `pyde_abi!` proc-macro generating compile-time typed wrappers.
- `multisig` module: `canonical_msg`, `sign_action`, `BundleEntry`,
  `SigBundle`, `MultisigTxPayload`, domain-byte constants.
- `error::SdkError` + `ErrorCode` (HOST_FN_ABI §4) — 18 named codes,
  revert-reason decoding (Borsh String / legacy u64-prefixed / raw
  UTF-8), `SdkError::error_code()` with longest-match-wins token
  scanning + integer-code parsing.
- Utility functions: `parse_quanta`, `format_quanta`, `parse_units`,
  `format_units`, hex helpers.
- Gas constants: `GAS_TRANSFER`, `GAS_ERC20_CALL`, `GAS_ERC721_CALL`,
  `GAS_DEPLOY`, `GAS_CROSS_CALL_ORCHESTRATOR`.
- Security: codec DoS cap (`MAX_DECODE_ELEMENTS = 1_000_000`), WS
  flood caps (`MAX_PENDING_NOTIF_BUCKETS = 128`,
  `MAX_PENDING_NOTIF_DEPTH = 256`), `PendingTx` hash cross-check,
  constant-time `FalconPubkey::eq` via `subtle::ConstantTimeEq`.
- Examples: `wallet_basics`, `keystore`, `transfer`, `devnet_e2e`,
  `subscribe_logs`, `contract_dynamic`, `contract_typed`,
  `nft_marketplace`, `halt_methods`, `multisig_treasury`.
- 163 unit + 114 integration tests + cargo-fuzz harnesses
  (`fuzz_value_decode`, `fuzz_tx_decode`, `fuzz_jsonrpc_response`,
  `fuzz_revert_reason`, `fuzz_extract_error_code`, `fuzz_abi_extract`).
- Comprehensive docs at `docs/` — 12-chapter TOC covering install,
  quickstart, concepts, wallets, transactions, providers, contracts,
  events, errors, multisig, examples, compatibility.

[Unreleased]: https://github.com/pyde-net/pyde-rust-sdk/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pyde-net/pyde-rust-sdk/releases/tag/v0.1.0
