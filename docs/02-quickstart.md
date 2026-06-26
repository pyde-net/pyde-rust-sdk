# 2. Quickstart

[← back to TOC](README.md) · prev: [Install](01-install.md) · next: [Concepts →](03-concepts.md)

---

End-to-end in five minutes: start a local devnet, generate a
wallet, sign a transfer, wait for the receipt.

## Table of contents

- [2.1 Start the devnet](#21-start-the-devnet)
- [2.2 Create a new cargo project](#22-create-a-new-cargo-project)
- [2.3 The full program](#23-the-full-program)
- [2.4 Run](#24-run)
- [2.5 What just happened — step-by-step](#25-what-just-happened--step-by-step)
- [2.6 Common errors during the first run](#26-common-errors-during-the-first-run)
- [2.7 Next steps](#27-next-steps)

---

## 2.1 Start the devnet

In a terminal with `otigen` on your `PATH`
(see [Install §1.4](01-install.md#14-installing-otigen-devnet--contracts)):

```sh
otigen devnet --rpc-listen 127.0.0.1:9933 --prefund-count 5
```

**Expected banner (truncated):**

```
═══════════════════════════════════════════════════════════════
  Pyde devnet — single-validator, instant-wave
═══════════════════════════════════════════════════════════════

Chain id:         31337
Prefund count:    5
Prefund balance:  10000000000 quanta per account

Prefunded accounts (deterministic from canonical seed):

  (0) address: 0xf07856fdf4796baa6d477ddfe926774d367b25c20e8c7d9d337b63034c9e0cfa
      secret:  0x7720ecbbdff51d016964b92cd9d6c8082078adb55730802b796fb8e199e77991
      ...
```

Note the **deterministic** prefunded accounts: each one's secret
is derived from `Blake3("pyde-devnet-v1/" || i.to_le_bytes())`,
so the same banner appears on every fresh start. We'll use
`devnet-0` (index 0) as our sender.

Leave this terminal running; open a second terminal for the
rest.

---

## 2.2 Create a new cargo project

```sh
cargo new pyde-quickstart
cd pyde-quickstart
```

`Cargo.toml`:

```toml
[package]
name = "pyde-quickstart"
version = "0.1.0"
edition = "2021"

[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
blake3 = "1"          # only needed to re-derive the devnet banner's seeds
```

| Dep | Why |
|---|---|
| `pyde-rust-sdk` | The SDK itself. |
| `tokio` | The async runtime — every network call returns a Future. |
| `anyhow` | Convenience error type for `main()`. |
| `blake3` | Re-derives the devnet's deterministic prefund seeds locally. Not needed in a real dapp. |

---

## 2.3 The full program

`src/main.rs`:

```rust,no_run
use std::sync::Arc;
use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
use pyde_rust_sdk::types::Address;
use pyde_rust_sdk::util::parse_quanta;
use pyde_rust_sdk::{Provider, Signer, TxBuilder, Wallet};

/// The devnet's deterministic pre-fund seed scheme.
/// Re-derives the same wallet the devnet banner enumerates.
fn devnet_seed(i: u64) -> [u8; 32] {
    let mut input = Vec::with_capacity(b"pyde-devnet-v1/".len() + 8);
    input.extend_from_slice(b"pyde-devnet-v1/");
    input.extend_from_slice(&i.to_le_bytes());
    *blake3::hash(&input).as_bytes()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── 1. Connect ──────────────────────────────────────────────────
    let transport = HttpTransport::new("http://127.0.0.1:9933")?;
    let provider = Arc::new(RootProvider::new(transport));
    let chain_id = provider.chain_id().await?;
    println!("connected: chain_id = {chain_id}");

    // ── 2. Reproduce devnet-0 (the first pre-funded account) ────────
    let sender = Wallet::from_seed(&devnet_seed(0))?;
    println!("sender: {}", sender.address());

    let bal = provider.get_balance(&sender.address()).await?;
    println!("balance: {bal} quanta");

    // ── 3. Build, sign, submit a transfer ───────────────────────────
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

    // ── 4. Wait for the receipt ─────────────────────────────────────
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

---

## 2.4 Run

```sh
cargo run
```

**Expected output:**

```
connected: chain_id = 31337
sender: 0xf07856fdf4796baa6d477ddfe926774d367b25c20e8c7d9d337b63034c9e0cfa
balance: 10000000000 quanta
submitted: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
committed in wave 12 / tx #0 / status Success
```

(The tx hash and wave number will differ on your run; the address
and balance will match, since the devnet seed scheme is
deterministic.)

That's it. You've put bytes on chain.

---

## 2.5 What just happened — step-by-step

### Step 1 — Connect

```rust,no_run
# use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
# use std::sync::Arc;
# fn run() -> pyde_rust_sdk::Result<()> {
let transport = HttpTransport::new("http://127.0.0.1:9933")?;
let provider = Arc::new(RootProvider::new(transport));
# Ok(()) }
```

`HttpTransport::new` builds a reqwest client with `rustls` TLS
(no system OpenSSL required) and validates the URL up-front.
`RootProvider::new(transport)` wraps the transport to expose the
26-method `Provider` trait. We wrap in `Arc` so the provider can
be shared across tasks cheaply.

→ See [Providers §6.1](06-providers.md#61-transports).

### Step 2 — Reproduce the devnet wallet

```rust,no_run
# use pyde_rust_sdk::Wallet;
# fn devnet_seed(_: u64) -> [u8; 32] { [0u8; 32] }
# fn run() -> pyde_rust_sdk::Result<()> {
let sender = Wallet::from_seed(&devnet_seed(0))?;
# Ok(()) }
```

`devnet_seed(0)` computes `Blake3("pyde-devnet-v1/" || 0u64.to_le_bytes())`
— exactly the same scheme the devnet runner uses to derive its
prefund accounts. `Wallet::from_seed(seed)` deterministically
generates a FALCON-512 keypair from those 32 bytes.

The same seed → same keypair → same address every time, so the
address matches the one in the devnet banner. In a real dapp
you'd use `Wallet::generate()` (OS entropy) instead.

→ See [Wallets §4.2](04-wallets.md#42-wallet--high-level-keypair).

### Step 3 — Build + sign + submit

```rust,no_run
# use pyde_rust_sdk::{Signer, TxBuilder, Wallet};
# use pyde_rust_sdk::types::Address;
# async fn run(sender: Wallet, recipient: Address, amount: u128, chain_id: u64, nonce: u64) -> pyde_rust_sdk::Result<()> {
let mut tx = TxBuilder::new()
    .from(sender.address())
    .chain_id(chain_id)
    .nonce(nonce)
    .gas_limit(100_000)
    .transfer(recipient, amount)
    .build()?;
sender.sign_tx(&mut tx).await?;
# Ok(()) }
```

`TxBuilder` is fluent + functional — every setter takes `self`
by value and returns it. `.transfer(recipient, amount)` is a
convenience that sets `tx_type = Standard`, `to = recipient`,
`value = amount` in one call.

`.build()` produces an unsigned `Tx`. `sender.sign_tx(&mut tx)`
then:
1. Computes the canonical Poseidon2 pre-image (`tx_hash`,
   excluding the signature field).
2. Calls `signer.sign_hash(&hash)` → FALCON-512 signature.
3. Patches the result into `tx.signature`.

→ See [Transactions §5.2](05-transactions.md#52-txbuilder), [§5.4](05-transactions.md#54-signing).

### Step 4 — Wait for the receipt

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::{Provider, types::Tx};
# async fn run(provider: Arc<dyn Provider>, tx: &Tx) -> pyde_rust_sdk::Result<()> {
let pending = provider.send_transaction(tx).await?;
let receipt = pending.wait_for_receipt().await?;
# Ok(()) }
```

`send_transaction` does two things: submits the tx via
`pyde_sendRawTransaction` and returns a `PendingTx` ready to
poll. `wait_for_receipt` then polls `pyde_getReceipt` every
1 s (default) until a receipt lands, with a 30-second timeout
(default; tunable via `.with_poll_interval` / `.with_timeout`).

The returned `Receipt` carries `wave_id`, `tx_index`, `status`,
`gas_used`, plus any events the tx emitted.

→ See [Providers §6.7](06-providers.md#67-pendingtx).

---

## 2.6 Common errors during the first run

### `Connection refused`

The devnet isn't running, or it's bound to a different address.
`otigen devnet` picks a random RPC port unless you pass
`--rpc-listen` explicitly — match the SDK target to whatever URL
the devnet startup banner advertised, or pin it with
`--rpc-listen 127.0.0.1:9933` and use that.

The SDK retries transient connection failures automatically by
default — 3 retries with exponential backoff. If you see
`Connection refused` after retries exhaust, the devnet really
isn't listening.

### `expected 10 PYDE pre-fund; got X`

Old engine binary — the help text + actual default were
inconsistent in early builds. Newer engines ship 10 PYDE
consistently. Rebuild your `otigen` binary.

### `InvalidArgument: nonce too low`

You re-ran the program without restarting the devnet, so your
account's on-chain nonce advanced. The program reads
`provider.get_nonce(...)` afresh each run, so this shouldn't
happen — but if you cached the nonce somewhere and re-used it,
you'll see this error. Get a fresh nonce per submission.

### `Signing: FALCON sign failed`

`pyde-crypto` failed to fetch (private repo) or your `rustc` is
too old. See [Install §1.7](01-install.md#17-troubleshooting).

---

## 2.7 Next steps

- **Encrypt the wallet at rest** — [Wallets §4.4 Keystore](04-wallets.md#44-encrypted-keystore-on-disk).
- **Deploy a contract** — [Contracts §7.1 Deployment](07-contracts.md#71-deployment).
- **Subscribe to events live** — [Events §8.3 Live subscriptions](08-events.md#83-live-subscriptions).
- **Decode revert reasons** — [Errors §9.3 HOST_FN_ABI codes](09-errors.md#93-host_fn_abi-4-error-codes).
- **Authorise a treasury spend** — [Multisig §10.4 End-to-end](10-multisig.md#104-treasury-spend--end-to-end).
- **Read all the examples** — [Examples §11](11-examples.md).
