# 3. Concepts

[← back to TOC](README.md) · prev: [Quickstart](02-quickstart.md) · next: [Wallets →](04-wallets.md)

---

Pyde's primitives are mostly the post-quantum / ZK-friendly
counterparts to Ethereum-stack defaults. This chapter pins down
the parts you'll see again in every later chapter.

## Table of contents

- [3.1 FALCON-512 signatures](#31-falcon-512-signatures)
- [3.2 Dual hash — Poseidon2 + Blake3](#32-dual-hash--poseidon2--blake3)
- [3.3 Addresses](#33-addresses)
- [3.4 Nonce window (16 slots)](#34-nonce-window-16-slots)
- [3.5 Wave vs block](#35-wave-vs-block)
- [3.6 Units (PYDE / quanta / decimals)](#36-units-pyde--quanta--decimals)
- [3.7 Transaction lifecycle](#37-transaction-lifecycle)
- [3.8 What Pyde does NOT have](#38-what-pyde-does-not-have)
- [3.9 What Pyde DOES have that Ethereum doesn't](#39-what-pyde-does-have-that-ethereum-doesnt)

---

## 3.1 FALCON-512 signatures

Pyde signs transactions with **FALCON-512** — a NIST-standardised
post-quantum signature scheme. Every account that ever signs a
tx has:

| Key part | Size | What |
|---|---|---|
| `FalconPubkey` | 897 bytes | Public key; goes on chain in `AuthKeys::Single(pk)`. |
| `FalconSecret` | 1281 bytes | Private key; stays on the signer's machine. |
| `FalconSignature` | ~666 bytes (average) | Variable-length, randomised per-signing. |

### Example

```rust,no_run
use pyde_rust_sdk::Wallet;

# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
assert_eq!(w.pubkey().as_bytes().len(), 897);
# Ok(()) }
```

### Key properties to remember

| Property | Implication |
|---|---|
| **Randomised** sigs | Two sigs over the same message under the same key produce different bytes — both verify. You can't fingerprint a sender from sig bytes. |
| **Variable length** | Sigs average ~666 bytes but each one differs slightly. Always borsh-encode (which prefixes the length); don't assume a fixed size. |
| **No deterministic mode** | There's no `sign_deterministic` variant. If you need a (key, msg) → bytes fingerprint for idempotence, hash the message yourself first. |
| **897-byte pubkey** | Significantly larger than ECDSA's 33 bytes. Means `AuthKeys::Single` entries are heavier, and the per-tx signature footprint dominates calldata in many cases. |

### Verification

The SDK doesn't expose a `verify` helper directly — verification
is the chain's job during tx admission. If you need to verify
out-of-band (e.g., for an off-chain attestation system), use
`pyde_crypto::falcon::falcon_verify` from the underlying crypto
crate:

```rust,no_run
use pyde_crypto::falcon::{
    falcon_verify, FalconPublicKey, FalconSignature as CryptoSig,
};
# use pyde_rust_sdk::types::FalconPubkey;
# fn run(pk: &FalconPubkey, msg: &[u8; 32], sig_bytes: &[u8]) -> bool {
let pk_crypto = FalconPublicKey::from_bytes(pk.as_bytes()).unwrap();
let sig_crypto = CryptoSig::from_bytes(sig_bytes).unwrap();
falcon_verify(&pk_crypto, msg, &sig_crypto)
# }
```

---

## 3.2 Dual hash — Poseidon2 + Blake3

Pyde uses two hash functions side-by-side:

| Hash | Where in the SDK | Why |
|---|---|---|
| **Poseidon2** | `tx_hash`, state root, address derivation, multisig canonical messages, slot derivation in contracts | ZK-friendly — every field-aligned constraint is cheap to prove in a SNARK. |
| **Blake3** | Mempool dedupe, RPC integrity, event-signature topics, devnet seed derivation, keystore params | Fast — saturates SIMD, no field math overhead. |

Both produce 32-byte digests, but the SDK ships them as **distinct
types** so you can't accidentally mix them:

```rust,no_run
use pyde_rust_sdk::types::{Blake3Hash, Poseidon2Hash, TxHash};

let _: TxHash = TxHash::zero();
let _: Poseidon2Hash = Poseidon2Hash::zero();
let _: Blake3Hash = Blake3Hash::zero();

// This wouldn't compile — different types:
//   fn takes_tx_hash(_: TxHash) {}
//   takes_tx_hash(Poseidon2Hash::zero());
```

### When you need raw access

All three types expose `.as_bytes() -> &[u8; 32]` and a `new(bytes: [u8; 32])`
constructor.

```rust,no_run
use pyde_rust_sdk::types::TxHash;

let h = TxHash::new([0xAA; 32]);
println!("{}", h);                     // "0xaaaa...aaaa"
assert_eq!(h.as_bytes()[0], 0xAA);
```

---

## 3.3 Addresses

A Pyde address is a **full 32-byte Poseidon2 digest** of the
account's FALCON pubkey. No truncation, no checksum — the address
**is** the hash.

```
address = Poseidon2(falcon_pubkey_bytes)
```

### Address from a wallet

```rust,no_run
use pyde_rust_sdk::types::Address;
# use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
let addr: Address = w.address();
println!("{}", addr);
println!("{}", addr.to_hex());
assert_eq!(addr.as_bytes().len(), 32);
# Ok(()) }
```

**Example output:**
```
0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
```

`Display` and `to_hex()` are identical (both produce lowercase
hex with `0x` prefix).

### Address from hex

```rust,no_run
use pyde_rust_sdk::types::Address;

# fn run() -> pyde_rust_sdk::Result<()> {
let addr = Address::from_hex(
    "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
)?;
// `0x` prefix is optional:
let same = Address::from_hex(
    "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899",
)?;
assert_eq!(addr, same);
# Ok(()) }
```

### Contract addresses

Contract addresses are derived from the contract's chosen name:

```
contract_address = Poseidon2("pyde-contract/" || name_bytes)
```

This means `Address::from_contract_name("counter")` predicts the
deploy target before the deploy tx is broadcast — useful for
ENS-style name reservation flows or for hard-coding constants
in dapps that depend on a known contract.

```rust,no_run
use pyde_rust_sdk::types::Address;

let predicted = Address::from_contract_name("counter");
println!("counter will deploy at: {predicted}");
```

### Address derivations available

| Method | Formula | Use case |
|---|---|---|
| `Address::from_pubkey(&pk)` | `Poseidon2(pubkey_bytes)` | EOA derivation (the default — every `Wallet` does this). |
| `Address::create(&deployer, nonce)` | `Poseidon2(deployer ‖ nonce_le)` | Ethereum-style `CREATE` for deploy-by-nonce. |
| `Address::create2(&deployer, &salt, &code_hash)` | `Poseidon2(deployer ‖ salt ‖ code_hash)` | Ethereum-style `CREATE2` for deterministic deploy. |
| `Address::from_contract_name(name)` | `Poseidon2("pyde-contract/" ‖ name)` | Pyde-native named contracts. |

### Special addresses

| Constant | Value | Meaning |
|---|---|---|
| `Address::ZERO` | All zeros | Sentinel for "no recipient" — used by `Deploy` and envelope-style txs (`MultisigTx`). |

---

## 3.4 Nonce window (16 slots)

Pyde uses a **16-slot sliding nonce window** rather than a strict
monotonic counter. The chain accepts any nonce in the range
`[expected, expected + 15]` and bumps the high-water mark on
commit.

### Practical impact

| Behaviour | Consequence |
|---|---|
| **Pipelining is free** | Sign 16 txs back-to-back without waiting for receipts in between. |
| **Out-of-order arrival is fine** | If tx with nonce `N+3` arrives before nonce `N+1`, the chain holds it until `N+1`–`N+2` show up. |
| **Strict monotonicity still works** | Just use the next `get_nonce()` value each time, single-tx style. |

### Getting the right nonce

`provider.get_nonce(&addr)` returns the **next expected nonce**
— i.e., the bottom of the window. For pipelined submission, ask
once + increment locally per tx:

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::{Provider, TxBuilder, Wallet, Signer};
# use pyde_rust_sdk::types::Address;
# async fn run(provider: Arc<dyn Provider>, wallet: Wallet) -> pyde_rust_sdk::Result<()> {
let base = provider.get_nonce(&wallet.address()).await?;
for (i, recipient) in (0..5u64).map(|i| (i, Address::ZERO)) {
    let mut tx = TxBuilder::new()
        .from(wallet.address())
        .nonce(base + i)
        .gas_limit(100_000)
        .transfer(recipient, 1)
        .build()?;
    wallet.sign_tx(&mut tx).await?;
    provider.send_raw_transaction(&tx).await?;
}
# Ok(()) }
```

All 5 txs land in the same wave (or split across two waves if
gas budgets push them).

---

## 3.5 Wave vs block

A **wave** is what Pyde calls a committed batch of transactions —
the same role an Ethereum block plays. Pyde uses "wave" everywhere
because consensus produces them differently (DAG-style
overlapping waves rather than a strict linear chain).

### Practical impact for SDK users

- Time fields on a receipt are `wave_id` + `tx_index_within_wave`.
- The `Provider` trait exposes `wave_id()` as the canonical
  current-head accessor.
- `WaveHeader` is the canonical struct; `BlockHeader =
  WaveHeader` is provided as an Ethereum-vocabulary alias for
  porting muscle memory.

```rust,no_run
# use std::sync::Arc;
# use pyde_rust_sdk::Provider;
# async fn run(provider: Arc<dyn Provider>) -> pyde_rust_sdk::Result<()> {
let head: u64 = provider.wave_id().await?;
println!("chain head: wave {head}");
# Ok(()) }
```

**Expected output (varies):**
```
chain head: wave 1234
```

### New code

Use `WaveHeader` directly. The `BlockHeader` alias is preserved
for compatibility but not the recommended API for fresh code.

---

## 3.6 Units (PYDE / quanta / decimals)

PYDE has **9 decimal places**. Internal balances are always
`u128` quanta — the smallest unit. **Never use floats** for
balance math; use the conversion helpers.

```
1 PYDE = 10^9 quanta = 1_000_000_000 quanta
PYDE_DECIMALS = 9
```

### Converting

```rust,no_run
use pyde_rust_sdk::util::{format_quanta, parse_quanta};

# fn run() -> Result<(), String> {
// human → wire
let q: u128 = parse_quanta("1.5")?;
assert_eq!(q, 1_500_000_000);

let big: u128 = parse_quanta("100.000000001")?;
assert_eq!(big, 100_000_000_001);

// wire → human
let s: String = format_quanta(1_500_000_000);
assert_eq!(s, "1.5");

let s2: String = format_quanta(7);
assert_eq!(s2, "0.000000007");
# Ok(()) }
```

Both round-trip exactly. See [Utilities §13.2](13-utilities.md#132-pyde--quanta-conversion)
for the full API.

### Strict input on `parse_quanta`

`parse_quanta` rejects:

- Negative inputs (`"-1"` → error)
- Scientific notation (`"1e9"` → error)
- More than 9 fractional digits (`"1.0000000001"` → error)
- Non-numeric strings (`"one point five"` → error)

It does NOT reject:

- Leading zeros (`"01"` → 1_000_000_000)
- Trailing zeros (`"1.500"` → 1_500_000_000)
- Bare integers without decimal (`"5"` → 5_000_000_000)

---

## 3.7 Transaction lifecycle

```
build (TxBuilder)          ── you assemble the unsigned Tx
   │
   ▼
sign (Signer.sign_tx)      ── canonical Poseidon2 pre-image,
   │                            FALCON-sign, patch tx.signature
   ▼
submit (Provider.send_*)   ── borsh-encode, JSON-RPC over HTTP/WS
   │
   ▼
mempool                    ── chain validates: nonce window, sig,
   │                            balance, gas budget
   ▼
wave commit                ── tx lands in wave_N at tx_index_K
   │
   ▼
receipt (PendingTx.wait_*) ── poll pyde_getReceipt, return Receipt
```

Each phase has its own chapter:

| Phase | Chapter |
|---|---|
| build + sign + encode | [Transactions §5](05-transactions.md) |
| submit + receipt polling | [Providers §6](06-providers.md) |
| error decoding | [Errors §9](09-errors.md) |

---

## 3.8 What Pyde does NOT have

So you don't go hunting for them:

| Missing | What this means in practice |
|---|---|
| **ECDSA / secp256k1** | All signing is FALCON-512. No `eth_signTypedData`-equivalent today; the equivalent flow would be `Signer::sign_hash`. |
| **Gas refunds (EIP-3529, SELFDESTRUCT)** | `gas_used` is what you pay. No SSTORE refund mechanics. PIP-4 explains why. |
| **Multiple nonces per account** | One nonce counter per account (with the 16-slot window). No EIP-7702-style per-key nonces. |
| **Reorgs after a wave commits** | Once `wait_for_receipt` returns, the receipt is final. No need to wait N confirmations. |
| **`eth_*` JSON-RPC namespace** | The chain uses `pyde_*` method names. Wallets that hard-code `eth_chainId` won't work; they need `pyde_chainId`. |

---

## 3.9 What Pyde DOES have that Ethereum doesn't

| Feature | Notes |
|---|---|
| **Native multisig** | `AuthKeys::MultiSig` is a first-class account shape and the treasury system account is multisig-only. See [Multisig §10](10-multisig.md). |
| **First-class contract names** | `Contract::load("counter", …)` uses the chain's name resolver instead of demanding a 32-byte address. |
| **Threshold-decrypted txs** (opt-in) | Per-account encryption for MEV-resistant transfers. The SDK exposes the wire shape; full helper surface ships in v2. |
| **Wave parallelism** | Independent txs in the same wave execute in parallel — declare an `access_list` to help the scheduler. |
| **Deterministic devnet** | `otigen devnet` always produces the same 10 prefund accounts via `Blake3("pyde-devnet-v1/" || i)`. Reproducible test environments without snapshot files. |
| **Post-quantum from day one** | FALCON-512 sigs aren't a future migration — they're the v1 default. |
