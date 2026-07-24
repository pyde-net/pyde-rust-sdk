# Changelog

All notable changes to `pyde-rust-sdk`. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning
follows [SemVer](https://semver.org/), with the caveat that everything
pre-1.0 may have breaking changes at any minor bump (see
[docs/12-compatibility.md](docs/12-compatibility.md#msrv--semver-intent)).

## [Unreleased]

## [0.3.2] — 2026-07-24

### Added
- `Receipt.nonce` (hex) + `Receipt.commit_reveal` (`bool`), matching the engine's
  new receipt fields, plus `Receipt::nonce_u64()` / `try_nonce_u64()` helpers
  (mirroring `wave_id_u64`). For a **commit-reveal inner op**, `nonce` is the
  INNER tx's nonce, so a lookup by the inner-op hash (not a standalone indexed
  tx) reports it. `commit_reveal` is delayed-disclosure ordering protection,
  **not confidentiality** — the op is public plaintext once revealed, so don't
  label it "private". Both `#[serde(default)]` → empty/`false` on older nodes.

## [0.3.1] — 2026-07-23

### Changed
- `AccountInfo.state_root` renamed to **`storage_root`** — matching the
  engine's accurate name for the per-account root (a v1 stub: always
  `0x0`, reserved for v2). Non-breaking: `#[serde(alias = "state_root")]`
  keeps deserialization working against nodes that emit either name.

## [0.3.0] — 2026-07-22

### Added
- **Factory-pattern off-chain surface** — new `factory`
  module (re-exported at the crate root) covering everything a
  wallet/script/indexer needs around `pyde::instantiate`:
  - `child_address(parent, template, salt)` — the canonical child
    derivation `Poseidon2("pyde-child:" ‖ parent ‖ template ‖ salt)`,
    byte-identical to the engine's. Compute a child's counterfactual
    address before it exists, offline, from public inputs only.
  - `child_preimage(...)` — the 107-byte fixed-width preimage,
    exposed for tooling.
  - `Salt::of(&value)` — identity salt: `Poseidon2(borsh(value))`
    for any `BorshSerialize` type (counters, names, config tuples).
  - `Salt::of_unordered_pair(a, b)` — symmetric-market salt: the two
    addresses sorted ascending bytewise (unsigned), concatenated raw,
    hashed. Same pool address regardless of token listing order.
  - `Instantiated` event decoder (`decode` / `TryFrom<&Event>`) for
    the provenance event the engine emits on every successful
    `pyde::instantiate`, with the pinned topic-0
    `factory::INSTANTIATED_TOPIC` = `Blake3("pyde.Instantiated")`.
    Rejects wrong topic-0, wrong topic count, wrong data length, and
    an emitter that doesn't match the recorded parent.
  - Conformance pinned by a full replay of the shared golden vectors
    (`tests/fixtures/child_address.golden.json`, a verbatim copy of
    the canonical `pyde-host/vectors/child_address.json`) — preimage
    assembly, the Poseidon2 hash itself, identity salts including
    the empty-borsh case, and the unordered-pair sort including the
    0x7f/0x80 sign-boundary vector.

## [0.2.0] — 2026-07-17

### Changed (BREAKING) — canonical keystore format
- The keystore is now the **canonical Pyde account keystore**: a
  multi-account JSON vault (`{ version, accounts: { name: entry } }`)
  shared byte-for-byte with the `otigen` CLI, `pyde-ts-sdk`, the
  playground, and the wallet. A keystore minted by any conformant tool
  decrypts in every other.
  - Argon2id parallelism `p` is now **4** (was 1), matching the spec
    floor and the reference implementation.
  - The address is no longer bound as AEAD associated data; AES-256-GCM
    is used with **no AAD** for cross-implementation interchange.
  - Entry shape is flat: `kdf { name, memory_kb, iterations, parallelism }`
    with `salt` / `nonce` / `ciphertext` / `cipher` at the entry level.
- **API:** `Wallet::to_keystore` and `Wallet::from_keystore` now take an
  account name (`to_keystore(name, password)`,
  `from_keystore(&ks, name, password)`). New: `add_to_keystore`,
  `Keystore::account_names`, and `KeystoreEntry` / `KdfParams` are public.
- **Migration:** `Wallet::from_keystore_json` reads both the canonical
  vault and the older nested single-account keystore this SDK wrote at
  `0.1.0`, so existing files keep opening. The reader accepts only
  `aes-256-gcm` and applies an anti-DoS upper clamp on the KDF params
  (`memory_kb ≤ 1 GiB`, `iterations ≤ 16`, `parallelism ≤ 16`, matching
  the reference implementations); it imposes no lower floor, so a
  legitimately-owned below-floor keystore still opens.
- Cross-impl parity is pinned by `tests/keystore_parity.rs`, which
  decrypts an `otigen`-CLI-minted golden keystore.

## [0.1.0] — 2026-06-14

Initial public alpha, published to crates.io — every wire type
byte-pinned to the chain engine. The MEV lane shipped as commit-reveal;
the earlier threshold-encryption design was removed before publication.

### Added
- **Two new `Provider` methods** for wave-head + fee-data queries:
  - `get_wave_head()` → `pyde_getWave` (no-arg form). Returns the
    latest committed wave in one round-trip; pairs with the
    engine's light-client head query. `None` only on a chain
    that has never committed a wave.
  - `get_fee_data()` → `pyde_getFeeData`. Current base-fee +
    suggested-tip snapshot plus the last 10 committed waves'
    gas utilisation (driving a wallet's gas-price slider or
    network-load chart in one round-trip).
- New `RecentWaveSummary { wave_id, gas_used, gas_limit, utilisation }`
  type for per-wave entries in `FeeData.recent_waves`.
- **Commit-reveal private mempool.** The MEV-protected lane is now a
  two-phase commit-reveal flow (no decryption key anywhere). New
  `TxType` variants `Commit = 0x11` and `Reveal = 0x12`, with typed
  payloads carried in `tx.data`:
  - `CommitPayload { commitment: [u8; 32], value_ceiling: u128 }` —
    the `tx.data` of a Commit; `tx.value` must equal
    `required_bond(value_ceiling)`.
  - `RevealPayload { commitment: [u8; 32], nonce: [u8; 32], inner_tx: Vec<u8> }`
    — the `tx.data` of a Reveal.
  - `crate::tx::commitment_hash(inner_tx_bytes, nonce)` =
    `Blake3(b"pyde-commit-reveal-v1" || inner_tx_bytes || nonce)`.
  - `crate::tx::required_bond(value_ceiling)` =
    `max(MIN_COMMIT_BOND, value_ceiling * COMMIT_BOND_BPS / 10_000)`.
  - Constants in `crate::tx`: `COMMIT_REVEAL_WINDOW_WAVES = 120`,
    `MIN_COMMIT_BOND = 1_000_000_000` (1 PYDE), `COMMIT_BOND_BPS = 100`.
  What this protects: content-targeted front-running is prevented —
  a transaction's ordering position is fixed before its contents are
  visible. It is not a total ordering lock: the reveal necessarily
  exposes the contents before the inner tx executes, and an unrelated
  tx arriving in the reveal-to-execute window can still be ordered
  around it.
- **`RootProvider::send_private(&signer, inner_tx) -> PrivateSendHandle`**
  — the one-call private-send flow: signs the inner tx, submits the
  Commit, waits the window, then submits the Reveal. `PrivateSendHandle`
  exposes `commit_hash()`, `reveal_hash()`, `inner_hash()`, and
  `await_receipt()` (which resolves on the *inner* tx receipt).
- **`TxBuilder::commit(commitment, value_ceiling)` and
  `TxBuilder::reveal(commitment, nonce, inner_tx_bytes)`** — low-level
  builders for relays or split-phase submission.
- **New `Provider` method** for the hard-finality cert:
  - `get_hard_finality_cert(wave_id)` →
    `pyde_getHardFinalityCert`. Returns the wave's hard-finality
    cert (bundle of ≥85 validator signatures) for light-client +
    cross-chain-bridge use. `wave_id` is sent as a bare JSON
    number on the wire (engine quirk shared with `get_wave`).
- `examples/private_transfer.rs` — end-to-end round-trip of the
  private mempool path via `send_private`. Signs a Tx, commits under
  `commitment_hash`, waits the commit-reveal window, reveals, and waits
  for the receipt under the inner tx hash. Run with
  `cargo run --example private_transfer`.
  Live-verified on `otigen devnet`: inner receipt lands on the
  single-validator devnet.
- `RetryConfig` for `HttpTransport` — exponential backoff with jitter
  on transient failures (connection refused, TCP/TLS errors, HTTP 5xx,
  HTTP 429). Default: 3 retries, 100 ms base, 5 s cap, ±25% jitter.
  Disable via `HttpTransport::new(url)?.with_retry_config(RetryConfig::no_retry())`.
- HTTP 429 (rate-limited) is now classified as transient + retried.
  The engine's rate limiter ships its retry-after hint in the
  response body (`"Wait for Ns"`); `HttpTransport` parses it and
  uses it as the next sleep duration (capped at 60 s to bound
  worst-case latency from a hostile server) instead of the default
  exponential backoff. Previously, 429 surfaced to the caller
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
- **Structured `revert_reason` plumbing.** New types
  `RevertCategory` (`EngineValidation` / `Contract` / `Vm` /
  `Other(String)` for forward-compat) and `RevertReason
  { category, message }` mirror the engine's `pyde_engine_types`
  wire shape. `Receipt.revert_reason: Option<RevertReason>` is
  populated when the engine emits the field and `None` for nodes
  that don't — backward-compat preserved via
  `#[serde(default)]`. Three new `Receipt` accessors
  (`is_engine_validation_revert`, `is_contract_revert`,
  `is_vm_trap`) make explorer/wallet badging trivial.
- **`SdkError::Reverted` carries the structured reason.** New
  `SdkError` methods: `revert_category()`,
  `is_engine_validation_revert()`, `is_contract_revert()`,
  `is_vm_trap()`. The `Reverted` variant gained a
  `reason: Option<RevertReason>` field — branch on category for
  UX (engine-side rejects deserve different treatment than
  contract `revert(msg)` than VM traps).
- **`SdkError::from_receipt(&Receipt) -> Option<Self>`** maps a
  non-success receipt to the right `SdkError` variant.
  `OutOfGas` synthesises a `Vm`-category reason; `Reverted`
  carries the engine's structured reason through (or `None` when
  the receipt didn't carry one). Drops the boilerplate where dapps were
  re-deriving the error variant from status + return_data
  manually.
- Test coverage grew from 163 unit / 114 integration in 0.1.0 to
  170 unit / 139 integration in this release (revert-reason
  plumbing, FeeData rework, commit-reveal payload serialisation,
  snapshot/manifest, hard-finality cert, private-send flow,
  retry-policy, structured Receipt accessors).
- Crates.io metadata: `homepage`, `documentation`, `readme`,
  `keywords` (`blockchain`, `pyde`, `post-quantum`, `falcon`,
  `rpc-client`), `categories` (`cryptography`, `api-bindings`,
  `asynchronous`). Path-deps (`pyde-crypto`, `pyde-sdk-macros`)
  now carry paired `version` requirements so `cargo publish`
  accepts the manifest — actual publish still blocked on the
  upstream deps shipping to crates.io.

### Removed
- **Breaking**: Threshold-encryption submission surface, superseded by
  the commit-reveal private mempool above. The engine physically
  deleted the threshold lane, so these no longer exist client-side:
  - `Provider::send_raw_encrypted_transaction` /
    `pyde_sendRawEncryptedTransaction`.
  - `Provider::get_threshold_public_key` / `pyde_getThresholdPublicKey`.
  - Types `EncryptedTxEnvelope` and `ThresholdPublicKey` (there is no
    key to fetch under commit-reveal).
  - `examples/encrypted_transfer.rs` (replaced by
    `examples/private_transfer.rs`).
  Migrate encrypted submits to `RootProvider::send_private` (or the
  `TxBuilder::commit` / `TxBuilder::reveal` split-phase builders).

### Changed
- **Breaking**: Reworked `FeeData` struct: dropped the `gas_price`
  field (was an unused alias for `base_fee`); added `wave_id`,
  `suggested_tip` (always `0` in v1, future-proof), and
  `recent_waves: Vec<RecentWaveSummary>`.
- **Breaking**: `SdkError::Reverted` now has a third field —
  `reason: Option<RevertReason>`. Callers using struct-syntax
  pattern matching need to add `reason: _` (or destructure it).
  Constructions need `reason: None` (older code path) or
  `reason: receipt.revert_reason.clone()` (when building from a
  receipt). Idiomatic path is `SdkError::from_receipt(&receipt)`
  which handles both forks. Code that already uses
  `{ gas_used, data, .. }` (with rest-pattern) is unaffected;
  only exhaustive struct-pattern matches need the new field.
- **Breaking (wire)**: `Provider::get_nonce` now sends
  `pyde_getNonce` instead of `pyde_getTransactionCount`. The
  deployed engine accepts both, so this is wire-equivalent at the
  live edge; only callers pinning fixtures by JSON-RPC method
  name need to update.
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

### Added (initial surface)
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

[Unreleased]: https://github.com/pyde-net/pyde-rust-sdk/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/pyde-net/pyde-rust-sdk/releases/tag/v0.3.0
[0.2.0]: https://github.com/pyde-net/pyde-rust-sdk/releases/tag/v0.2.0
[0.1.0]: https://github.com/pyde-net/pyde-rust-sdk/releases/tag/v0.1.0
