# 2. Quickstart

[← back to TOC](README.md) · prev: [Install](01-install.md) · next: [Concepts →](03-concepts.md)

---

End-to-end in five minutes: generate a wallet, sign a transfer
against a local devnet, wait for the receipt.

## 0. Start the devnet

In one terminal, with the `pyde` binary on `PATH` (see
[Install](01-install.md#optional-pyde-devnet-binary)):

```sh
pyde devnet --rpc-listen 127.0.0.1:8545 --prefund-count 5
```

The banner enumerates 5 pre-funded accounts. Note the **secret**
on `devnet-0` — we'll use it as our sender so the transfer
actually has funds to spend.

```
(0) address: 0xf07856fd…
    secret:  0x7720ecbbdff51d016964b92cd9d6c8082078adb55730802b796fb8e199e77991
```

## 1. New cargo project

```sh
cargo new pyde-quickstart
cd pyde-quickstart
```

`Cargo.toml`:

```toml
[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
blake3 = "1"          # only needed to re-derive the devnet banner's seeds
```

## 2. The full program

`src/main.rs`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::Address;
use pyde_rust_sdk::util::parse_quanta;
use pyde_rust_sdk::{Provider, Signer, TxBuilder, Wallet};

/// The devnet's deterministic pre-fund seed scheme — matches
/// `engine/crates/node/src/devnet/runner.rs::devnet_secret`.
/// Re-derives the same wallet the devnet banner enumerates.
fn devnet_seed(i: u64) -> [u8; 32] {
    let mut input = Vec::with_capacity(b"pyde-devnet-v1/".len() + 8);
    input.extend_from_slice(b"pyde-devnet-v1/");
    input.extend_from_slice(&i.to_le_bytes());
    *blake3::hash(&input).as_bytes()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Connect ──
    let transport = HttpTransport::new("http://127.0.0.1:8545")?;
    let provider = Arc::new(RootProvider::new(transport));
    let chain_id = provider.chain_id().await?;
    println!("connected: chain_id = {chain_id}");

    // ── Reproduce devnet-0 (the first pre-funded account) ──
    let sender = Wallet::from_seed(&devnet_seed(0))?;
    println!("sender: {}", sender.address());

    let bal = provider.get_balance(&sender.address()).await?;
    println!("balance: {} quanta", bal);

    // ── Build, sign, submit a transfer ──
    let recipient = Address::from_hex(
        "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
    )?;
    let amount = parse_quanta("1.5").map_err(|e| anyhow::anyhow!(e))?;
    let nonce = provider.get_nonce(&sender.address()).await?;

    let mut tx = TxBuilder::new()
        .from(sender.address())
        .chain_id(chain_id)
        .nonce(nonce)
        .gas_limit(100_000)
        .transfer(recipient, amount)
        .build()?;
    sender.sign_tx(&mut tx).await?;

    let pending = provider.send_transaction(&tx).await?;
    println!("submitted: {}", pending.hash());

    // ── Wait for the receipt ──
    let receipt = pending.wait_for_receipt().await?;
    println!(
        "committed in wave {} / tx #{} / status {:?}",
        receipt.wave_id_u64(),
        receipt.tx_index_u32(),
        receipt.status
    );

    Ok(())
}
```

## 3. Run

```sh
cargo run
```

```
connected: chain_id = 31337
sender: 0xf07856fdf4796baa6d477ddfe926774d367b25c20e8c7d9d337b63034c9e0cfa
balance: 10000000000 quanta
submitted: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
committed in wave 12 / tx #0 / status Success
```

That's it. You've put bytes on chain.

## What just happened

| Step | What | Reference |
|---|---|---|
| `HttpTransport::new` | TLS-rustls reqwest client wrapped against the RPC URL | [Providers](06-providers.md) |
| `Wallet::from_seed(seed)` | Deterministic FALCON-512 keypair from a 32-byte seed; address derived from the pubkey via Poseidon2 | [Wallets](04-wallets.md), [Concepts](03-concepts.md#addresses) |
| `parse_quanta("1.5")` | Converts the human-readable PYDE string to `u128` quanta (`PYDE_DECIMALS = 9`) | [Concepts](03-concepts.md#units) |
| `TxBuilder::new().transfer(…)` | Builds an unsigned `Tx` with `tx_type = Standard`, `to = recipient`, `value = amount` | [Transactions](05-transactions.md) |
| `sender.sign_tx(&mut tx)` | Computes the canonical Poseidon2 pre-image, FALCON-signs it, patches `tx.signature` | [Transactions](05-transactions.md#signing) |
| `provider.send_transaction` | Submits over JSON-RPC; returns a `PendingTx` that polls until the receipt lands | [Providers](06-providers.md#pending-transactions) |
| `pending.wait_for_receipt` | Polls `pyde_getReceipt` until the tx is in a committed wave; cross-checks the returned hash | [Providers](06-providers.md#pending-transactions) |

## Next steps

- **Deploy a contract** → [Contracts](07-contracts.md)
- **Subscribe to events** → [Events](08-events.md)
- **Decode revert reasons** → [Errors](09-errors.md)
- **Build a treasury spend** → [Multisig](10-multisig.md)
