# 4. Wallets

[← back to TOC](README.md) · prev: [Concepts](03-concepts.md) · next: [Transactions →](05-transactions.md)

---

The SDK ships three layers:

| Layer | Type | What you reach for it for |
|---|---|---|
| Low | `LocalSigner` | Just a FALCON-512 keypair + `Signer` impl. No persistence. |
| Mid | `Wallet` | Same but with `to_keystore` / `from_keystore` for at-rest encryption. The 95% default. |
| Custom | `impl Signer for YourType` | HSMs, hardware wallets, remote signers. Two methods (`address`, `sign_hash`) and you're in. |

## Creating a wallet

Three constructors:

```rust,no_run
use pyde_rust_sdk::Wallet;

# fn run() -> pyde_rust_sdk::Result<()> {
// OS entropy — what every new wallet should do.
let w1 = Wallet::generate()?;

// Deterministic from a 32-byte seed — fixtures, tests, devnet
// reproduction. Pinches security to the seed; NOT for production.
let seed = [42u8; 32];
let w2 = Wallet::from_seed(&seed)?;
# Ok(()) }
```

For full keypair restoration from already-encrypted material,
use the keystore round-trip (`to_keystore` / `from_keystore`)
shown below. `Wallet::from_keys(pubkey, secret)` is also
available when you're working with raw key bytes (e.g.
re-importing from a custodial backend).

Common gotchas:

- `from_seed` is **deterministic** — same seed → same FALCON
  keypair → same address. Use it for tests and `pyde devnet`
  banner reproduction; never for real wallets.
- `from_keys` verifies the pubkey actually matches the secret by
  signing + self-verifying a probe message; you get
  `SdkError::Signing` if they don't pair.

## Inspecting

```rust,no_run
# use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;

println!("address: {}", w.address());            // 32-byte hex
println!("pubkey:  {}", w.pubkey().to_hex());    // 897-byte hex
println!("{}", w.address() == w.address());      // true
# Ok(()) }
```

Address comparison is **constant-time** under the hood
(`subtle::ConstantTimeEq` on `FalconPubkey::eq`) — no leak via
early-exit timing on a side channel.

## Encrypted keystore

For at-rest persistence the SDK ships an Argon2id + AES-256-GCM
keystore in a JSON envelope. Wallets call `to_keystore(password)`
to encrypt and `from_keystore(&ks, password)` to restore.

```rust,no_run
use pyde_rust_sdk::{Keystore, Wallet};

# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;

// Encrypt.
let ks: Keystore = w.to_keystore("correct horse battery staple")?;
let json = serde_json::to_string_pretty(&ks)?;
std::fs::write("/tmp/wallet.json", json)?;

// Decrypt.
let bytes = std::fs::read("/tmp/wallet.json")?;
let ks: Keystore = serde_json::from_slice(&bytes)?;
let w2 = Wallet::from_keystore(&ks, "correct horse battery staple")?;

assert_eq!(w.address(), w2.address());
# Ok(()) }
```

### Envelope shape

```json
{
  "version": 1,
  "address": "0x...",
  "pubkey": "0x...",                       // 897-byte FALCON pubkey
  "kdf": {
    "name": "argon2id",
    "params": {
      "m": 65536,                          // memory in KiB
      "t": 3,                              // iterations
      "p": 1,                              // parallelism
      "salt": "0x..."                      // 16 random bytes
    }
  },
  "cipher": {
    "name": "aes-256-gcm",
    "nonce": "0x...",                      // 12 random bytes
    "ciphertext": "0x..."                  // includes appended GCM tag
  }
}
```

Each write picks a fresh salt + nonce — re-encrypting the same
wallet twice produces different ciphertexts (and different
addresses-on-paper-but-same-on-restore).

### KDF parameters

The Argon2id defaults (m=64 MiB, t=3, p=1) target ~250 ms on a
modern laptop CPU. If you're encrypting a lot of wallets in a
batch and 250 ms × N hurts, you can override (though the SDK
doesn't expose the override yet — file an issue if you need it).

### Importing from `pyde-ts-sdk`

You can't, directly. The TS SDK uses ChaCha20-Poly1305 + a flat
envelope shape (`{ cipher: "chacha20-poly1305", nonce, ciphertext }`
at the top level). Convergence is on the roadmap — see
[Compatibility](12-compatibility.md#keystores).

If you need to migrate today: decrypt the TS keystore with a
ChaCha20 + Argon2id implementation outside the SDK, extract the
897-byte pubkey + 1281-byte secret, then `Wallet::from_keys` + a
fresh `to_keystore("…")` to land it in the Rust format.

## The `Signer` trait

Wallets implement `Signer`. The contract is small:

```rust
#[async_trait]
pub trait Signer: Send + Sync {
    fn address(&self) -> Address;
    fn pubkey(&self) -> FalconPubkey;
    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError>;
    async fn sign_tx(&self, tx: &mut Tx) -> Result<(), SdkError> { /* default impl */ }
}
```

`sign_tx` has a default impl that:
1. Computes the canonical Poseidon2 pre-image via `tx::tx_hash(tx)`.
2. Calls `sign_hash(&hash)`.
3. Patches the signature back into `tx.signature`.

You only override it if your backend can do the pre-image hash
more efficiently (e.g. a remote signer that wants the full
pre-image in one round trip).

### Custom signer example — HSM facade

```rust,ignore
use async_trait::async_trait;
use pyde_rust_sdk::error::{Result, SdkError};
use pyde_rust_sdk::signer::Signer;
use pyde_rust_sdk::types::{Address, FalconPubkey, FalconSignature, TxHash};

struct HsmSigner {
    address: Address,
    pubkey: FalconPubkey,
    hsm: MyHsmClient, // your hardware/cloud signing backend
}

#[async_trait]
impl Signer for HsmSigner {
    fn address(&self) -> Address { self.address }
    fn pubkey(&self) -> FalconPubkey { self.pubkey.clone() }

    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature> {
        let sig_bytes = self.hsm
            .sign_falcon512(hash.as_bytes())
            .await
            .map_err(|e| SdkError::Signing(format!("HSM rejected: {e}")))?;
        Ok(FalconSignature::new(sig_bytes))
    }
}
```

Pass an `HsmSigner` anywhere the `transfer` / `deploy` / `call`
examples take a `Wallet` — they're all generic over `&dyn Signer`.

## Multisig wallets

Pyde has first-class multisig at the **account-shape** level
(`AuthKeys::MultiSig`) and at the **action** level (treasury
spends, validator key rotations, emergency pause/resume — all
signed by `k-of-n` bundles).

The treasury-spend helpers live in their own chapter:
[Multisig](10-multisig.md).

For per-account multisig (a user-level wallet that requires N
co-signatures): currently you assemble the bundle by hand using
the same primitive — `multisig::canonical_msg` +
`multisig::sign_action`. A first-class "multisig wallet" helper
is on the v2 roadmap once programmable accounts land.

## Auth keys

Every account has an `AuthKeys` discriminant that controls who
can sign for it:

| Tag  | Variant                       | Meaning |
|------|-------------------------------|---------|
| 0x00 | `AuthKeys::None`              | Unauthorised — funded-but-unregistered. Can only receive funds + run `RegisterPubkey` once. |
| 0x01 | `AuthKeys::Single(pk)`        | Single FALCON pubkey. The 99% case. |
| 0x02 | `AuthKeys::MultiSig {…}`      | `k-of-n` FALCON threshold. |
| 0x03 | `AuthKeys::Programmable`      | Reserved for v2 programmable accounts; encoded but rejected by v1 engines. |

`Wallet` always corresponds to a `Single` account. The chain's
treasury system account is the canonical `MultiSig` example —
see [Multisig](10-multisig.md) for how to authorise actions
against a multisig account.
