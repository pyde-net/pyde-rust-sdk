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

All five share one signing primitive: each signer signs the same
**canonical message** under their FALCON key; the chain verifies
≥ `threshold` distinct valid signatures from the on-chain signer
set before applying the action.

## Canonical message

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

## Bundle wire shape

```rust,ignore
pub struct BundleEntry {
    pub signer_index: u32,
    pub signature: FalconSignature,    // FALCON-512 over canonical_msg
}
pub type SigBundle = Vec<BundleEntry>;
```

`signer_index` is the 0-based position of the signing pubkey in
`MultisigState.signers`. Out-of-range indices and duplicates
within a bundle are rejected by the chain.

## Treasury spend — end-to-end

The full flow for a `k-of-n` spend:

```rust,no_run
use pyde_rust_sdk::multisig::{sign_action, MultisigTxPayload, SigBundle};
use pyde_rust_sdk::signer::LocalSigner;
use pyde_rust_sdk::types::{Address, TxType};
use pyde_rust_sdk::{Signer, TxBuilder};

# async fn run() -> pyde_rust_sdk::Result<()> {
// ── Setup ─────────────────────────────────────────────────────────
// In production these come from the on-chain MultisigState —
// the local signer must hold a secret matching one of the
// authorised pubkeys at a known index.
let signers = [
    LocalSigner::random()?,
    LocalSigner::random()?,
    LocalSigner::random()?,
];
let treasury_nonce: u64 = 0;

let target = Address::from_hex(
    "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
)?;
let amount: u128 = 1_500_000_000;     // 1.5 PYDE

// ── Each signer signs the same canonical message ──────────────────
let payload_bytes = MultisigTxPayload::canonical_bytes(target, amount)?;
let mut bundle: SigBundle = Vec::new();
for idx in [0u32, 2] {                // signers 0 and 2 participate
    let entry = sign_action(
        &signers[idx as usize],
        idx,                          // index in MultisigState.signers
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
    .nonce(0)                          // chain ignores tx.nonce for
                                       // multisig actions
    .gas_limit(0)
    .multisig_treasury_spend(target, amount, bundle)?
    .build()?;
# Ok(()) }
```

Then submit via `provider.send_raw_transaction(&tx)` — see
[Providers](06-providers.md#rpc-method-catalogue).

## Inspecting the on-chain `MultisigState`

To pick the right `signer_index` and `treasury_nonce`, read the
treasury account:

```rust,no_run
use pyde_rust_sdk::types::Address;
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let treasury = Address::from_contract_name("pyde-treasury");
let info = provider.get_account(&treasury).await?;
// info carries the AuthKeys::MultiSig { threshold, signers } shape
# Ok(()) }
```

The treasury's name is the canonical `"pyde-treasury"`. The
returned `AccountInfo` carries the current signer set; the
treasury nonce comes from the same shape (it's also exposed via
`provider.get_nonce(&treasury)`).

## What the SDK does NOT do (yet)

- **Off-chain bundle assembly** — there's no built-in "round
  one signer signs, sends partial bundle to round two signer"
  flow. Each signer calls `sign_action` independently against
  the same `(tx_type, nonce, payload)` triple; whoever's
  collecting the bundle (a coordinator wallet) just appends
  each returned `BundleEntry` to a `Vec`.
- **Per-user-account multisig wallet helper** — for end-user
  multisig wallets (i.e., a `Wallet` shape that's
  `AuthKeys::MultiSig`), the type vocabulary is there but the
  helper surface ships in v2 alongside programmable accounts.
  Assemble bundles manually using the same primitive in the
  interim.
- **Rotate / Pause / Resume / DisputeSlash helpers** — the
  `canonical_msg` + `sign_action` primitive supports them, but
  there's no `TxBuilder::multisig_rotate(…)` helper today. The
  payload borsh shapes for each are in
  `engine/crates/tx/src/handlers/`; mirror them in your
  caller and use `canonical_msg(TxType::Rotate…, nonce, payload)`
  to produce the signatures.

## Constants reference

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

## See also

- Example: [`examples/multisig_treasury.rs`](../examples/multisig_treasury.rs) — 2-of-3 walkthrough with wire round-trip.
- Source: [`src/multisig.rs`](../src/multisig.rs) — the entire module is one file.
- Chain spec: Ch 11 §11.7.
