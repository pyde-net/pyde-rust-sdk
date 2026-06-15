# 12. Compatibility

[← back to TOC](README.md) · prev: [Examples](11-examples.md) · next: [Utilities →](13-utilities.md)

---

The SDK's job is to put the right bytes on the wire. This
chapter pins down what "right" means — version-by-version — so
you can reason about upgrade safety and cross-SDK behaviour.

## Table of contents

- [12.1 Wire-format guarantees](#121-wire-format-guarantees)
- [12.2 Hash strategy](#122-hash-strategy)
- [12.3 ABI version range](#123-abi-version-range)
- [12.4 TS SDK delta](#124-ts-sdk-delta)
- [12.5 Keystore format differences](#125-keystore-format-differences)
- [12.6 Size limits](#126-size-limits)
- [12.7 MSRV + semver intent](#127-msrv--semver-intent)
- [12.8 Versioning of this crate](#128-versioning-of-this-crate)
- [12.9 Reporting drift](#129-reporting-drift)

---

## 12.1 Wire-format guarantees

Everything the SDK puts on the JSON-RPC wire is **byte-for-byte
identical** to its counterpart in `engine/crates/types/`. Tests
in this crate pin discriminants, field orders, and constants
against the engine; if the engine reshapes a type, our test
suite fails before the drift hits production.

### Type-by-type pins

| SDK type | Engine type | Pin |
|---|---|---|
| `Tx` | `pyde_engine_types::Tx` | Field order matches Ch 11 §11.6 verbatim. |
| `TxType` | `pyde_engine_types::TxType` | 16 variants, tags `0x00`–`0x10` (`0x02` reserved gap). |
| `AuthKeys` | `pyde_engine_types::AuthKeys` | 4 variants, tags `0x00`–`0x03`. |
| `FeePayer` | `pyde_engine_types::FeePayer` | 3 variants, tags `0x00`–`0x02`. |
| `AccessEntry` | `pyde_engine_types::AccessEntry` | Field order: `address`, `storage_keys`, `access_type`. |
| `AccessType` | `pyde_engine_types::AccessType` | 2 variants, tags `0x00` (Read) / `0x01` (ReadWrite). |
| `DeployData` | `pyde_engine_types::DeployData` | Field order: `name`, `wasm_bytes`, `contract_type`, `init_calldata`. |
| `ContractType` | `pyde_engine_types::ContractType` | 2 variants, tags `0x00` (Contract) / `0x01` (Parachain). |
| `MultisigTxPayload` | `pyde_engine_tx::handlers::multisig_tx::MultisigTxPayload` | `target` ‖ `amount` ‖ `bundle`. |
| `BundleEntry` | `pyde_engine_tx::multisig::BundleEntry` | `signer_index` ‖ `signature`. |
| Multisig domain bytes | `pyde_engine_tx::multisig::domain_byte` | `0x09`, `0x0A`, `0x0B`, `0x0C`, `0x10`. |
| `Receipt` | `pyde_engine_types::Receipt` | Field order matches engine. |
| `Event` | `pyde_engine_types::Event` | Hex-string fields throughout (JSON-RPC convention). |

### Tx canonical hash

The hash a signer signs is:

```
preimage = borsh((from, to, value, Poseidon2(data),
                  gas_limit, nonce, fee_payer, access_list,
                  deadline, chain_id, tx_type))
tx_hash  = Poseidon2(preimage)
```

The chain computes the same hash on submission. **Any field
drift after signing invalidates the signature.** SDK + engine
both implement this exactly; tests pin a known-input vector to
catch drift early.

---

## 12.2 Hash strategy

Pyde's dual-hash design is locked at v1:

| Hash | Where | Crate |
|---|---|---|
| **Poseidon2** | `tx_hash`, address derivation, multisig `canonical_msg`, state root, slot derivation | `pyde_crypto::poseidon2` |
| **Blake3** | Event-signature topics, devnet seed derivation, mempool dedupe | `blake3` |

Both produce 32-byte digests. The SDK ships them as distinct
newtypes (`Poseidon2Hash`, `Blake3Hash`, `TxHash`) so you can't
accidentally feed a Blake3 result where Poseidon2 is required.

| Type | Distinguished by |
|---|---|
| `TxHash` | The canonical Poseidon2 hash of a `Tx` pre-image. |
| `Poseidon2Hash` | Anything else Poseidon2-hashed (state roots, addresses). |
| `Blake3Hash` | Blake3 output (topics, seeds). |

All three are 32-byte `[u8; 32]` newtypes; type-distinct at
compile time, byte-identical at runtime.

---

## 12.3 ABI version range

Contracts ship their ABI inside the WASM via a `pyde.abi` custom
section. The SDK can decode every version up to
`ContractAbi::MAX_SUPPORTED`:

| Constant | Value | Notes |
|---|---|---|
| `V1_0` | `0x0001_0000` | First v1 ABI version. |
| `V1_1` | `0x0001_0001` | Added optional fields (additive). |
| `V1_2` | `0x0001_0002` | Current. |
| `MAX_SUPPORTED` | `Self::V1_2` | What this SDK release decodes. |

Encoding: `(major << 16) | minor`. Minor bumps are additive
(new optional fields); major bumps require an SDK upgrade.

If `extract_abi(wasm)` returns
`SdkError::InvalidArgument("unsupported ABI version 0x...")`,
upgrade the SDK to a newer release.

---

## 12.4 TS SDK delta

`pyde-ts-sdk` is the in-browser companion SDK. Shared with this
crate:

| Shared with TS SDK | Notes |
|---|---|
| **Chain wire format** | `Tx`, `TxType`, `AuthKeys`, FALCON pubkey/sig byte shapes, Poseidon2 / Blake3 hashes. A tx signed in Rust verifies fine on a TS-side client (and vice versa). |
| **Provider trait shape** | Same 23 method names, same JSON-RPC parameter conventions. |
| **ABI parser** | Both crates parse the same `pyde.abi` custom section into matching structs. |

| NOT shared |
|---|
| **Keystores** — different cipher + envelope shape. See §12.5. |
| **Browser-native HTTP client** — TS uses `fetch`; this SDK uses reqwest+rustls. |
| **The `pyde_abi!` proc-macro** — TS uses runtime ABI loading via type generation at build time. |

### What this SDK has that TS doesn't

- Hardware-wallet-ready `Signer` trait — object-safe + async-friendly.
- Synchronous `tx::encode` / `tx::decode` for tooling that
  doesn't want to drag in tokio.
- The `pyde_abi!` proc-macro generating typed wrappers at compile time.

### What TS has that this SDK doesn't

- Browser-native HTTP client (no native bindings).
- WebCrypto-backed signing path on devices that support FALCON
  via WASM crypto.

---

## 12.5 Keystore format differences

Different cipher, different envelope shape. Side-by-side:

| Field | This SDK | `pyde-ts-sdk` |
|---|---|---|
| Pubkey field name | `pubkey` (snake_case) | `publicKey` (camelCase) |
| KDF shape | nested `{ "kdf": { "name": "argon2id", "params": {…} } }` | flat `"kdf": "argon2id"` + separate `"kdfParams": {…}` |
| Cipher algorithm | **AES-256-GCM** | **ChaCha20-Poly1305** |
| Cipher shape | nested `{ "cipher": { "name": "aes-256-gcm", "nonce", "ciphertext" } }` | flat `"cipher": "chacha20-poly1305"` + top-level `"nonce"` + `"ciphertext"` |
| Hex prefix | `"0x..."` | no prefix |

### A keystore generated in one SDK cannot be loaded by the other today

If you need to migrate today: decrypt the source SDK's keystore
with its native crypto stack, extract the 897-byte pubkey +
1281-byte secret, then construct a new keystore via the target
SDK's `Wallet::from_keys` + `to_keystore`.

### Convergence is on the roadmap

Both SDKs will eventually settle on one format (probably
ChaCha20-Poly1305 + flat shape since TS's is cleaner and the
cipher choice is more browser-friendly). When that lands, you
won't need migration code.

---

## 12.6 Size limits

The SDK enforces a few in-process caps to defend against hostile
RPC responses:

| Limit | Value | What |
|---|---|---|
| `MAX_DECODE_ELEMENTS` | `1_000_000` | Max declared length on any borsh `Vec` / `String` decoded by `src/contract/codec.rs`. Stops a peer from declaring a gigabyte-long array. |
| Max concurrent WS subscriptions per transport (private) | `128` | 129th subscription is dropped. |
| Max queued events per WS subscription (private) | `256` | Events past this cap drop the slowest reader. |

The chain has its own caps (`MAX_CALLDATA`, `MAX_TX_SIZE`,
`MAX_STORAGE_VALUE`, etc.) — see the chain spec for those. The
SDK's caps are about defending the in-process process, not the
chain itself.

See [Constants §14.5](14-constants.md#145-codec-caps).

---

## 12.7 MSRV + semver intent

- **MSRV**: Rust **1.75** (`async fn` in traits stabilised here).
  Declared via `rust-version = "1.75"` in `Cargo.toml`; bumping
  is a minor-version SDK change.
- **Semver pre-1.0**: expect breaking changes at any minor bump
  (`0.x → 0.y`). Pin git revisions in production.
- **Wire-format changes**: coordinated with the engine and the
  TS SDK; ship as paired commits across all three repos. We
  never quietly change the wire.

### Verifying MSRV locally

```sh
rustup install 1.75
cargo +1.75 build
```

If that fails on your machine, the MSRV claim is wrong — file
an issue.

---

## 12.8 Versioning of this crate

Until 1.0, treat every `0.x → 0.(x+1)` as a potentially
breaking change. Pin git revs in production:

```toml
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk", rev = "<sha>" }
```

### Post-1.0 policy (planned)

- `1.x.y` → `1.x.(y+1)`: patches, bug fixes, doc tweaks.
- `1.x.y` → `1.(x+1).0`: additive changes, new helpers.
- `1.x.y` → `2.0.0`: breaking wire-format changes (rare —
  coordinated with engine + TS SDK).

### Following changes

Every release goes into [CHANGELOG.md](../CHANGELOG.md), in
[Keep a Changelog](https://keepachangelog.com) format. Skim the
**Unreleased** section before pulling latest on `main`.

---

## 12.9 Migration: `get_receipt` return type (Unreleased)

If you're tracking `main` past T27 (commit `c503490`),
`Provider::get_receipt` changed shape. **Hot-state callers
should switch to `get_transaction_receipt`** — same data, no
code changes beyond the method name.

### What changed

```diff
- async fn get_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError>;
+ async fn get_receipt(&self, hash: &TxHash) -> Result<Option<RawReceipt>, SdkError>;
```

The trait-level doc previously claimed `get_receipt` and
`get_transaction_receipt` returned "the same shape." That was
wrong — the engine emits them via two distinct paths:

| Method | Engine path | Wire shape |
|---|---|---|
| `pyde_getTransactionReceipt` | custom `receipt_to_json()` formatter | All-hex strings + snake_case status |
| `pyde_getReceipt` | raw `serde_json::to_value(Receipt)` | Byte-array `tx_hash`, raw integers, PascalCase status |

The engine team confirmed in the [RPC catalog](https://github.com/pyde-net/engine)
(gotcha #1) that both shapes are stable contracts — the
divergence won't be unified server-side. The SDK now carries
one deserializer per method.

### Which method should you use?

Decision tree:

1. **You're a wallet / dapp polling for "did my tx commit?"** —
   Use `get_transaction_receipt(hash)`. Returns `Option<Receipt>`
   (unchanged). Hot-state path; matches `PendingTx::wait_for_receipt`.

2. **You're an explorer / indexer doing archival lookups past
   the hot-state TTL** — Use `get_receipt(hash)`. Returns
   `Option<RawReceipt>` (new type). Reads from the consensus
   archive.

3. **You don't know which one** — Default to
   `get_transaction_receipt`. It falls through to the consensus
   archive automatically when the hot state has aged out.

### Migrating code

#### Path 1 — you used `get_receipt` for hot-state polling

Just rename:

```diff
- let receipt = provider.get_receipt(&hash).await?;
+ let receipt = provider.get_transaction_receipt(&hash).await?;
  match receipt {
      Some(r) if r.is_success() => { /* unchanged */ }
      // ...
  }
```

No other code changes — `Receipt` and its accessors
(`r.is_success()`, `r.gas()`, `r.fee_paid_quanta()`,
`r.wave_id_u64()`, `r.tx_index_u32()`) work identically.

#### Path 2 — you genuinely need the archival shape

Switch to the new `RawReceipt` type. The field names match
`Receipt` but the types are different — accessors are
unnecessary because the integers are already integers, not
hex strings:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::types::{RawReceipt, RawReceiptStatus, TxHash};
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>, hash: TxHash) -> pyde_rust_sdk::Result<()> {
match provider.get_receipt(&hash).await? {
    Some(receipt) => {
        // Direct field access — no hex parsing needed.
        let wave = receipt.wave_id;                // u64, raw
        let gas = receipt.gas_used;                // u64, raw
        let fee = receipt.fee_paid;                // u128, raw
        let ok = matches!(receipt.status, RawReceiptStatus::Success);
        let tx_hash_bytes: [u8; 32] = receipt.tx_hash;  // raw byte array

        println!("archival receipt: wave {wave} gas {gas} fee {fee} ok={ok}");
        let _ = tx_hash_bytes;
    }
    None => println!("not in consensus archive (may still be in hot state)"),
}
# Ok(()) }
```

### Bridging the two

If you need a single function that handles both shapes
(e.g., explorer fallback code), convert `RawReceiptStatus` →
`ReceiptStatus` via the provided `From` impl and write a small
adapter:

```rust,no_run
# use pyde_rust_sdk::types::{RawReceipt, ReceiptStatus};
fn raw_is_success(r: &RawReceipt) -> bool {
    let status: ReceiptStatus = r.status.into();
    matches!(status, ReceiptStatus::Success)
}
```

---

## 12.10 Reporting drift

Found a wire-format claim in these docs that doesn't match the
engine? File an issue at
[github.com/pyde-net/pyde-rust-sdk/issues](https://github.com/pyde-net/pyde-rust-sdk/issues)
with the field name + the engine source file you're comparing
against.

Wire-format claims are tested but the surface is large; fresh
eyes catch things. Include:

- The doc file + line claiming X.
- The engine file + line showing Y.
- A minimal reproducer if behaviour drift is at play (not just doc-only).
