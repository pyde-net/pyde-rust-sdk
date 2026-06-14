# 5. Transactions

[← back to TOC](README.md) · prev: [Wallets](04-wallets.md) · next: [Providers →](06-providers.md)

---

A transaction is a borsh-encoded `Tx` struct, FALCON-signed by
its sender, submitted over JSON-RPC. The SDK gives you a builder
to assemble it without remembering the field order.

## The `Tx` struct

```rust,ignore
pub struct Tx {
    pub from:        Address,           // signer's address
    pub to:          Address,           // recipient (or ZERO for envelope-style)
    pub value:       u128,              // quanta to transfer
    pub data:        Vec<u8>,           // payload — calldata, deploy bytes, etc.
    pub gas_limit:   u64,
    pub nonce:       u64,
    pub signature:   FalconSignature,   // patched in by Signer.sign_tx
    pub fee_payer:   FeePayer,          // who pays — Sender, GasTank, Paymaster
    pub access_list: Vec<AccessEntry>,  // pre-declared state touches
    pub deadline:    Option<u64>,       // wave_id deadline, or None for no expiry
    pub chain_id:    u64,
    pub tx_type:     TxType,            // discriminant per Ch 11
}
```

Field order matches the engine's `borsh` derive byte-for-byte —
see [Compatibility](12-compatibility.md#tx-wire-format).

## `TxBuilder`

Build is functional + fluent — every setter takes self by value
and returns self. Build terminates with `.build()` (returns
`Result<Tx>`).

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::{Address, FeePayer, TxType};

# fn run() -> pyde_rust_sdk::Result<()> {
let sender = Address::ZERO;
let recipient = Address::ZERO;

let tx = TxBuilder::new()
    .from(sender)
    .chain_id(31337)
    .nonce(0)
    .gas_limit(100_000)
    .transfer(recipient, 1_500_000_000)     // 1.5 PYDE
    .build()?;
# Ok(()) }
```

### Convenience helpers

| Helper | Sets | Use case |
|---|---|---|
| `.transfer(to, value)` | `tx_type=Standard`, `to`, `value` | Plain PYDE send |
| `.deploy(name, wasm, kind, init)` | `tx_type=Deploy`, `to=ZERO`, `data=borsh(DeployData)` | New contract |
| `.call(contract, calldata)` | `tx_type=Standard`, `to=contract`, `data=calldata` | Contract method invocation |
| `.multisig_treasury_spend(target, amount, bundle)` | `tx_type=MultisigTx`, `to=ZERO`, `data=borsh(MultisigTxPayload)` | Treasury spend |

Without a helper you can drive everything raw:

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::{Address, AccessEntry, AccessType, FeePayer, TxType};
# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .to(Address::ZERO)
    .value(0)
    .data(vec![1, 2, 3])
    .gas_limit(50_000)
    .nonce(7)
    .chain_id(31337)
    .tx_type(TxType::Standard)
    .fee_payer(FeePayer::Sender)
    .access_list(vec![AccessEntry { address: Address::ZERO, storage_keys: vec![], access_type: AccessType::ReadWrite }])
    .deadline(1234)
    .build()?;
# Ok(()) }
```

### `from` is required

`build()` returns `SdkError::InvalidArgument` if `from` was never
set. Everything else has a sensible default (`tx_type` =
`Standard`, `fee_payer` = `Sender`, `value` = 0, etc.).

## Signing

A `Signer` (typically `Wallet`) signs an unsigned `Tx` in place:

```rust,no_run
# use pyde_rust_sdk::{Signer, TxBuilder, Wallet};
# use pyde_rust_sdk::types::Address;
# async fn run() -> pyde_rust_sdk::Result<()> {
let wallet = Wallet::generate()?;
let mut tx = TxBuilder::new()
    .from(wallet.address())
    .chain_id(31337)
    .nonce(0)
    .transfer(Address::ZERO, 1)
    .build()?;

wallet.sign_tx(&mut tx).await?;
assert!(!tx.signature.as_bytes().is_empty());
# Ok(()) }
```

`sign_tx` does three things:
1. Computes `tx_hash(&tx)` — canonical Poseidon2 pre-image. The
   signature field is **excluded** from the hash (otherwise
   you'd have a chicken-and-egg problem).
2. Calls `signer.sign_hash(&hash)`.
3. Patches `tx.signature` with the returned bytes.

### What goes into `tx_hash`

The canonical pre-image is a borsh-encoded tuple of every Tx
field **except** `signature`, hashed under Poseidon2 (with one
inner Poseidon2 pass on `data` so very long calldata doesn't
bloat the outer hash input).

```
preimage = borsh((from, to, value, Poseidon2(data),
                  gas_limit, nonce, fee_payer, access_list,
                  deadline, chain_id, tx_type))
tx_hash  = Poseidon2(preimage)
```

The same hash is computed by the chain on submission — any field
drift after signing invalidates the signature.

### Direct access to `tx_hash`

For systems that want to compute the canonical hash without
signing (gateways, mempool inspectors, custom signers):

```rust,no_run
use pyde_rust_sdk::tx::tx_hash;
# use pyde_rust_sdk::types::Tx;
# fn run(tx: &Tx) {
let h = tx_hash(tx);
println!("hash: {h}");
# }
```

## Wire encoding

`borsh::to_vec(&tx)` works. The SDK ships wrappers that translate
borsh errors into `SdkError`:

```rust,no_run
use pyde_rust_sdk::tx::{decode, encode};
# use pyde_rust_sdk::types::Tx;
# fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
let bytes: Vec<u8> = encode(tx)?;
let parsed: Tx = decode(&bytes)?;
# Ok(()) }
```

`decode` is a thin borsh wrapper that translates parse errors
into `SdkError::InvalidArgument`. The contract-side `Value`
decoders separately cap variable-length sequences at
`MAX_DECODE_ELEMENTS = 1_000_000` (see
[`src/contract/codec.rs`](../src/contract/codec.rs)).

## Submitting

The Provider trait carries the raw RPC: `send_raw_transaction(&tx)`
returns a `TxHash`. The `RootProvider` wrapper adds
`send_transaction(&tx)` which gives you a `PendingTx` ready to
poll. See [Providers](06-providers.md#pending-transactions).

## Gas + fees

Pyde meters gas exactly like Ethereum at the unit level — a `u64`
gas counter, per-host-fn gas charges, transfers cost `21_000`,
deploys cost more, contract calls bill per executed host fn.

Reference constants in [`src/constants.rs`](../src/constants.rs):

| Constant | Value | What |
|---|---|---|
| `GAS_TRANSFER` | 21_000 | Simple PYDE send |
| `GAS_ERC20_CALL` | 100_000 | Standard ERC20 method (transfer, approve, balanceOf) |
| `GAS_ERC721_CALL` | 150_000 | Standard ERC721 method |
| `GAS_DEPLOY` | 5_000_000 | Headroom for `Deploy` — actual usage smaller |
| `GAS_CROSS_CALL_ORCHESTRATOR` | 500_000 | Outer wrapper for a contract that internally calls another contract |

These are SDK-side recommended floors — the actual gas the chain
charges is reported in `Receipt.gas_used`. **There are no gas
refunds in v1** — `gas_used` is what you pay.

`gas_limit` is what you authorise. If the tx burns less, you pay
`gas_used × gas_price`; if it burns more, the chain reverts with
`out_of_gas`.

## Fee payer alternatives

Three options:

```rust,ignore
pub enum FeePayer {
    Sender = 0x00,                  // tx.from pays — the default
    GasTank = 0x01,                 // protocol gas-tank pays (paymaster-style)
    Paymaster(Address) = 0x02,      // a designated paymaster account pays
}
```

| Variant | When |
|---|---|
| `Sender` | Normal case. The signer's address gets debited gas. |
| `GasTank` | Protocol-sponsored — used by `RegisterPubkey` so a fresh user can bootstrap without holding PYDE. |
| `Paymaster(addr)` | Custom — the named account must have authorised the sponsorship, otherwise the chain rejects. v1 has no SDK-side paymaster authorisation helper; ship the agreement off-chain. |

## Access lists

If your tx touches a known set of accounts, declaring them
up-front lets the chain parallelise execution (a `Read` entry
doesn't block other readers). The list is hint-only — declaring
nothing is fine, just slower.

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::{AccessEntry, AccessType, Address};

# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .chain_id(31337)
    .nonce(0)
    .gas_limit(200_000)
    .transfer(Address::ZERO, 1)
    .access_list(vec![
        AccessEntry { address: Address::ZERO, storage_keys: vec![], access_type: AccessType::ReadWrite },  // sender + recipient
    ])
    .build()?;
# Ok(()) }
```

## Deadlines

`.deadline(wave_id)` sets a wave-ID cap; if the tx hasn't
committed by that wave, the mempool drops it and your
`PendingTx::wait_for_receipt` returns an error.

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::Address;
# async fn run(current_wave: u64) -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .chain_id(31337)
    .nonce(0)
    .deadline(current_wave + 50)        // tolerate ~50 waves of latency
    .transfer(Address::ZERO, 1)
    .build()?;
# Ok(()) }
```

`.clear_deadline()` removes a previously-set deadline.
