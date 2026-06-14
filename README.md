<p align="center">
  <img src="./assets/logo.png" width="120" alt="Pyde logo" />
</p>

<h1 align="center">pyde-rust-sdk</h1>

<p align="center">
  <em>Async Rust SDK for the Pyde Network blockchain</em>
</p>

---

Account generation, FALCON-512 signing, transaction construction, JSON-RPC client (HTTP + WebSocket), typed contract interaction. Modeled on alloy-rs.

> v1 surface is locked. Wire types are byte-for-byte compatible with the chain engine.

## Install

```toml
[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
```

## Quick start

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::util::parse_quanta;
use pyde_rust_sdk::{Address, Provider, Signer, TxBuilder, Wallet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let transport = HttpTransport::new("http://127.0.0.1:8545")?;
    let provider = Arc::new(RootProvider::new(transport));

    let wallet = Wallet::generate()?;
    let recipient = Address::from_hex(
        "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
    )?;

    let chain_id = provider.chain_id().await?;
    let nonce = provider.get_nonce(&wallet.address()).await?;
    let amount = parse_quanta("1.5").map_err(|e| anyhow::anyhow!(e))?;

    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .chain_id(chain_id)
        .nonce(nonce)
        .transfer(recipient, amount)
        .build()?;
    wallet.sign_tx(&mut tx).await?;
    let pending = provider.send_transaction(&tx).await?;
    let receipt = pending.wait_for_receipt().await?;
    println!("committed in wave {}", receipt.wave_id_u64());
    Ok(())
}
```

## What's in the box

| Module | What |
|---|---|
| [`types`](src/types) | `Address`, `TxHash`, `FalconPubkey`/`FalconSignature`, `Tx`, `TxType`, `FeePayer`, `AccessEntry`, `Receipt`, `Event`, `ContractAbi`, `ParamType`, `StateSchema` — all wire types Borsh-compatible with the engine |
| [`tx`](src/tx) | `TxBuilder` + `tx_hash` (Poseidon2 over the canonical pre-image, signature excluded) + Borsh `encode` / `decode` |
| [`signer`](src/signer) | `Signer` trait + `LocalSigner` (FALCON-512 keypair via `pyde-crypto`) |
| [`wallet`](src/wallet) | `Wallet` (implements `Signer`) + `Keystore` (argon2id + AES-256-GCM, JSON envelope, wire-compatible with `pyde-ts-sdk`) |
| [`provider`](src/provider) | `Provider` trait (23 methods) + `HttpProvider` (reqwest) + `PendingTx` |
| [`ws`](src/ws) | `WsProvider` + `Subscription<Event>` (v1 ships `subscribe_logs`; other kinds queued behind the engine) |
| [`abi`](src/abi) | `extract_abi(wasm)` — pulls the `pyde.abi` custom section from a contract's bytecode |
| [`contract`](src/contract) | Dynamic `Contract` runtime + `pyde_abi!` proc-macro for compile-time typed wrappers |
| [`util`](src/util) | hex helpers + PYDE↔quanta unit conversion |
| [`error`](src/error) | `SdkError` + `Result` |

## Typed contracts via `pyde_abi!`

```rust,ignore
use std::sync::Arc;
use pyde_rust_sdk::{Address, Provider, Wallet};

pyde_rust_sdk::pyde_abi!(Counter, "abi/counter.json");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let provider: Arc<dyn Provider> = /* HttpProvider */ todo!();
    let wallet = Wallet::generate()?;
    let counter = Counter::new(Address::ZERO, provider);

    let count: u64 = counter.get_count().await?;
    let pending = counter.add(&wallet, 5, 200_000, 0).await?;
    let _ = pending.wait_for_receipt().await?;
    Ok(())
}
```

VIEW functions become `async fn name(&self, args...) -> Result<RetType>`. Non-view functions become `async fn name(&self, signer, args..., gas_limit, value) -> Result<PendingTx>`. The ABI is baked in at compile time — no runtime fetch.

## Dynamic contracts

For tools that don't know the contract at compile time (explorers, indexers, multi-contract wallets):

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::contract::{Contract, Value};
use pyde_rust_sdk::Provider;

# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let contract = Contract::load("counter", provider).await?;
let count = contract.call("get_count", vec![]).await?;
println!("{count:?}");
# Ok(()) }
```

## Examples

| File | What |
|---|---|
| [`examples/wallet_basics.rs`](examples/wallet_basics.rs) | Generate a wallet, sign a hash, verify |
| [`examples/keystore.rs`](examples/keystore.rs) | Encrypted at-rest persistence + load |
| [`examples/transfer.rs`](examples/transfer.rs) | Sign + submit a PYDE transfer |
| [`examples/contract_dynamic.rs`](examples/contract_dynamic.rs) | Load a contract by name, dynamic call |
| [`examples/contract_typed.rs`](examples/contract_typed.rs) | Macro-generated typed wrapper |
| [`examples/subscribe_logs.rs`](examples/subscribe_logs.rs) | Open WS, stream event logs |
| [`examples/devnet_e2e.rs`](examples/devnet_e2e.rs) | Live devnet smoke test — chain info, transfer, deploy, view + send |
| [`examples/nft_marketplace.rs`](examples/nft_marketplace.rs) | Multi-account, multi-contract orchestration — ERC20 + ERC721 + atomic-swap marketplace |

Local examples (no node required):

```sh
cargo run --example wallet_basics
cargo run --example keystore
```

Network examples take `PYDE_RPC_URL` (and friends):

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example transfer
PYDE_RPC_URL=http://127.0.0.1:8545 PYDE_CONTRACT_NAME=counter cargo run --example contract_dynamic
PYDE_WS_URL=ws://127.0.0.1:8546 cargo run --example subscribe_logs
```

## Upstream notes

[`docs/upstream-asks.md`](docs/upstream-asks.md) tracks drift the
SDK has surfaced in the engine + otigen toolchain repos. SDK-side
workarounds are baked into the examples so live integration works
today; each item collapses out once the corresponding upstream
patch lands.

## Compatibility

- **Chain wire format**: every type the SDK puts on the wire is byte-for-byte identical to its counterpart in `engine/crates/types/`. Hash algorithm (`tx_hash`), Borsh field order, and `TxType` / `FeePayer` / `AuthKeys` tag values all match.
- **Keystore JSON**: compatible with `pyde-ts-sdk` — a wallet generated in the browser SDK loads here unchanged.
- **ABI schema**: `pyde.abi` custom section decoded up to `ContractAbi::V1_2`.

## Status

| Phase | Status |
|---|---|
| A1 — types, signer, wallet, canonical tx hash | ✓ |
| A2 — provider (23 RPC methods), WS subscriptions, PendingTx | ✓ |
| A3 — ABI parser, Contract runtime, `pyde_abi!` macro | ✓ |
| Examples + docs | ✓ |
| CI mirror | pending |
| Pre-mainnet spec audit pass | pending |

## License

Apache-2.0.
