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

## Documentation

Comprehensive docs live in [`docs/`](docs/README.md) — 14 chapters with detailed per-API references, examples, and expected output:

1. [Install](docs/01-install.md) — Cargo dep, MSRV, `otigen` install, system tooling
2. [Quickstart](docs/02-quickstart.md) — 5-minute end-to-end against a local devnet
3. [Concepts](docs/03-concepts.md) — FALCON, Poseidon2/Blake3, addresses, nonce window, units
4. [Wallets](docs/04-wallets.md) — `Wallet`, `LocalSigner`, `Keystore`, custom signers, zeroize
5. [Transactions](docs/05-transactions.md) — `TxBuilder`, `tx_hash`, signing, encoding, gas + fees
6. [Providers](docs/06-providers.md) — `HttpProvider`, `WsProvider`, every RPC method, `PendingTx`, retry policy
7. [Contracts](docs/07-contracts.md) — Deploy, `pyde_abi!` macro, dynamic `Contract`, `Value`, codec
8. [Events](docs/08-events.md) — `LogFilter`, `EventFilter`, cursor pagination, WS subscriptions
9. [Errors](docs/09-errors.md) — `SdkError`, `ErrorCode`, revert-reason decoding, structured `RevertCategory`, dapp UX
10. [Multisig](docs/10-multisig.md) — Treasury bundle, `canonical_msg`, `sign_action`, 2-of-3 walkthrough
11. [Examples](docs/11-examples.md) — Per-example walkthrough with expected output
12. [Compatibility](docs/12-compatibility.md) — Wire format guarantees, ABI versions, MSRV, TS SDK delta
13. [Utilities](docs/13-utilities.md) — Every helper in `crate::util` — hex, units, byte/slice
14. [Constants](docs/14-constants.md) — Every public constant — gas, keystore, codec caps, error codes, etc.

Changelog: [CHANGELOG.md](CHANGELOG.md).

## Install

```toml
[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
```

Full install + tooling instructions in [docs/01-install.md](docs/01-install.md).

## Quick start

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::util::parse_quanta;
use pyde_rust_sdk::{Address, Provider, Signer, TxBuilder, Wallet};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `otigen devnet` picks a random RPC port. Set PYDE_RPC_URL to
    // its advertised URL, or substitute it directly here.
    let url = std::env::var("PYDE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:9933".to_string());
    let transport = HttpTransport::new(&url)?;
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

Step-by-step explanation: [docs/02-quickstart.md](docs/02-quickstart.md).

## What's in the box

| Module | What |
|---|---|
| [`types`](src/types) | `Address`, `TxHash`, `FalconPubkey`/`FalconSignature`, `Tx`, `TxType`, `FeePayer`, `AccessEntry`, `Receipt`, `Event`, `ContractAbi`, `ParamType`, `StateSchema` — all wire types Borsh-compatible with the engine |
| [`tx`](src/tx) | `TxBuilder` + `tx_hash` (Poseidon2 over the canonical pre-image, signature excluded) + Borsh `encode` / `decode` |
| [`signer`](src/signer) | `Signer` trait + `LocalSigner` (FALCON-512 keypair via `pyde-crypto`) |
| [`wallet`](src/wallet) | `Wallet` (implements `Signer`) + `Keystore` (Argon2id + AES-256-GCM, SDK-specific format) |
| [`provider`](src/provider) | `Provider` trait (28 RPC methods) + `HttpProvider` (reqwest) + `PendingTx` |
| [`ws`](src/ws) | `WsProvider` + `Subscription<Event>` (v1 ships `subscribe_logs`; other kinds queued behind the engine) |
| [`abi`](src/abi) | `extract_abi(wasm)` — pulls the `pyde.abi` custom section from a contract's bytecode |
| [`contract`](src/contract) | Dynamic `Contract` runtime + `pyde_abi!` proc-macro for compile-time typed wrappers |
| [`util`](src/util) | hex helpers + PYDE↔quanta unit conversion |
| [`multisig`](src/multisig.rs) | Treasury `k-of-n` FALCON bundles — canonical message, `sign_action`, `MultisigTxPayload`, `TxBuilder::multisig_treasury_spend` |
| [`error`](src/error) | `SdkError` + `Result` |

## Examples

| File | What |
|---|---|
| [`examples/wallet_basics.rs`](examples/wallet_basics.rs) | Generate a wallet, sign a hash, verify |
| [`examples/keystore.rs`](examples/keystore.rs) | Encrypted at-rest persistence + load |
| [`examples/transfer.rs`](examples/transfer.rs) | Sign + submit a PYDE transfer |
| [`examples/private_transfer.rs`](examples/private_transfer.rs) | MEV-protected transfer via the private mempool (commit-reveal round-trip with `send_private`) |
| [`examples/contract_dynamic.rs`](examples/contract_dynamic.rs) | Load a contract by name, dynamic call |
| [`examples/contract_typed.rs`](examples/contract_typed.rs) | Macro-generated typed wrapper |
| [`examples/subscribe_logs.rs`](examples/subscribe_logs.rs) | Open WS, stream event logs |
| [`examples/devnet_e2e.rs`](examples/devnet_e2e.rs) | Live devnet smoke test — chain info, transfer, deploy, view + send |
| [`examples/nft_marketplace.rs`](examples/nft_marketplace.rs) | Multi-account, multi-contract orchestration — ERC20 + ERC721 + atomic-swap marketplace |
| [`examples/halt_methods.rs`](examples/halt_methods.rs) | Every Pyde halt mode + structured error parsing |
| [`examples/multisig_treasury.rs`](examples/multisig_treasury.rs) | 2-of-3 FALCON treasury spend |

Walkthroughs + run instructions: [docs/11-examples.md](docs/11-examples.md).

Local examples (no node required):

```sh
cargo run --example wallet_basics
cargo run --example keystore
cargo run --example multisig_treasury
```

Network examples take `PYDE_RPC_URL` (and friends). `otigen devnet`
picks a random RPC port each time it starts, so set the env var to
whatever URL the devnet logs on launch — for example:

```sh
PYDE_RPC_URL=http://127.0.0.1:<port> cargo run --example transfer
PYDE_RPC_URL=http://127.0.0.1:<port> PYDE_CONTRACT_NAME=counter cargo run --example contract_dynamic
PYDE_WS_URL=ws://127.0.0.1:<port>/ws cargo run --example subscribe_logs
```

`examples/transfer.rs` defaults to the prefunded `devnet-0` account.
Set `PYDE_SENDER_SEED=<32-byte hex>` to send from a different
wallet.

## Compatibility

- **Chain wire format**: every type the SDK puts on the wire is byte-for-byte identical to its counterpart in `engine/crates/types/`. Hash algorithm (`tx_hash`), Borsh field order, and `TxType` / `FeePayer` / `AuthKeys` tag values all match.
- **Keystore JSON**: SDK-specific (AES-256-GCM + nested envelope). Not interchangeable with `pyde-ts-sdk`'s keystore (which uses ChaCha20-Poly1305 + a flat envelope) — convergence is planned; see [docs/12-compatibility.md](docs/12-compatibility.md).
- **ABI schema**: `pyde.abi` custom section decoded up to `ContractAbi::V1_2`.

## License

Apache-2.0.
