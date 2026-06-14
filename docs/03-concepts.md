# 3. Concepts

[← back to TOC](README.md) · prev: [Quickstart](02-quickstart.md) · next: [Wallets →](04-wallets.md)

---

Pyde's primitives are mostly the post-quantum / ZK-friendly
counterparts to Ethereum-stack defaults. This chapter pins down
the parts you'll see again in every later chapter.

## Cryptography

### FALCON-512 signatures

Pyde signs transactions with FALCON-512 — a NIST-standardised
post-quantum signature scheme. Every account that ever signs a
tx has a 897-byte FALCON pubkey + 1281-byte secret. Signatures
are variable-length (averaging ~666 bytes) and **randomised** —
signing the same message twice with the same key yields different
bytes that both verify.

```rust,no_run
use pyde_rust_sdk::Wallet;

# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
assert_eq!(w.pubkey().as_bytes().len(), 897);
# Ok(()) }
```

Two callouts:
- **Randomised** sigs mean you can't fingerprint a sender from a
  signature alone, and you can't use signature equality as
  identity. Always compare addresses or pubkey bytes.
- **No determinism by design** — there's no `sign_deterministic`
  variant. If you need a per-(key, message) fingerprint for
  idempotence, hash the message yourself.

### Dual hash: Poseidon2 + Blake3

Pyde uses two hash functions side-by-side:

| Hash | Where | Why |
|---|---|---|
| **Poseidon2** | Transaction hash, state root, address derivation, multisig canonical messages | ZK-friendly — every field-aligned constraint is cheap to prove in a SNARK |
| **Blake3** | Mempool dedupe, RPC integrity, event-signature topics, devnet seed derivation | Fast — saturates SIMD, no field math overhead |

Both expose 32-byte digests. The SDK's `Poseidon2Hash` /
`Blake3Hash` / `TxHash` newtypes are all 32 bytes but distinct
types so you can't accidentally mix them.

```rust,no_run
use pyde_rust_sdk::types::{Blake3Hash, Poseidon2Hash, TxHash};

let _: TxHash = TxHash::zero();
let _: Poseidon2Hash = Poseidon2Hash::zero();
let _: Blake3Hash = Blake3Hash::zero();
```

## Addresses

A Pyde address is a full 32-byte Poseidon2 digest of the
account's FALCON pubkey. No truncation, no checksum — the address
**is** the hash.

```
address = Poseidon2(falcon_pubkey_bytes)
```

```rust,no_run
use pyde_rust_sdk::types::Address;
# use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
let addr: Address = w.address();
println!("{}", addr);                       // "0x…" 64 hex chars
println!("{}", addr.to_hex());              // same
assert_eq!(addr.as_bytes().len(), 32);
# Ok(()) }
```

Contract addresses are derived from the contract's chosen name:

```
contract_address = Poseidon2("pyde-contract/" || name_bytes)
```

So `Address::from_contract_name("counter")` predicts the deploy
target before the deploy tx is broadcast. See
[Contracts](07-contracts.md#deployment).

Special addresses:

- `Address::ZERO` — all zeros. Sentinel for "no recipient" — used
  by `Deploy` and envelope-style txs (e.g. `MultisigTx`).

## Nonce window

Pyde uses a 16-slot **nonce window** rather than a strict
monotonic counter. The chain accepts any nonce in the range
`[expected, expected + 15]` and bumps the high-water mark on
commit. Practical impact:

- Pipelining is free — you can sign 16 txs back-to-back without
  waiting for receipts.
- Out-of-order arrival is fine; the chain reorders.
- Strictly-monotonic dapps still work — just pick the next
  `get_nonce()` value each time.

The SDK's `get_nonce()` returns the next expected nonce (i.e.
the bottom of the window), which is what most wallets want.

## Wave vs block

A **wave** is what Pyde calls a committed batch of transactions —
the same role an Ethereum block plays. We use "wave" everywhere
internally because Pyde's consensus produces them differently
than Ethereum (DAG-style overlapping waves rather than a strict
linear chain).

Practical implication for SDK users:
- Time fields on a receipt are `wave_id` + `tx_index_within_wave`.
- The Provider trait exposes both `wave_id()` (canonical) and
  `BlockHeader` (Ethereum-vocab alias for `WaveHeader`).
- New code should use `WaveHeader`; the `BlockHeader` alias is
  there for porting code from Ethereum SDKs.

## Units

PYDE has 9 decimal places. Internal balances are always `u128`
quanta — the smallest unit. **Never use floats.**

```
1 PYDE = 10^9 quanta = 1_000_000_000 quanta
PYDE_DECIMALS = 9
```

The SDK provides two helpers:

```rust,no_run
use pyde_rust_sdk::util::{parse_quanta, format_quanta};

// human → wire
let q: u128 = parse_quanta("1.5").unwrap();
assert_eq!(q, 1_500_000_000);

// wire → human
let s: String = format_quanta(1_500_000_000);
assert_eq!(s, "1.5");
```

Both round-trip exactly. `parse_quanta` rejects negative inputs,
scientific notation, and more than 9 fractional digits.

## Transaction lifecycle

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
mempool                    ── pyde validates: nonce window, sig,
   │                            balance, gas budget
   ▼
wave commit                ── tx lands in wave_N at tx_index_K
   │
   ▼
receipt (PendingTx.wait_*) ── poll pyde_getReceipt, return Receipt
```

Each step has its own chapter — [Transactions](05-transactions.md)
covers build + sign + encode; [Providers](06-providers.md) covers
submit + receipt polling.

## Things Pyde does NOT have

So you don't go hunting for them:

- **No ECDSA / secp256k1.** All signing is FALCON-512.
- **No gas refunds.** `gas_used` is what you pay. No SSTORE
  refund mechanics, no SELFDESTRUCT refund (PIP-4).
- **No nonces-per-account-tag** like EIP-7702 — `auth_keys` is
  a single discriminant per account (see [Wallets](04-wallets.md#auth-keys)).
- **No reorgs after a wave commits.** Once `wait_for_receipt`
  returns, the receipt is final — no need to wait N waves.

## Things Pyde DOES have that Ethereum doesn't

- **Native multisig.** `AuthKeys::MultiSig` is a first-class
  account shape and the treasury system account is multisig-only.
  See [Multisig](10-multisig.md).
- **First-class contract names.** `Contract::load("counter", …)`
  uses the chain's name resolver instead of demanding a 32-byte
  address.
- **Threshold-decrypted txs** (optional, opt-in) — see the chain
  spec; the SDK exposes the wire shape but the v1 helper surface
  treats encryption as a per-account opt-in handled by the wallet.
