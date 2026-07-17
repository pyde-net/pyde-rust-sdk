# 4. Wallets

[← back to TOC](README.md) · prev: [Concepts](03-concepts.md) · next: [Transactions →](05-transactions.md)

---

The SDK ships three layers for key management:

| Layer | Type | What you reach for it for |
|---|---|---|
| Low | `LocalSigner` | Just a FALCON-512 keypair + `Signer` impl. No persistence. |
| Mid | `Wallet` | Same but with `to_keystore` / `from_keystore` for at-rest encryption. The 95% default. |
| Custom | `impl Signer for YourType` | HSMs, hardware wallets, remote signers. Two methods (`address`, `sign_hash`) and you're in. |

## Table of contents

- [4.1 `LocalSigner` — bare keypair](#41-localsigner--bare-keypair)
- [4.2 `Wallet` — high-level keypair](#42-wallet--high-level-keypair)
- [4.3 `Wallet` API reference](#43-wallet-api-reference)
- [4.4 Encrypted keystore on disk](#44-encrypted-keystore-on-disk)
- [4.5 `Keystore` API reference](#45-keystore-api-reference)
- [4.6 The `Signer` trait](#46-the-signer-trait)
- [4.7 Custom signer — HSM facade](#47-custom-signer--hsm-facade)
- [4.8 Auth keys](#48-auth-keys)
- [4.9 Memory safety + zeroize](#49-memory-safety--zeroize)
- [4.10 Multisig wallets](#410-multisig-wallets)

---

## 4.1 `LocalSigner` — bare keypair

`LocalSigner` holds a FALCON-512 keypair in memory and implements
the `Signer` trait. It's what `Wallet` wraps internally; you'd
use it directly only if you don't need the encrypted-keystore
helpers.

### Constructors

| Method | Use case |
|---|---|
| `LocalSigner::random()` | Fresh OS-entropy keypair. Default for new wallets. |
| `LocalSigner::from_seed(&[u8; 32])` | Deterministic from a 32-byte seed. Test / fixture only. |
| `LocalSigner::from_secret(FalconSecret)` | Load a previously-generated secret. Address is re-derived from the pubkey. |
| `LocalSigner::from_keys(FalconPubkey, FalconSecret)` | Load both halves explicitly. Verifies they pair via a probe signature. |

### Example

```rust,no_run
use pyde_rust_sdk::signer::LocalSigner;

# fn run() -> pyde_rust_sdk::Result<()> {
let s = LocalSigner::random()?;
println!("{}", s.address());
# Ok(()) }
```

**Expected output:**
```
0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
```

---

## 4.2 `Wallet` — high-level keypair

`Wallet` is a thin convenience layer over `LocalSigner`:

- Implements `Signer` (plugs straight into the provider's
  `sign_tx` path).
- Exposes `to_keystore` / `from_keystore` for at-rest encryption.
- Forwards the common `address()` / `pubkey()` accessors.

### Three constructors

```rust,no_run
use pyde_rust_sdk::Wallet;

# fn run() -> pyde_rust_sdk::Result<()> {
// 1. OS entropy — what every new wallet should do.
let w1 = Wallet::generate()?;

// 2. Deterministic from a 32-byte seed — fixtures, tests,
//    devnet reproduction. Pinches security to the seed; NOT
//    for production.
let seed = [42u8; 32];
let w2 = Wallet::from_seed(&seed)?;

// 3. Round-trip via keystore for "restore from at-rest material."
let ks = w1.to_keystore("acct", "pw")?;
let w3 = Wallet::from_keystore(&ks, "acct", "pw")?;
# Ok(()) }
```

Common gotchas:

- `from_seed` is **deterministic** — same seed → same FALCON
  keypair → same address. Use it for tests and `otigen devnet`
  banner reproduction; never for real wallets.
- The seed is the *entire* security base. A 32-byte
  cryptographically-strong random seed is fine; a 32-byte ASCII
  password is NOT.

---

## 4.3 `Wallet` API reference

### `Wallet::generate()`

| | |
|---|---|
| Signature | `fn generate() -> Result<Wallet, SdkError>` |
| Returns | A fresh `Wallet` with a FALCON-512 keypair from OS entropy. |
| Errors | `SdkError::Signing` if FALCON keygen fails (~unreachable on commodity hardware — only happens if `getrandom` itself fails). |

```rust,no_run
use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
# Ok(()) }
```

### `Wallet::from_seed(seed)`

| | |
|---|---|
| Signature | `fn from_seed(seed: &[u8; 32]) -> Result<Wallet, SdkError>` |
| `seed` | 32-byte seed. Determines the entire keypair. |
| Returns | Wallet with a deterministically-derived keypair. |
| Errors | `SdkError::Signing` on keygen failure. |

```rust,no_run
use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::from_seed(&[7u8; 32])?;
println!("{}", w.address()); // same on every call with the same seed
# Ok(()) }
```

### `Wallet::from_keys(pubkey, secret)`

| | |
|---|---|
| Signature | `fn from_keys(pubkey: FalconPubkey, secret: FalconSecret) -> Result<Wallet, SdkError>` |
| `pubkey` | 897-byte FALCON pubkey. |
| `secret` | 1281-byte FALCON secret. |
| Returns | Wallet wired with both halves. |
| Errors | `SdkError::Signing` if `pubkey` doesn't pair with `secret` — checked by signing + self-verifying a probe message. |

### `Wallet::address() -> Address`

Returns the on-chain address (32-byte Poseidon2 of the pubkey).

### `Wallet::pubkey() -> FalconPubkey`

Returns the 897-byte FALCON pubkey.

### `Wallet::to_keystore(account_name, password)`

| | |
|---|---|
| Signature | `fn to_keystore(&self, account_name: &str, password: &str) -> Result<Keystore, SdkError>` |
| `account_name` | Name for this account inside the vault. |
| `password` | Encryption password. |
| Returns | A canonical `Keystore` vault with one entry (Argon2id + AES-256-GCM). |
| Errors | `SdkError::Other` on KDF/cipher failure (~unreachable). |

Add more accounts to the same vault with
`add_to_keystore(&mut keystore, name, password)`.

```rust,no_run
use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
let ks = w.to_keystore("my-account", "correct horse battery staple")?;
let json = serde_json::to_string_pretty(&ks)?;
std::fs::write("keystore.json", json)?;
# Ok(()) }
```

### `Wallet::from_keystore(&ks, account_name, password)`

| | |
|---|---|
| Signature | `fn from_keystore(keystore: &Keystore, account_name: &str, password: &str) -> Result<Wallet, SdkError>` |
| `keystore` | A canonical `Keystore` vault. |
| `account_name` | Which account in the vault to open. |
| `password` | Decryption password — must match `to_keystore`. |
| Returns | The restored `Wallet`. |
| Errors | `SdkError::InvalidArgument` for wrong password, unknown account, unsupported version, a KDF below the Argon2id floor, malformed envelope, or pubkey/address mismatch. |

For raw JSON that may also be the legacy `0.1.0` nested keystore,
use `Wallet::from_keystore_json(json, account_name, password)`.

```rust,no_run
use pyde_rust_sdk::{Keystore, Wallet};
# fn run() -> pyde_rust_sdk::Result<()> {
let bytes = std::fs::read("keystore.json")?;
let ks: Keystore = serde_json::from_slice(&bytes)?;
let w = Wallet::from_keystore(&ks, "my-account", "correct horse battery staple")?;
println!("{}", w.address());
# Ok(()) }
```

---

## 4.4 Encrypted keystore on disk

For at-rest persistence the SDK encrypts the FALCON secret under
a password-derived AES-256-GCM key. Argon2id is the KDF; the
defaults target ~250 ms on a modern laptop CPU.

### Full round-trip

```rust,no_run
use pyde_rust_sdk::{Keystore, Wallet};

# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;

// Encrypt + save.
let ks: Keystore = w.to_keystore("my-account", "correct horse battery staple")?;
let json = serde_json::to_string_pretty(&ks)?;
std::fs::write("/tmp/keystore.json", json)?;

// Load + decrypt.
let bytes = std::fs::read("/tmp/keystore.json")?;
let ks: Keystore = serde_json::from_slice(&bytes)?;
let w2 = Wallet::from_keystore(&ks, "my-account", "correct horse battery staple")?;

assert_eq!(w.address(), w2.address());
# Ok(()) }
```

**Expected output:** (no output — assertion passes silently.)

### JSON envelope shape

A keystore file looks like this:

```json
{
  "version": 1,
  "address": "0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2",
  "pubkey": "0x09597966f87e94eb3e50d78a56a04918eed0faab09ddbf45e29210a8c5111c2e...",
  "kdf": {
    "name": "argon2id",
    "params": {
      "m": 65536,
      "t": 3,
      "p": 1,
      "salt": "0xec9b365e27d087f8798c1ab7ff478696"
    }
  },
  "cipher": {
    "name": "aes-256-gcm",
    "nonce": "0x53534e013ff24cdc8e6ab071",
    "ciphertext": "0x9141630b54d872..."
  }
}
```

| Field | Meaning |
|---|---|
| `version` | Envelope version. Currently `1`. |
| `address` | The wallet's on-chain address (32 bytes, hex). |
| `pubkey` | The FALCON pubkey (897 bytes, hex). |
| `kdf.name` | Always `"argon2id"` in v1. |
| `kdf.params.m` | Argon2id memory cost, in KiB. Default `65536` (64 MiB). |
| `kdf.params.t` | Argon2id iteration count. Default `3`. |
| `kdf.params.p` | Argon2id parallelism (lanes). Default `1`. |
| `kdf.params.salt` | 16 random bytes per write (hex). |
| `cipher.name` | Always `"aes-256-gcm"` in v1. |
| `cipher.nonce` | 12 random bytes per write (hex). |
| `cipher.ciphertext` | The encrypted FALCON secret, plus the appended GCM tag (hex). |

Each write picks a fresh salt + nonce — re-encrypting the same
wallet twice produces different `ciphertext` and `salt` bytes
but the same `address` on restore.

### Tampering

The keystore binds the `address` into the AEAD as associated data.
If anyone edits the `pubkey` or `address` field after encryption,
`from_keystore` returns `SdkError::InvalidArgument` ("decrypt
failed — bad password or corrupt keystore").

### Wrong password

```rust,no_run
use pyde_rust_sdk::{Keystore, Wallet};
# fn run() -> pyde_rust_sdk::Result<()> {
let w = Wallet::generate()?;
let ks = w.to_keystore("acct", "right")?;
let err = Wallet::from_keystore(&ks, "acct", "wrong").unwrap_err();
println!("{err}");
# Ok(()) }
```

**Expected output:**
```
invalid argument: decrypt failed — bad password or corrupt keystore
```

### Importing from the `otigen` CLI and other tools

Directly. This SDK writes and reads the canonical Pyde account
keystore, so a vault minted by `otigen wallet new` (or any
conformant tool) opens here with no migration step:

```rust,ignore
let keystore: Keystore = serde_json::from_slice(&std::fs::read("keystore.json")?)?;
let wallet = Wallet::from_keystore(&keystore, "my-account", password)?;
```

`Wallet::from_keystore_json` additionally opens the older nested
keystore this SDK wrote at `0.1.0`. See
[Compatibility §12.5](12-compatibility.md#125-keystore-format-canonical-cross-tool)
for the full format and the `pyde-ts-sdk` convergence status.

---

## 4.5 `Keystore` API reference

### `Keystore::encrypt(pubkey, secret, password)`

The low-level encrypt path. `Wallet::to_keystore` is a thin
wrapper that calls this.

| | |
|---|---|
| Signature | `fn encrypt(pubkey: &FalconPubkey, secret: &FalconSecret, password: &str) -> Result<Keystore, SdkError>` |
| Returns | A new `Keystore`. |

### `Keystore::decrypt(password)`

| | |
|---|---|
| Signature | `fn decrypt(&self, password: &str) -> Result<(FalconPubkey, FalconSecret), SdkError>` |
| Returns | The decrypted pubkey + secret tuple. |
| Errors | `SdkError::InvalidArgument` on wrong password / unsupported version / malformed hex / pubkey-address mismatch. |

### Constants

| Constant | Value | What |
|---|---|---|
| `KEYSTORE_VERSION` | `1` | The envelope version this SDK encrypts and decrypts. |
| `ARGON2_M_KIB` | `65_536` | Memory cost (64 MiB). |
| `ARGON2_T` | `3` | Iteration count. |
| `ARGON2_P` | `1` | Parallelism (lanes). |
| `ARGON2_SALT_LEN` | `16` | Salt size in bytes. |
| `AES_KEY_LEN` | `32` | Derived AES key size. |
| `AES_NONCE_LEN` | `12` | GCM nonce size. |

See [Constants §14.4](14-constants.md#144-keystore-parameters).

---

## 4.6 The `Signer` trait

`Wallet` and `LocalSigner` both implement `Signer`. Custom
signers (HSMs, remote APIs, hardware wallets) implement it too.

```rust,ignore
#[async_trait]
pub trait Signer: Send + Sync {
    fn address(&self) -> Address;
    fn pubkey(&self) -> FalconPubkey;
    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError>;
    async fn sign_tx(&self, tx: &mut Tx) -> Result<(), SdkError> {
        // default impl — most signers want this as-is
    }
}
```

| Method | Required | What |
|---|---|---|
| `address` | yes | Return the signer's on-chain address. |
| `pubkey` | yes | Return the signer's 897-byte FALCON pubkey. |
| `sign_hash` | yes | Sign a 32-byte hash. Caller is responsible for ensuring it's a canonical `tx_hash`. |
| `sign_tx` | no — default impl | Sign a `Tx` in place: compute `tx_hash(tx)`, call `sign_hash`, patch `tx.signature`. |

You only override `sign_tx` if your backend can do the pre-image
hash more efficiently in one round trip (e.g., a remote signer
that prefers to receive the full pre-image rather than the hash).

### Using a signer

```rust,no_run
use pyde_rust_sdk::{Signer, TxBuilder, Wallet};
use pyde_rust_sdk::types::Address;

# async fn run() -> pyde_rust_sdk::Result<()> {
let signer: Wallet = Wallet::generate()?;     // any `impl Signer` works
let mut tx = TxBuilder::new()
    .from(signer.address())
    .chain_id(31337)
    .nonce(0)
    .gas_limit(100_000)
    .transfer(Address::ZERO, 1)
    .build()?;
signer.sign_tx(&mut tx).await?;
# Ok(()) }
```

---

## 4.7 Custom signer — HSM facade

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
    fn address(&self) -> Address {
        self.address
    }
    fn pubkey(&self) -> FalconPubkey {
        self.pubkey.clone()
    }
    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature> {
        let sig_bytes = self.hsm
            .sign_falcon512(hash.as_bytes())
            .await
            .map_err(|e| SdkError::Signing(format!("HSM rejected: {e}")))?;
        Ok(FalconSignature::new(sig_bytes))
    }
}
```

Pass `&HsmSigner` anywhere the SDK takes a `&dyn Signer` (every
contract `.send()` call, every `wallet.sign_tx(&mut tx)`).

The HSM is responsible for:
- Producing a FALCON-512 signature over the exact 32-byte hash
  it receives.
- Returning the variable-length signature bytes verbatim.

---

## 4.8 Auth keys

Every account on chain has an `AuthKeys` discriminant that
controls who can sign for it:

| Tag  | Variant                       | Meaning |
|------|-------------------------------|---------|
| 0x00 | `AuthKeys::None`              | Unauthorised — funded-but-unregistered. Can only receive funds + run `RegisterPubkey` once. |
| 0x01 | `AuthKeys::Single(pk)`        | Single FALCON pubkey. The 99% case. |
| 0x02 | `AuthKeys::MultiSig {…}`      | `k-of-n` FALCON threshold. |
| 0x03 | `AuthKeys::Programmable`      | Reserved for v2 programmable accounts; encoded but rejected by v1 engines. |

```rust,no_run
use pyde_rust_sdk::types::{AuthKeys, FalconPubkey};

# fn run(pk: FalconPubkey) {
let single = AuthKeys::Single(pk.clone());
let multi = AuthKeys::MultiSig {
    threshold: 2,
    signers: vec![pk.clone(), pk.clone(), pk],
};
let none = AuthKeys::None;
# }
```

### Validation

```rust,no_run
use pyde_rust_sdk::types::AuthKeys;

# fn run(keys: &AuthKeys) -> pyde_rust_sdk::Result<()> {
keys.validate().map_err(|e| pyde_rust_sdk::SdkError::InvalidArgument(e.to_string()))?;
# Ok(()) }
```

For `MultiSig`: validates `1 <= threshold <= signers.len() <=
MAX_MULTISIG_SIGNERS` and rejects empty signer sets.

| Constant | Value |
|---|---|
| `MAX_MULTISIG_SIGNERS` | `16` (Ch 11 §11.5) |

---

## 4.9 Memory safety + zeroize

The SDK takes basic care of secret-key memory:

- **`FalconSecret`** derives `Zeroize, ZeroizeOnDrop` — the
  1281-byte secret buffer is wiped to zeros before the allocator
  reclaims it.
- **`LocalSigner`** inherits this via Rust's auto-Drop — its
  `secret: FalconSecret` field is dropped → wiped.
- **`Wallet`** inherits in turn.
- **Keystore encrypt/decrypt** explicitly `zeroize()`s the
  AES-256 key and any plaintext buffer used as a staging area
  before returning.

```rust,no_run
use pyde_rust_sdk::Wallet;
# fn run() -> pyde_rust_sdk::Result<()> {
{
    let w = Wallet::generate()?;
    // ... use w ...
} // ← secret bytes wiped here as `w` drops
# Ok(()) }
```

### What's NOT zeroized

- **Your password `&str`** — the SDK takes a borrowed `&str` so
  the caller controls its lifetime. If you need the password
  zeroed, use `zeroize::Zeroizing<String>` on your side and pass
  `&zeroizing.as_str()`.
- **Borsh-encoded tx bytes** — the public-key bytes appear in
  any encoded `Tx`, but those aren't secret. Signatures are
  randomised, so even the signature bytes aren't secret either.

The pin test `wallet_drop_wipes_secret` (in
`src/wallet/mod.rs`) verifies the Drop chain stays intact — if
someone removes the `ZeroizeOnDrop` derive in a future refactor,
that test fails immediately.

---

## 4.10 Multisig wallets

Pyde has multisig at two levels:

| Level | What | SDK surface |
|---|---|---|
| **Account shape** | `AuthKeys::MultiSig { threshold, signers }` — any account on chain can have this shape. | Wire-type only in v1; helper surface for end-user multisig wallets is v2. |
| **Action shape** | Treasury spends, validator key rotations, emergency pause/resume — all signed by `k-of-n` bundles against the treasury's `MultisigState`. | Full helper surface — see [Multisig §10](10-multisig.md). |

For the **treasury action flow**, use the helpers in
`pyde_rust_sdk::multisig` — `canonical_msg`, `sign_action`,
`MultisigTxPayload`, `TxBuilder::multisig_treasury_spend`. See
the dedicated chapter for the full walkthrough.

For **per-account multisig wallets** (a user-level wallet that
requires N co-signatures): assemble the bundle by hand using the
same primitive. A first-class "multisig wallet" helper is planned
for v2 once programmable accounts land.
