# 12. Compatibility

[← back to TOC](README.md) · prev: [Examples](11-examples.md)

---

The SDK's job is to put the right bytes on the wire. This
chapter pins down what "right" means — version-by-version — so
you can reason about upgrade safety and cross-SDK behaviour.

## Wire-format guarantees

Everything the SDK puts on the JSON-RPC wire is **byte-for-byte
identical** to its counterpart in
`engine/crates/types/`. Tests in this crate pin discriminants,
field orders, and constants against the engine; if the engine
reshapes a type, our test suite fails before the drift hits
production.

| SDK type | Engine type | Pin |
|---|---|---|
| `Tx`                  | `pyde_engine_types::Tx`            | Field order matches Ch 11 §11.6 |
| `TxType`              | `pyde_engine_types::TxType`        | 16 variants, tags 0x00–0x10 (0x02 reserved gap) |
| `AuthKeys`            | `pyde_engine_types::AuthKeys`     | 4 variants, tags 0x00–0x03 |
| `FeePayer`            | `pyde_engine_types::FeePayer`      | 3 variants, tags 0x00–0x02 |
| `AccessEntry`         | `pyde_engine_types::AccessEntry`   | Field order: address, storage_keys, access_type |
| `AccessType`          | `pyde_engine_types::AccessType`    | 2 variants, tags 0x00 (Read) / 0x01 (ReadWrite) |
| `DeployData`          | `pyde_engine_types::DeployData`    | Field order: name, wasm_bytes, contract_type, init_calldata |
| `ContractType`        | `pyde_engine_types::ContractType`  | 2 variants, tags 0x00 (Contract) / 0x01 (Parachain) |
| `MultisigTxPayload`   | `pyde_engine_tx::handlers::multisig_tx::MultisigTxPayload` | target ‖ amount ‖ bundle |
| `BundleEntry`         | `pyde_engine_tx::multisig::BundleEntry` | signer_index ‖ signature |
| Multisig domain bytes | `pyde_engine_tx::multisig::domain_byte` | 0x09, 0x0A, 0x0B, 0x0C, 0x10 |

## Hash strategy

Pyde's dual-hash design is locked at v1:

| Hash | Where | Crate |
|---|---|---|
| **Poseidon2** | `tx_hash`, address derivation, multisig canonical_msg, state root | `pyde_crypto::poseidon2` |
| **Blake3** | Event-signature topics, devnet seed derivation, mempool dedupe | `blake3` |

Both produce 32-byte digests. The SDK ships them as distinct
newtypes (`Poseidon2Hash`, `Blake3Hash`, `TxHash`) so you
can't accidentally feed a Blake3 result where Poseidon2 is
required.

## ABI version range

Contracts ship their ABI inside the WASM via a `pyde.abi` custom
section. The SDK can decode every version up to
`ContractAbi::MAX_SUPPORTED`:

```rust,ignore
pub const V1_0: u32          = 0x0001_0000;
pub const V1_1: u32          = 0x0001_0001;
pub const V1_2: u32          = 0x0001_0002;
pub const MAX_SUPPORTED: u32 = Self::V1_2;
```

Encoding is `(major << 16) | minor`. Minor bumps are additive
(new optional fields); major bumps require an SDK upgrade.

`extract_abi(wasm)` returns `SdkError::InvalidArgument` if the
ABI version exceeds `MAX_SUPPORTED`. Upgrade the SDK to decode
newer-version contracts.

## Size limits

| Limit | Value | What |
|---|---|---|
| `MAX_DECODE_ELEMENTS` | 1,000,000 | Max declared length on any borsh `Vec` / `String` decoded by `src/contract/codec.rs`. Protects against hostile RPC responses that try to allocate gigabytes. |
| `MAX_PENDING_NOTIF_BUCKETS` | 128 | Max concurrent subscriptions a `WsTransport` will demux. |
| `MAX_PENDING_NOTIF_DEPTH` | 256 | Max pending events per subscription before the slowest reader gets dropped. |

The chain has its own size caps (`MAX_CALLDATA`, `MAX_TX_SIZE`,
`MAX_STORAGE_VALUE`, etc.) — see the chain spec for those.
The SDK's caps are about defending the in-process process, not
the chain.

## MSRV + semver intent

- **MSRV**: Rust 1.75 (`async fn in traits`). Bumping the MSRV
  is a minor-version SDK change (`0.x → 0.y`).
- **Semver**: pre-1.0, expect breaking changes at any minor
  bump. Once we tag 1.0, we'll publish a written semver policy
  pinning what counts as breaking (cipher swaps, wire format
  changes, public type renames, etc.).
- **Wire-format changes** are coordinated with the engine and
  the TS SDK and ship as a paired commit across all three
  repos. We never quietly change the wire.

## TS SDK delta

`pyde-ts-sdk` is the in-browser companion SDK. Shared with this
crate:

- **Chain wire format** — `Tx`, `TxType`, `AuthKeys`, FALCON
  pubkey/sig byte shapes, Poseidon2 / Blake3 hashes. A tx signed
  in Rust verifies fine on a TS-side client (and vice versa).
- **Provider trait shape** — same 23 method names, same JSON-RPC
  parameter conventions.
- **ABI parser** — both crates parse the same `pyde.abi`
  custom section into matching structs.

NOT shared:

### Keystores

Different cipher, different envelope shape. Side-by-side:

| Field | This SDK | `pyde-ts-sdk` |
|---|---|---|
| Pubkey field name | `pubkey` (snake_case) | `publicKey` (camelCase) |
| KDF shape | nested `{ "kdf": { "name": "argon2id", "params": {…} } }` | flat `"kdf": "argon2id"` + separate `"kdfParams": {…}` |
| Cipher algorithm | **AES-256-GCM** | **ChaCha20-Poly1305** |
| Cipher shape | nested `{ "cipher": { "name": "aes-256-gcm", "nonce", "ciphertext" } }` | flat `"cipher": "chacha20-poly1305"` + top-level `"nonce"` + `"ciphertext"` |
| Hex prefix | `"0x..."` | no prefix |

A keystore generated in one SDK can't be loaded by the other
without manual translation (decrypt with the source SDK's
crypto, re-encrypt with the target SDK's). **Convergence is on
the roadmap** — both SDKs will eventually settle on one format
(probably ChaCha20-Poly1305 + flat shape since TS's is cleaner
and the cipher choice is browser-friendlier).

### What the TS SDK has that this one doesn't

- Browser-native HTTP client (no native bindings).
- WebCrypto-backed signing path on devices that support FALCON
  via WASM crypto.

### What this SDK has that the TS one doesn't

- Hardware-wallet-ready `Signer` trait that's object-safe and
  async-friendly.
- Synchronous `tx::encode` / `tx::decode` for tooling that
  doesn't want to drag in tokio.
- The `pyde_abi!` proc-macro (TS uses runtime ABI loading).

## Versioning of this crate

Until 1.0, treat every `0.x → 0.(x+1)` as a potentially
breaking change. Pin git revs in production:

```toml
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk", rev = "<sha>" }
```

After 1.0 (post-mainnet):

- `1.x.y` → `1.x.(y+1)`: patches, bug fixes, doc tweaks.
- `1.x.y` → `1.(x+1).0`: additive changes, new helpers.
- `1.x.y` → `2.0.0`: breaking wire format changes (rare —
  coordinated with engine + TS SDK).

## Reporting drift

Found a wire-format claim in these docs that doesn't match the
engine? File an issue at
[github.com/pyde-net/pyde-rust-sdk/issues](https://github.com/pyde-net/pyde-rust-sdk/issues)
with the field name + the engine source file you're comparing
against. Wire-format claims are tested but the surface is large;
fresh eyes catch things.
