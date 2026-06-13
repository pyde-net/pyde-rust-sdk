# pyde-rust-sdk — v1 proposal

**Status**: awaiting your review. Don't write code until this is signed off.

**Scope confirmed in chat**: alloy-rs-equivalent for Pyde. Comprehensive surface — account gen, signing, signing, multisig, tx construction, RPC client (HTTP + WS), event subscriptions, contract codegen, encoding/decoding utils. Surface is large by design.

---

## 1. Recommendation: single crate + one proc-macro sub-crate

```text
pyde-rust-sdk/             ← THIS repo (single Cargo package today)
├── Cargo.toml             ← main crate
├── src/
│   ├── lib.rs             ← module declarations + curated re-exports
│   ├── types/             ← Address, TxHash, Receipt, Tx, FeePayer, etc.
│   ├── tx/                ← TxBuilder, canonical encoding, tx_hash
│   ├── signer/            ← Signer trait, LocalSigner
│   ├── wallet/            ← Wallet (Signer + helpers), Keystore (encrypted KDF)
│   ├── provider/          ← Provider trait, HttpProvider, fillers
│   ├── ws/                ← WsProvider, Subscription<T> → Stream
│   ├── contract/          ← Contract<P>, CallBuilder, Event filter
│   ├── abi/               ← ContractAbi parsing (pyde.abi WASM custom section)
│   ├── util/              ← parse/format quanta + units, hex helpers, address fmt
│   └── error/             ← SdkError taxonomy
├── pyde-sdk-macros/       ← proc-macro sub-crate (REQUIRED by Rust — macros must be their own crate)
│   ├── Cargo.toml         ← proc-macro = true
│   └── src/lib.rs         ← pyde_abi! macro
├── examples/              ← runnable wallet + dapp examples
└── tests/                 ← integration + parity tests
```

**Why single main crate, not alloy-style workspace?** Alloy splits ~15 crates for ecosystem reasons (no_std primitives shared with reth; per-HSM signer crates published separately). Pyde v1 doesn't need that. Single crate is faster to iterate, simpler dep graph, easier to understand. Internal module structure mirrors logical splits, so if we ever need to break it into `pyde-primitives` + `pyde-signer` + etc., the refactor is mechanical. Keep it simple for v1.

**Why a sub-crate for the macro?** Rust requires proc-macros to live in a crate marked `proc-macro = true`. Cannot coexist with regular code. Standard pattern; same as how `serde` + `serde_derive`, `tokio` + `tokio-macros`, `alloy` + `alloy-sol-macro`.

---

## 2. Public API shape — alloy-rs-shaped, Pyde-spec-aligned

User-facing convenience pattern:

```rust
use pyde_rust_sdk::{Provider, Wallet, Address};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider = Provider::http("http://127.0.0.1:8545")?;
    let wallet = Wallet::generate()?;
    let recipient = Address::parse("0xaa...")?;

    let pending = provider
        .transfer(&wallet, recipient, 1_000_000_000)  // 1 PYDE in quanta
        .send()
        .await?;
    let receipt = pending.await_finalized().await?;
    println!("{:?}", receipt);
    Ok(())
}
```

Lower-level fluent builder (for non-transfer ops):

```rust
let tx = Tx::builder()
    .from(wallet.address())
    .to(contract_addr)
    .value(0)
    .data(calldata)
    .gas_limit(200_000)
    .nonce(provider.get_nonce(&wallet.address()).await?)
    .chain_id(provider.chain_id().await?)
    .deadline(provider.wave_id().await? + 120)
    .build();
let signed = wallet.sign_tx(tx)?;
let receipt = provider.send_and_wait(&signed, 5_000).await?;
```

Contract codegen (Phase A.late, see §4):

```rust
use pyde_rust_sdk::pyde_abi;

pyde_abi! {
    contract Counter {
        fn increment();
        fn get_count() -> u64;
        event CountChanged(u64);
    }
}

let counter = Counter::at(contract_addr, &provider);
counter.increment(&wallet).send().await?.await_finalized().await?;
let n = counter.get_count().call().await?;
```

Subscription:

```rust
let ws = WsProvider::connect("ws://127.0.0.1:8546").await?;
let mut stream = ws.subscribe_logs(&filter).await?.into_stream();
while let Some(log) = stream.next().await {
    println!("{:?}", log);
}
```

---

## 3. Dep graph (Cargo.toml)

```toml
[dependencies]
# Crypto — sibling path-dep per pyde-net convention
pyde-crypto = { path = "../pyde-crypto" }       # FALCON, Poseidon2, Blake3, Kyber

# Async runtime + transport
tokio = { version = "1", features = ["time", "sync", "macros", "rt-multi-thread"] }
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
tokio-tungstenite = { version = "0.24", features = ["native-tls"] }
futures-util = "0.3"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
borsh = "1"                  # canonical Pyde wire encoding (matches engine)
hex = "0.4"

# Encrypted keystore (KDF + AEAD for at-rest secret-key storage)
argon2 = "0.5"
aes-gcm = "0.10"
rand = "0.8"
zeroize = { version = "1", features = ["alloc"] }

# WASM-binary parsing (read pyde.abi custom section from contract bytecode)
wasmparser = "0.220"

# Big-integer support for contract calldata u256/i256 params
ruint = { version = "1", features = ["serde"] }   # cleaner than ethnum; alloy uses ruint too

# Error handling
thiserror = "2"

# In-crate macro re-export
pyde-sdk-macros = { path = "./pyde-sdk-macros" }
```

Notable choices vs pre-pivot:
- `ruint` instead of `ethnum` (alloy convention; better serde support)
- `borsh` ADDED (pre-pivot tried to delegate canonical encoding to `pyde-tx` — we now own it)
- `wasmparser` ADDED (parse `pyde.abi` custom section from `pyde_getContractCode` results)
- `reqwest` switched to `rustls-tls` (cleaner than native-tls; one fewer system dep)
- `pyde-tx` / `pyde-account` REMOVED entirely (SDK owns its types — standard pattern)

---

## 4. v1 surface — phased

**Phase A1: foundation (week 1, no contract codegen yet)**
- `types::Address`, `TxHash`, `Receipt`, `Log`, `LogFilter`, `BlockHeader` / `WaveHeader`, `Tx`, `FeePayer`, `AccessEntry`, `AuthKeys`, `AccountType`, `TxType`
- `util::` — `parse_quanta`, `format_quanta`, `parse_units`, `format_units`, `hexlify`, `parse_address`, `format_address`, `is_valid_address`, `zero_pad`, `concat_bytes`
- `error::SdkError`, `Result`
- `tx::TxBuilder`, `tx::canonical_encode`, `tx::tx_hash` (Poseidon2 over canonical pre-image per Ch 11 §11.6)
- `signer::Signer` trait, `signer::LocalSigner` (in-memory FALCON keypair)
- `wallet::Wallet::generate`, `Wallet::from_signer`, `Wallet::sign_tx`, `Wallet::address`
- `wallet::Keystore` (argon2 + AES-GCM encrypted secret-key at-rest, like ethers' encrypted JSON keystore)
- 10 unit tests + canonical-encoding parity tests against engine

**Phase A2: provider + RPC (week 2)**
- `provider::Provider` trait, `provider::HttpProvider`
- All 23 RPC method wrappers from T2 survey
- `provider::PendingTx` (alloy's `PendingTransactionBuilder` equivalent — `.send()` → `.await_committed()` → `.await_finalized()`)
- Convenience builders on `Provider`: `transfer(&wallet, to, value)`, `deploy(&wallet, wasm_bytes)`, `call_contract(&wallet, addr, fn_name, args)`
- Fillers (alloy-style middleware): `NonceFiller`, `GasFiller`, `ChainIdFiller`, `WalletFiller` — composed via builder
- `ws::WsProvider`, `Subscription<T>` → `Stream` (subscribe_logs, subscribe_events, subscribe_new_waves)
- Integration tests against a mocked RPC + a real devnet (gated `#[ignore]` for CI)

**Phase A3: contract codegen (week 3)**
- `abi::parse_pyde_abi(&wasm_bytes)` → `ContractAbi` (reads the `pyde.abi` custom section)
- `contract::Contract<P: Provider>` — typed wrapper around an address + provider
- `contract::CallBuilder` — fluent build for contract calls
- `pyde-sdk-macros::pyde_abi!` proc-macro — generates typed Instance + Function calls + Event filters from a contract declaration. **Selector = Blake3(name)[..4]** per spec §3.7 (see Open Question 1 — confirm with otigen first).
- Typed calldata encoders/decoders for all primitive types: `bool, u8/u16/u32/u64/u128, i8/i16/i32/i64/i128, U256/I256, Address, String, Vec<u8>, Vec<T>`
- Event filter helpers

**Phase B (post-v1, follow-up PRs in order):**
- B1: Multisig signing flow (`AuthKeys::MultiSig` — collect threshold sigs, build aggregated tx)
- B2: Hardware-wallet `Signer` impl traits (no concrete HSM ship yet; just the trait shape so external crates can plug in)
- B3: Access-list builder via `pyde_simulateTransaction` (once node spec drift resolved — see action A2)
- B4: Encrypted-tx (MEV-protected) — wire format + Provider methods (once node ships those RPC methods — action A3)
- B5: Parachain `Network` trait (when parachain RPC ships)

**Phase C (v2 / post-mainnet):**
- Session keys
- Programmable accounts
- Threshold encryption helpers

---

## 5. What gets deleted from pre-pivot

Doing the salvage-and-update I committed to:

| File | LOC | Action |
|---|---|---|
| `src/types.rs` | 342 | Salvage all utility helpers; rewrite `Receipt`/`Log`/etc. to match current engine wire shapes |
| `src/error.rs` | 99 | Salvage taxonomy; align error codes with HOST_FN_ABI §4 (the 17 spec'd codes) |
| `src/signer.rs` | 16 | Salvage trait shape, async-ify if needed |
| `src/wallet.rs` | 810 | Salvage Keystore (encryption is intact); rewrite Wallet methods that depend on broken engine types |
| `src/client.rs` (Provider) | 578 | Significant rewrite — drop 4 missing methods, rename 4, add 8 new (per T2 survey) |
| `src/contract.rs` | 635 | Significant rewrite — current SDK uses generic `Value` enum; new SDK uses typed codegen via macro. Salvage decode helpers. |
| `src/abi.rs` | 1588 | Substantial rewrite — pre-pivot was Solidity-shaped JSON ABI; new SDK uses Pyde's Borsh `pyde.abi` custom section (totally different parser) |
| `src/encrypted.rs` + `src/encrypted_wire.rs` | 522 | **DELETE for v1**, restore in Phase B4 when RPC ships |
| `src/ws.rs` | 223 | Rewrite — must wrap `pyde_subscribe`/`pyde_unsubscribe` matching node-side dispatcher shape |
| `examples/loadgen_ext.rs` | 316 | Rewrite as a clean example file once SDK shape settles |
| `tests/live_test.rs` | 743 | Delete and rewrite as separate integration tests per module |

**Net**: ~3000 LOC salvageable (utility helpers, keystore, basic structure), ~2900 LOC rewritten or deleted.

---

## 6. Quality bar (your commitment from the chat)

Every module ships with:
- Module-level `//!` doc explaining what it does, when to use it, what it owns
- Function-level `///` doc with `# Errors` and `# Examples` where they help
- Unit tests for the obvious cases + at least one negative test per function
- No `unwrap()` outside tests (use `?` + `SdkError`)
- No `panic!()` outside `unreachable!()` invariants that compile-time-prove can't fire
- Clippy-clean under the same lint bar as pyde-crypto-wasm (`unwrap_used = deny`, `print_*  = deny`, etc.)
- `cargo doc` builds clean — no broken intradoc links
- Public APIs use Pyde-spec terminology consistently (wave_id not block_number; quanta not wei; pubkey not public_key for FALCON-specific types)

---

## 7. Open questions for you before the gate

1. **Confirm Phase A scope (A1 + A2 + A3) is what you want for v1.** Or pull contract codegen (A3) out as Phase B and ship transfer + tx-construction + RPC client first?

2. **Crate name — keep `pyde-rust-sdk` or rename to `pyde-sdk`?** Alloy/ethers/web3 don't include language in name. `pyde-ts-sdk` already exists so naming parallel is broken either way. I'd default to keeping the existing name (less churn), but flagging in case you want the cleaner one.

3. **Should `Wallet::generate` use system RNG or a deterministic seed?** Default-deterministic-seed makes tests easier; default-OS-entropy is safer for production. Standard pattern: `generate()` uses OS entropy; `generate_from_seed(&[u8; 32])` is opt-in for testing.

4. **Keystore format compatibility with pyde-ts-sdk + pyde-crypto-wasm.** If a user encrypts a wallet on Rust and imports on TS, should they be byte-compatible? Worth doing — argon2 + AES-GCM with a JSON envelope is the standard pattern (matches ethers/eth-keyfile JSON v3). Confirm we want cross-language compatibility?

5. **Action items A1–A4 from T3 — should I open issues / send messages to those other sessions now, or wait until I'm actually blocked?** I'd lean wait-until-blocked for v1, since A2 + A3 + A4 are Phase B concerns and A1 (selector hash) only affects the macro in A3. So I could ship A1+A2 without unblocking any of those first.

---

## 8. Once approved → execution plan

Add to task tracker:
- T7 `cargo init` clean tear-down (rm src/, fresh skeleton matching §1 layout)
- T8 Phase A1 — types + util + error + tx encoding + signer + wallet + keystore
- T9 Phase A2 — provider + http + ws + fillers + RPC method wrappers
- T10 Phase A3 — abi parser + contract instance + pyde_abi! macro
- T11 Examples + integration tests
- T12 CI mirror (Makefile + GH Actions per the pyde-crypto-wasm pattern)
- T13 PR + audit pass against spec

---

**Awaiting your sign-off on (a) the proposed crate structure, (b) the dep graph, (c) the phased v1 scope, and (d) the 5 open questions in §7.** Once you respond, I'll mark T6 complete and start T7.
