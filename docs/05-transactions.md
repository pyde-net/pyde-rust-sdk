# 5. Transactions

[← back to TOC](README.md) · prev: [Wallets](04-wallets.md) · next: [Providers →](06-providers.md)

---

A transaction is a borsh-encoded `Tx` struct, FALCON-signed by
its sender, submitted over JSON-RPC. This chapter walks you
through every layer — the wire shape, the builder, signing, gas,
fees, and access lists.

## Table of contents

- [5.1 The `Tx` struct](#51-the-tx-struct)
- [5.2 `TxBuilder`](#52-txbuilder)
- [5.3 `TxBuilder` API reference](#53-txbuilder-api-reference)
- [5.4 Signing](#54-signing)
- [5.5 `tx::tx_hash` — canonical hash](#55-txtx_hash--canonical-hash)
- [5.6 Wire encoding (`encode` / `decode`)](#56-wire-encoding-encode--decode)
- [5.7 Submitting](#57-submitting)
- [5.8 Gas + fees](#58-gas--fees)
- [5.9 Fee payer alternatives](#59-fee-payer-alternatives)
- [5.10 Access lists](#510-access-lists)
- [5.11 Deadlines](#511-deadlines)
- [5.12 `TxType` reference](#512-txtype-reference)

---

## 5.1 The `Tx` struct

```rust,ignore
pub struct Tx {
    pub from:        Address,           // signer's address
    pub to:          Address,           // recipient (or ZERO for envelope-style)
    pub value:       u128,              // quanta to transfer
    pub data:        Vec<u8>,           // payload — calldata, deploy bytes, etc.
    pub gas_limit:   u64,
    pub nonce:       u64,
    pub signature:   FalconSignature,   // patched in by Signer::sign_tx
    pub fee_payer:   FeePayer,          // who pays — Sender, GasTank, Paymaster
    pub access_list: Vec<AccessEntry>,  // pre-declared state touches
    pub deadline:    Option<u64>,       // wave_id deadline, or None for no expiry
    pub chain_id:    u64,
    pub tx_type:     TxType,            // discriminant per Ch 11
}
```

Field order matches the engine's `borsh` derive byte-for-byte —
see [Compatibility §12.1](12-compatibility.md#121-wire-format-guarantees).

`data` meaning depends on `tx_type`:

| `tx_type` | `data` |
|---|---|
| `Standard` | Calldata for the function being called. Empty for plain PYDE transfers. |
| `Deploy` | Borsh-encoded `DeployData { name, wasm_bytes, contract_type, init_calldata }`. |
| `MultisigTx` | Borsh-encoded `MultisigTxPayload { target, amount, bundle }`. |
| `RegisterPubkey` | Borsh-encoded `FalconPubkey` (the new key being registered). |
| `StakeDeposit`, `StakeWithdraw`, etc. | Per-tx-type payload — see the chain spec. |

---

## 5.2 `TxBuilder`

Fluent + functional — every setter takes `self` by value and
returns `Self`. `build()` produces an unsigned `Tx`.

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::Address;

# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .chain_id(31337)
    .nonce(0)
    .gas_limit(100_000)
    .transfer(Address::ZERO, 1_500_000_000)     // 1.5 PYDE
    .build()?;
# Ok(()) }
```

### Convenience helpers (set multiple fields at once)

| Helper | Sets | Use case |
|---|---|---|
| `.transfer(to, value)` | `tx_type=Standard`, `to`, `value` | Plain PYDE send |
| `.deploy(name, wasm, kind, init)` | `tx_type=Deploy`, `to=ZERO`, `data=borsh(DeployData)` | New contract |
| `.call(contract, calldata)` | `tx_type=Standard`, `to=contract`, `data=calldata` | Contract method invocation |
| `.multisig_treasury_spend(target, amount, bundle)` | `tx_type=MultisigTx`, `to=ZERO`, `data=borsh(MultisigTxPayload)` | Treasury spend |

### Default field values

If you don't set a field, the builder uses:

| Field | Default |
|---|---|
| `to` | `Address::ZERO` |
| `value` | `0` |
| `data` | `Vec::new()` |
| `gas_limit` | `0` (the chain rejects with `out_of_gas` unless you set this) |
| `nonce` | `0` |
| `fee_payer` | `FeePayer::Sender` |
| `access_list` | `Vec::new()` |
| `deadline` | `None` |
| `chain_id` | `0` (almost always wrong — set it from `provider.chain_id()`) |
| `tx_type` | `TxType::Standard` |

### `from` is required

`build()` returns `SdkError::InvalidArgument` if `from` was
never set — the signing flow needs to know the sender address
to derive the right nonce window slot and to assert the FALCON
pubkey matches.

```rust,no_run
use pyde_rust_sdk::TxBuilder;

# fn run() {
let err = TxBuilder::new().build().unwrap_err();
println!("{err}");
# }
```

**Expected output:**
```
invalid argument: from address must be set on TxBuilder
```

---

## 5.3 `TxBuilder` API reference

### `TxBuilder::new()`

| | |
|---|---|
| Signature | `fn new() -> TxBuilder` |
| Returns | An empty builder with default field values (see above). |

### Setters (single-field)

| Method | Signature | What |
|---|---|---|
| `.from(addr)` | `fn from(self, Address) -> Self` | Sender address. **Required.** |
| `.to(addr)` | `fn to(self, Address) -> Self` | Recipient address. Default `Address::ZERO`. |
| `.value(quanta)` | `fn value(self, u128) -> Self` | PYDE value in quanta. Default `0`. |
| `.data(bytes)` | `fn data(self, Vec<u8>) -> Self` | Raw payload bytes. |
| `.gas_limit(gas)` | `fn gas_limit(self, u64) -> Self` | Max gas the tx may burn. |
| `.nonce(n)` | `fn nonce(self, u64) -> Self` | Tx nonce (must be in the account's window). |
| `.chain_id(id)` | `fn chain_id(self, u64) -> Self` | Chain id. **Required** for any non-test chain. |
| `.tx_type(ty)` | `fn tx_type(self, TxType) -> Self` | Tx discriminant. Default `Standard`. |
| `.fee_payer(p)` | `fn fee_payer(self, FeePayer) -> Self` | Who pays gas. Default `Sender`. |
| `.access_list(list)` | `fn access_list(self, Vec<AccessEntry>) -> Self` | Pre-declared state touches. |
| `.deadline(wave)` | `fn deadline(self, u64) -> Self` | Wave id by which the tx must commit. |
| `.clear_deadline()` | `fn clear_deadline(self) -> Self` | Remove a previously-set deadline. |

### Convenience helpers

#### `.transfer(to, value)`

| | |
|---|---|
| Signature | `fn transfer(self, to: Address, quanta: u128) -> Self` |
| Sets | `tx_type=Standard`, `to=to`, `value=quanta` |
| Use case | Simple PYDE send. |

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::Address;
# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(100_000)
    .transfer(Address::ZERO, 1_000_000_000)
    .build()?;
# Ok(()) }
```

#### `.deploy(name, wasm_bytes, contract_type, init_calldata)`

| | |
|---|---|
| Signature | `fn deploy(self, name: impl Into<String>, wasm_bytes: Vec<u8>, contract_type: ContractType, init_calldata: Vec<u8>) -> Result<Self, SdkError>` |
| Sets | `tx_type=Deploy`, `to=Address::ZERO`, `data=borsh(DeployData)` |
| Returns | `Result<Self>` because borsh encoding can theoretically fail. |
| Errors | `SdkError::Other` if borsh encoding fails (unreachable for fixed-shape inputs). |
| Use case | Deploy a new contract — the deployed address is `Address::from_contract_name(&name)`. |

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::{Address, ContractType};
# fn run() -> pyde_rust_sdk::Result<()> {
let wasm = std::fs::read("artifacts/counter.bundle/contract.wasm")?;
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(5_000_000)
    .deploy("counter".to_string(), wasm, ContractType::Contract, Vec::new())?
    .build()?;
# Ok(()) }
```

#### `.call(contract, calldata)`

| | |
|---|---|
| Signature | `fn call(self, contract: Address, calldata: Vec<u8>) -> Self` |
| Sets | `tx_type=Standard`, `to=contract`, `data=calldata` |
| Use case | Call a contract method. `calldata` is typically `selector ‖ borsh-encoded args`. |

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::Address;
# fn run() -> pyde_rust_sdk::Result<()> {
let calldata = vec![0xa7, 0xd4, 0xbd, 0x36];  // 4-byte selector for `get`
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(100_000)
    .call(Address::from_contract_name("counter"), calldata)
    .build()?;
# Ok(()) }
```

#### `.multisig_treasury_spend(target, amount, bundle)`

| | |
|---|---|
| Signature | `fn multisig_treasury_spend(self, target: Address, amount: u128, bundle: SigBundle) -> Result<Self, SdkError>` |
| Sets | `tx_type=MultisigTx`, `to=Address::ZERO`, `data=borsh(MultisigTxPayload)` |
| Errors | `SdkError::Other` on borsh encoding failure (unreachable). |
| Use case | Treasury spend authorised by `>= threshold` signers. See [Multisig §10](10-multisig.md). |

### Terminal — `.build()`

| | |
|---|---|
| Signature | `fn build(self) -> Result<Tx, SdkError>` |
| Returns | An unsigned `Tx` with `signature = FalconSignature::empty()`. |
| Errors | `SdkError::InvalidArgument` if `from` wasn't set. |

---

## 5.4 Signing

A `Signer` (typically `Wallet`) signs an unsigned `Tx` in place:

```rust,no_run
use pyde_rust_sdk::{Signer, TxBuilder, Wallet};
use pyde_rust_sdk::types::Address;

# async fn run() -> pyde_rust_sdk::Result<()> {
let wallet = Wallet::generate()?;
let mut tx = TxBuilder::new()
    .from(wallet.address())
    .chain_id(31337)
    .nonce(0)
    .gas_limit(100_000)
    .transfer(Address::ZERO, 1)
    .build()?;

wallet.sign_tx(&mut tx).await?;
assert!(!tx.signature.as_bytes().is_empty());
# Ok(()) }
```

`sign_tx` does three things:

1. Computes `tx_hash(&tx)` — canonical Poseidon2 pre-image
   excluding the `signature` field (otherwise the hash would
   depend on itself).
2. Calls `signer.sign_hash(&hash)`.
3. Patches the FALCON signature back into `tx.signature`.

If you set `from` to something other than the signer's address,
the chain rejects the tx on submission (`SdkError::Rpc` with a
"signature does not match account" message). Set `from =
signer.address()` always.

---

## 5.5 `tx::tx_hash` — canonical hash

The canonical pre-image is a borsh-encoded tuple of every Tx
field **except** `signature`, hashed under Poseidon2. The `data`
field is hashed first (so giant calldata doesn't bloat the outer
hash input).

```
preimage = borsh((from, to, value, Poseidon2(data),
                  gas_limit, nonce, fee_payer, access_list,
                  deadline, chain_id, tx_type))
tx_hash  = Poseidon2(preimage)
```

The chain computes the same hash on submission — any field drift
after signing invalidates the signature.

### Direct access

```rust,no_run
use pyde_rust_sdk::tx::tx_hash;
use pyde_rust_sdk::types::TxHash;
# use pyde_rust_sdk::types::Tx;
# fn run(tx: &Tx) {
let h: TxHash = tx_hash(tx);
println!("hash: {h}");
# }
```

**Example output:**
```
hash: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
```

Useful for:
- Computing the hash before signing (for caching or auditing).
- Custom signers that want to hash on the SDK side.
- Mempool inspectors / gateways that need to dedupe on canonical
  hash.

---

## 5.6 Wire encoding (`encode` / `decode`)

```rust,no_run
use pyde_rust_sdk::tx::{decode, encode};
# use pyde_rust_sdk::types::Tx;
# fn run(tx: &Tx) -> pyde_rust_sdk::Result<()> {
let bytes: Vec<u8> = encode(tx)?;
let parsed: Tx = decode(&bytes)?;
# Ok(()) }
```

### `tx::encode(&tx) -> Result<Vec<u8>>`

Thin wrapper around `borsh::to_vec(&tx)`. Returns
`SdkError::Other` on the (unreachable) case of borsh encoding
failure.

### `tx::decode(&[u8]) -> Result<Tx>`

Thin wrapper around `Tx::try_from_slice(bytes)`. Returns
`SdkError::InvalidArgument` if the bytes don't conform to the
canonical Borsh schema (e.g., truncated input, wrong field
order).

### Size limits

`tx::decode` itself doesn't cap — the contract-side `Value`
decoders separately cap variable-length sequences at
`MAX_DECODE_ELEMENTS = 1_000_000` to keep hostile RPC responses
from allocating gigabytes. See [Constants §14.5](14-constants.md#145-codec-caps).

---

## 5.7 Submitting

The `Provider` trait carries the raw RPC: `send_raw_transaction(&tx)`
returns a `TxHash`. The `RootProvider` wrapper adds
`send_transaction(&tx)` which gives you a `PendingTx` ready to
poll. See [Providers §6.7](06-providers.md#67-pendingtx).

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::{Provider, types::Tx};
# async fn run(provider: Arc<dyn Provider>, tx: &Tx) -> pyde_rust_sdk::Result<()> {
let hash = provider.send_raw_transaction(tx).await?;
println!("submitted: {hash}");
# Ok(()) }
```

**Expected output:**
```
submitted: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
```

---

## 5.8 Gas + fees

Pyde meters gas exactly like Ethereum at the unit level — a `u64`
gas counter, per-host-fn gas charges, transfers cost `21_000`,
deploys cost more, contract calls bill per executed host fn.

Reference constants in [`src/constants.rs`](../src/constants.rs):

| Constant | Value | What |
|---|---|---|
| `GAS_TRANSFER` | 100_000 | Simple PYDE send (above `MIN_GAS_LIMIT = 21_000` for hashing + sig-verify headroom). |
| `GAS_TOKEN_CALL` | 500_000 | Standard fungible-token method (transfer, approve, transfer_from). |
| `GAS_NFT_CALL` | 1_000_000 | Standard NFT method (mint, transfer_from, approve, set_approval_for_all). |
| `GAS_DEPLOY` | 10_000_000 | Headroom for `Deploy` — actual usage usually smaller. |
| `GAS_CROSS_CALL_ORCHESTRATOR` | 2_000_000 | Outer wrapper for a contract that internally calls another contract. |

These are SDK-side recommended floors — the actual gas the chain
charges is reported in `Receipt.gas_used`. **There are no gas
refunds in v1** — `gas_used` is what you pay.

`gas_limit` is what you authorise. If the tx burns less, you pay
`gas_used × gas_price`; if it burns more, the chain reverts with
`out_of_gas`.

### Estimating before submission

Use `provider.simulate_transaction(&tx)` to get an estimate
without committing the tx. See [Providers §6.5](06-providers.md#65-simulating).

---

## 5.9 Fee payer alternatives

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

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::{Address, FeePayer};

# fn run() -> pyde_rust_sdk::Result<()> {
// Have the gas tank pay (typical for first-time RegisterPubkey).
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(50_000)
    .fee_payer(FeePayer::GasTank)
    .transfer(Address::ZERO, 0)
    .build()?;
# Ok(()) }
```

---

## 5.10 Access lists

If your tx touches a known set of accounts, declaring them
up-front lets the chain parallelise execution (a `Read` entry
doesn't block other readers). The list is **hint-only** —
declaring nothing is fine, just slower in waves with high
contention.

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::{AccessEntry, AccessType, Address};

# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(200_000)
    .access_list(vec![
        AccessEntry {
            address: Address::ZERO,
            storage_keys: vec![],
            access_type: AccessType::ReadWrite,
        },
    ])
    .transfer(Address::ZERO, 1)
    .build()?;
# Ok(()) }
```

| `AccessType` | Tag | Meaning |
|---|---|---|
| `Read` | `0x00` | Read-only — multiple txs can read the same address in parallel. |
| `ReadWrite` | `0x01` | Will mutate — the scheduler serialises competing writers. |

### Getting the right access list

Use `provider.simulate_transaction(&tx).await?.access_list` to
get the observed access pattern from a dry run. Feed it back
into the real submission's `access_list` to get the parallelism
benefit.

---

## 5.11 Deadlines

`.deadline(wave_id)` sets a wave-ID cap; if the tx hasn't
committed by that wave, the mempool drops it and your
`PendingTx::wait_for_receipt` returns an error.

```rust,no_run
use pyde_rust_sdk::TxBuilder;
use pyde_rust_sdk::types::Address;
# async fn run(current_wave: u64) -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .chain_id(31337)
    .nonce(0)
    .gas_limit(100_000)
    .deadline(current_wave + 50)        // tolerate ~50 waves of latency
    .transfer(Address::ZERO, 1)
    .build()?;
# Ok(()) }
```

`.clear_deadline()` removes a previously-set deadline if you
need to keep the builder around and change your mind.

When to set:

| Scenario | Deadline |
|---|---|
| Time-sensitive trade (price quote about to expire) | Yes — set a tight deadline. |
| Background batch submission | Optional — none means "wait forever in mempool". |
| Re-submission flow after a failure | Yes — bound how long the retry might live. |

---

## 5.12 `TxType` reference

All 16 v1 transaction types, with their wire tags and where in
the SDK to find the helper:

| Tag | `TxType` | Helper | Notes |
|---|---|---|---|
| `0x00` | `Standard` | `.transfer()`, `.call()` | Plain transfer or contract call. |
| `0x01` | `Deploy` | `.deploy(name, wasm, kind, init)` | Deploy a contract. |
| `0x03` | `StakeDeposit` | (no helper — raw setters) | Lock PYDE for validator stake. |
| `0x04` | `StakeWithdraw` | — | Begin unbonding. |
| `0x05` | `Slash` | — | Slash a validator (chain-internal in v1). |
| `0x06` | `ClaimReward` | — | Claim staking rewards. |
| `0x07` | `ClaimAirdrop` | — | Claim an airdrop entitlement. |
| `0x08` | `SweepAirdrop` | — | Sweep unclaimed airdrops back to treasury. |
| `0x09` | `MultisigTx` | `.multisig_treasury_spend()` | Treasury spend authorised by k-of-n bundle. |
| `0x0A` | `RotateMultisig` | — (multisig primitive) | Rotate treasury signer set. |
| `0x0B` | `EmergencyPause` | — (multisig primitive) | Halt wave production. |
| `0x0C` | `EmergencyResume` | — (multisig primitive) | Lift an emergency pause. |
| `0x0D` | `RegisterPubkey` | — | First-time pubkey registration for a funded-but-unregistered account. |
| `0x0E` | `Unjail` | — | Release a validator from `Jailed` back to `Active`. |
| `0x0F` | `RotateValidatorKeys` | — | Rotate the validator signing key. Active validators only. |
| `0x10` | `DisputeSlash` | — (multisig primitive) | Apply a slashing dispute resolution. |

Tag `0x02` is reserved (gap left for a future tx type that
needed alignment with the chain spec).

For tx types without a dedicated helper, build raw:

```rust,no_run
# use pyde_rust_sdk::TxBuilder;
# use pyde_rust_sdk::types::{Address, TxType};
# fn run() -> pyde_rust_sdk::Result<()> {
let tx = TxBuilder::new()
    .from(Address::ZERO).chain_id(31337).nonce(0).gas_limit(100_000)
    .tx_type(TxType::RegisterPubkey)
    .data(/* borsh-encoded payload per the chain spec */ vec![])
    .build()?;
# Ok(()) }
```
