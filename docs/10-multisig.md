# 10. Multisig

[← back to TOC](README.md) · prev: [Errors](09-errors.md) · next: [Examples →](11-examples.md)

---

Pyde's chain has a single on-chain **treasury** account guarded
by a `k-of-n` FALCON-512 multisig (Ch 11 §11.7). Five tx types
authenticate against it:

| TxType            | Action                                  | Domain byte |
|-------------------|-----------------------------------------|-------------|
| `MultisigTx`      | Treasury spend (debit treasury, credit target) | `0x09` |
| `RotateMultisig`  | Change the signer set + threshold       | `0x0A` |
| `EmergencyPause`  | Halt wave production                    | `0x0B` |
| `EmergencyResume` | Lift an emergency pause                 | `0x0C` |
| `DisputeSlash`    | Apply a slashing dispute resolution     | `0x10` |

All five share one signing primitive — each signer signs the
same canonical message under their FALCON key; the chain verifies
≥ `threshold` distinct valid signatures from the on-chain signer
set before applying the action.

## Table of contents

- [10.1 The canonical message](#101-the-canonical-message)
- [10.2 Bundle wire shape](#102-bundle-wire-shape)
- [10.3 `MultisigTxPayload`](#103-multisigtxpayload)
- [10.4 Treasury spend — end-to-end](#104-treasury-spend--end-to-end)
- [10.5 `multisig` module API reference](#105-multisig-module-api-reference)
- [10.6 Reading on-chain multisig state](#106-reading-on-chain-multisig-state)
- [10.7 Domain byte constants](#107-domain-byte-constants)
- [10.8 What the SDK doesn't do (yet)](#108-what-the-sdk-doesnt-do-yet)

---

## 10.1 The canonical message

```
canonical_msg = Poseidon2(domain_byte || nonce_le || Poseidon2(payload))
```

- **`domain_byte`** — per-action separator. A signature for
  `MultisigTx` can never be lifted into a `RotateMultisig` at
  the same nonce.
- **`nonce`** — the on-chain `MultisigState.nonce`, bumped on
  every successful action. Replay protection.
- **`payload`** — per-action body (e.g. borsh `(target, amount)`
  for `MultisigTx`; empty for `EmergencyPause`).

The SDK exposes this directly:

```rust,no_run
use pyde_rust_sdk::multisig::canonical_msg;
use pyde_rust_sdk::types::TxType;

let msg: Option<[u8; 32]> = canonical_msg(
    TxType::MultisigTx,
    42,                 // current MultisigState.nonce
    b"target||amount",  // borsh-encoded payload
);
assert!(msg.is_some());
```

Returns `None` if the `tx_type` isn't multisig-driven (e.g.,
`TxType::Standard`).

---

## 10.2 Bundle wire shape

```rust,ignore
pub struct BundleEntry {
    pub signer_index: u32,
    pub signature: FalconSignature,    // FALCON-512 over canonical_msg
}
pub type SigBundle = Vec<BundleEntry>;
```

`signer_index` is the **0-based position** of the signing pubkey
in `MultisigState.signers`. The chain rejects:

- Out-of-range indices.
- Duplicate `signer_index` within a single bundle.
- Bundles with fewer than `threshold` valid entries.

Bundles are NOT sorted by `signer_index` — preserve the order
the signers submitted in. The chain handles dedup on `signer_index`
regardless of order.

---

## 10.3 `MultisigTxPayload`

```rust,ignore
pub struct MultisigTxPayload {
    pub target: Address,    // recipient of the spend
    pub amount: u128,       // micro-PYDE to credit target
    pub bundle: SigBundle,  // authorising signatures
}
```

Borsh field order matches
`engine/crates/tx/src/handlers/multisig_tx.rs::MultisigTxPayload`
byte-for-byte. The bundle lives **inside** the payload so the
canonical message can stand on `borsh((target, amount))` alone
— the bundle bytes never enter the signed hash.

### `MultisigTxPayload::canonical_bytes(target, amount)`

| | |
|---|---|
| Signature | `fn canonical_bytes(target: Address, amount: u128) -> Result<Vec<u8>, SdkError>` |
| Returns | The exact `(target, amount)` borsh-encoded bytes to feed `canonical_msg`. |
| Errors | `SdkError::Other` on borsh failure (unreachable for `Address` + `u128`). |

Use this to produce the `payload` bytes for `canonical_msg` —
identical to what the chain feeds into `Poseidon2(payload)` when
verifying.

```rust,no_run
use pyde_rust_sdk::multisig::MultisigTxPayload;
use pyde_rust_sdk::types::Address;

# fn run() -> pyde_rust_sdk::Result<()> {
let payload_bytes: Vec<u8> = MultisigTxPayload::canonical_bytes(
    Address::ZERO,
    1_500_000_000,
)?;
assert_eq!(payload_bytes.len(), 32 + 16);     // address || u128 LE
# Ok(()) }
```

---

## 10.4 Treasury spend — end-to-end

The full flow for a `k-of-n` spend:

```rust,no_run
use pyde_rust_sdk::multisig::{sign_action, MultisigTxPayload, SigBundle};
use pyde_rust_sdk::signer::LocalSigner;
use pyde_rust_sdk::types::{Address, TxType};
use pyde_rust_sdk::{Signer, TxBuilder};

# async fn run() -> pyde_rust_sdk::Result<()> {
// ── Setup ─────────────────────────────────────────────────────────
// In production these come from the on-chain MultisigState —
// each local signer must hold a secret matching one of the
// authorised pubkeys at a known index.
let signers = [
    LocalSigner::random()?,
    LocalSigner::random()?,
    LocalSigner::random()?,
];
let treasury_nonce: u64 = 0;                  // read via provider.get_nonce(&treasury)

let target = Address::from_hex(
    "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
)?;
let amount: u128 = 1_500_000_000;             // 1.5 PYDE

// ── Each signer signs the same canonical message ──────────────────
let payload_bytes = MultisigTxPayload::canonical_bytes(target, amount)?;
let mut bundle: SigBundle = Vec::new();
for idx in [0u32, 2] {                        // signers 0 and 2 participate
    let entry = sign_action(
        &signers[idx as usize],
        idx,                                  // index in MultisigState.signers
        TxType::MultisigTx,
        treasury_nonce,
        &payload_bytes,
    ).await?;
    bundle.push(entry);
}

// ── Build the envelope-style tx ───────────────────────────────────
// `from` and `to` are both ZERO — the bundle authorises both
// the target and amount; tx.signature is left empty (the
// bundle IS the authorisation).
let tx = TxBuilder::new()
    .from(Address::ZERO)
    .chain_id(31337)
    .nonce(0)                                  // chain ignores tx.nonce
                                               // for multisig actions
    .gas_limit(0)
    .multisig_treasury_spend(target, amount, bundle)?
    .build()?;

// Submit unsigned — the bundle IS the signature.
//   provider.send_raw_transaction(&tx).await?;
# Ok(()) }
```

### Off-chain bundle coordination

The SDK doesn't ship a "pass partial bundle around" coordinator
— each signer calls `sign_action` independently against the same
`(tx_type, nonce, payload)` triple, and a coordinator wallet
appends each returned `BundleEntry` to a `Vec`. Coordinate the
nonce and payload through any out-of-band channel (a shared
spreadsheet, a Slack thread, a Substrate-style off-chain worker
— whatever works for your treasury workflow).

---

## 10.5 `multisig` module API reference

### `canonical_msg(tx_type, nonce, payload)`

| | |
|---|---|
| Signature | `fn canonical_msg(tx_type: TxType, nonce: u64, payload: &[u8]) -> Option<[u8; 32]>` |
| `tx_type` | One of `MultisigTx`, `RotateMultisig`, `EmergencyPause`, `EmergencyResume`, `DisputeSlash`. |
| `nonce` | The on-chain `MultisigState.nonce`. |
| `payload` | Per-action borsh bytes. Empty for `EmergencyPause`/`EmergencyResume`. |
| Returns | The 32-byte canonical message a signer signs. `None` if `tx_type` isn't a multisig action. |

### `sign_action(signer, signer_index, tx_type, nonce, payload)`

| | |
|---|---|
| Signature | `async fn sign_action(signer: &dyn Signer, signer_index: u32, tx_type: TxType, nonce: u64, payload: &[u8]) -> Result<BundleEntry, SdkError>` |
| `signer` | Any `Signer` impl whose pubkey is at `MultisigState.signers[signer_index]`. |
| `signer_index` | Position of this signer in the on-chain signer set. |
| Returns | A `BundleEntry { signer_index, signature }`. |
| Errors | `SdkError::InvalidArgument` if `tx_type` isn't a multisig action; `SdkError::Signing` propagated from the signer. |

```rust,no_run
use pyde_rust_sdk::multisig::sign_action;
use pyde_rust_sdk::signer::LocalSigner;
use pyde_rust_sdk::types::TxType;

# async fn run() -> pyde_rust_sdk::Result<()> {
let signer = LocalSigner::random()?;
let entry = sign_action(
    &signer,
    0,                              // signer_index
    TxType::MultisigTx,
    42,                             // treasury_nonce
    b"payload bytes",
).await?;
println!("signature: {} bytes", entry.signature.as_bytes().len());
# Ok(()) }
```

### `TxBuilder::multisig_treasury_spend(target, amount, bundle)`

| | |
|---|---|
| Signature | `fn multisig_treasury_spend(self, target: Address, amount: u128, bundle: SigBundle) -> Result<Self, SdkError>` |
| Sets | `tx_type=MultisigTx`, `to=Address::ZERO`, `data=borsh(MultisigTxPayload{ target, amount, bundle })` |
| Errors | `SdkError::Other` on borsh failure (unreachable). |

---

## 10.6 Reading on-chain multisig state

To pick the right `signer_index` and `treasury_nonce`, read the
treasury account via the normal RPC:

```rust,no_run
use pyde_rust_sdk::types::Address;
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let treasury = Address::from_contract_name("pyde-treasury");
let info = provider.get_account(&treasury).await?;
// info carries the AuthKeys::MultiSig { threshold, signers } shape

// Treasury nonce — same as the account-level nonce.
let nonce = provider.get_nonce(&treasury).await?;
println!("treasury nonce: {nonce}");
# Ok(()) }
```

| Field | Where to read it |
|---|---|
| Signer set + threshold | `provider.get_account(&treasury).await?` — inspect `AuthKeys::MultiSig`. |
| Treasury nonce | `provider.get_nonce(&treasury).await?`. |
| Treasury balance | `provider.get_balance(&treasury).await?`. |

### Treasury address

The treasury's canonical name is `"pyde-treasury"`. Its address
is `Address::from_contract_name("pyde-treasury")` and is stable
across runs (Poseidon2 of the name).

---

## 10.7 Domain byte constants

```rust,ignore
pub const DOMAIN_MULTISIG_TX:      u8 = 0x09;
pub const DOMAIN_ROTATE_MULTISIG:  u8 = 0x0A;
pub const DOMAIN_EMERGENCY_PAUSE:  u8 = 0x0B;
pub const DOMAIN_EMERGENCY_RESUME: u8 = 0x0C;
pub const DOMAIN_DISPUTE_SLASH:    u8 = 0x10;
```

Tests pin each byte against the engine's
`engine/crates/tx/src/multisig.rs::domain_byte`. If the engine
reshapes a domain byte, the SDK fails to build at test time
before any signature mismatch hits production.

See also [Constants §14.7](14-constants.md#147-multisig-domain-bytes).

---

## 10.8 What the SDK doesn't do (yet)

- **Off-chain bundle coordinator** — there's no built-in "round
  one signer signs, sends partial bundle to round two signer"
  flow. Each signer calls `sign_action` independently against
  the same `(tx_type, nonce, payload)` triple; whoever's
  collecting the bundle (a coordinator wallet) just appends
  each returned `BundleEntry` to a `Vec`.
- **Per-user-account multisig wallet helper** — for end-user
  multisig wallets (a `Wallet` shape that's `AuthKeys::MultiSig`),
  the type vocabulary is there but the helper surface ships in
  v2 alongside programmable accounts. Assemble bundles manually
  using the same primitive in the interim.
- **Rotate / Pause / Resume / DisputeSlash helpers** — the
  `canonical_msg` + `sign_action` primitive supports them, but
  there's no `TxBuilder::multisig_rotate(…)` helper today. The
  payload borsh shapes for each are in
  `engine/crates/tx/src/handlers/`; mirror them in your caller
  and use `canonical_msg(TxType::Rotate…, nonce, payload)` to
  produce the signatures.

### See also

- Example: [`examples/multisig_treasury.rs`](../examples/multisig_treasury.rs) — 2-of-3 walkthrough with wire round-trip.
- Source: [`src/multisig.rs`](../src/multisig.rs) — the entire module is one file.
- Chain spec: Ch 11 §11.7.
