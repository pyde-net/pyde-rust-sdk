//! `Wallet` + the canonical Pyde account [`Keystore`].
//!
//! A wallet is the user-facing entry point for the SDK:
//!
//! ```ignore
//! use pyde_rust_sdk::wallet::Wallet;
//!
//! // Create a new wallet from OS entropy.
//! let wallet = Wallet::generate()?;
//! println!("address: {}", wallet.address());
//!
//! // Persist it into a password-protected keystore vault.
//! let keystore = wallet.to_keystore("my-account", "correct horse battery staple")?;
//! std::fs::write("keystore.json", serde_json::to_vec_pretty(&keystore)?)?;
//!
//! // Load it back.
//! let stored: pyde_rust_sdk::wallet::Keystore =
//!     serde_json::from_slice(&std::fs::read("keystore.json")?)?;
//! let wallet = Wallet::from_keystore(&stored, "my-account", "correct horse battery staple")?;
//! ```
//!
//! ## Keystore format (canonical, cross-tool)
//!
//! The on-disk format is the **canonical Pyde account keystore**: a
//! multi-account JSON vault shared by `otigen-wallet`, `pyde-rust-sdk`,
//! `pyde-ts-sdk`, the playground, and the wallet. A keystore written by
//! any conformant tool decrypts in every other, because AES-256-GCM is
//! authenticated: a correct decrypt proves the derived key matched
//! byte-for-byte.
//!
//! ```json
//! {
//!   "version": 1,
//!   "accounts": {
//!     "my-account": {
//!       "address":    "0x…",
//!       "pubkey":     "0x…",
//!       "ciphertext": "0x…",
//!       "salt":       "0x… (16 bytes)",
//!       "nonce":      "0x… (12 bytes)",
//!       "cipher":     "aes-256-gcm",
//!       "kdf": { "name": "argon2id", "memory_kb": 65536, "iterations": 3, "parallelism": 4 }
//!     }
//!   }
//! }
//! ```
//!
//! - **KDF**: Argon2id (version 0x13) with `m = 64 MiB`, `t = 3`,
//!   `p = 4`, 32-byte output. Params are embedded per entry so a future
//!   param bump still decrypts old vaults.
//! - **Cipher**: AES-256-GCM, 12-byte random nonce per entry, 16-byte
//!   GCM tag appended to the ciphertext, **no associated data** (AAD),
//!   so the encoding is identical across implementations.
//! - **Encrypted payload**: only the 1281-byte FALCON-512 secret key.
//!   The public key and address are stored in the clear.
//!
//! The reader is param-agile: it derives each entry with that entry's
//! own stored Argon2id parameters and accepts only the `aes-256-gcm`
//! cipher suite. It applies an anti-DoS **upper** clamp on the KDF
//! parameters (a crafted `memory_kb` would otherwise force a multi-GiB
//! allocation) but deliberately imposes no lower floor, matching the
//! reference implementations, so a legitimately-owned below-floor
//! keystore still opens. [`Wallet::from_keystore_json`] additionally
//! reads the older nested single-account keystore this SDK wrote at
//! `0.1.0`.

use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use async_trait::async_trait;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::error::SdkError;
use crate::signer::{LocalSigner, Signer};
use crate::types::{
    Address, FalconPubkey, FalconSecret, FalconSignature, Tx, TxHash, FALCON_SECRET_LEN,
};

/// User-facing wallet — owns a [`LocalSigner`] under the hood.
///
/// `Wallet` is a thin convenience layer over [`LocalSigner`]: it
/// implements [`Signer`] (so it plugs straight into the provider's
/// `sign_tx` path), exposes the [`Self::to_keystore`] /
/// [`Self::from_keystore`] persistence pair, and forwards the common
/// `address()` / `pubkey()` accessors.
///
/// ## Memory safety
///
/// The inner [`FalconSecret`] derives `ZeroizeOnDrop`, so when a
/// `Wallet` is dropped the 1281-byte secret-key buffer is wiped
/// before the allocator reclaims the memory.
pub struct Wallet {
    signer: LocalSigner,
}

impl Wallet {
    /// Generate a fresh wallet from OS entropy.
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if FALCON keygen fails.
    pub fn generate() -> Result<Self, SdkError> {
        Ok(Self {
            signer: LocalSigner::random()?,
        })
    }

    /// Generate a deterministic wallet from a 32-byte seed.
    ///
    /// **Test-only / fixture-only.** Real wallets must use
    /// [`Self::generate`] so the kernel's entropy pool sets the key.
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if FALCON deterministic keygen
    /// fails.
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, SdkError> {
        Ok(Self {
            signer: LocalSigner::from_seed(seed)?,
        })
    }

    /// Construct a wallet from a previously-stored pubkey + secret.
    ///
    /// Validates that the pubkey matches the secret.
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] on a pubkey/secret mismatch.
    pub fn from_keys(pubkey: FalconPubkey, secret: FalconSecret) -> Result<Self, SdkError> {
        Ok(Self {
            signer: LocalSigner::from_keys(pubkey, secret)?,
        })
    }

    /// Borrow the underlying [`LocalSigner`].
    #[must_use]
    pub fn signer(&self) -> &LocalSigner {
        &self.signer
    }

    /// On-chain address of this wallet's key.
    #[must_use]
    pub fn address(&self) -> Address {
        self.signer.address()
    }

    /// FALCON-512 public key.
    #[must_use]
    pub fn pubkey(&self) -> FalconPubkey {
        self.signer.pubkey()
    }

    /// Encrypt this wallet into a fresh canonical [`Keystore`] vault
    /// holding a single account named `account_name`.
    ///
    /// Uses a fresh random salt (16 B) + nonce (12 B), so the same
    /// wallet + password produces a *different* ciphertext each call.
    ///
    /// # Errors
    /// Returns [`SdkError::Other`] only on RNG / cipher failures.
    pub fn to_keystore(&self, account_name: &str, password: &str) -> Result<Keystore, SdkError> {
        let mut keystore = Keystore::new();
        self.add_to_keystore(&mut keystore, account_name, password)?;
        Ok(keystore)
    }

    /// Add this wallet to an existing keystore vault under
    /// `account_name`, encrypting under `password`. Overwrites any
    /// entry already stored under that name.
    ///
    /// # Errors
    /// Returns [`SdkError::Other`] on RNG / cipher failure.
    pub fn add_to_keystore(
        &self,
        keystore: &mut Keystore,
        account_name: &str,
        password: &str,
    ) -> Result<(), SdkError> {
        let entry = encrypt_entry(&self.pubkey(), self.signer.secret(), password)?;
        keystore.accounts.insert(account_name.to_string(), entry);
        Ok(())
    }

    /// Decrypt the account named `account_name` from a canonical
    /// [`Keystore`] vault.
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] if the vault version is
    ///   unsupported, the account name is absent, the KDF is not
    ///   Argon2id or exceeds the accepted upper bound, the cipher suite
    ///   is not `aes-256-gcm`, hex is malformed, or the password is wrong.
    /// - [`SdkError::Signing`] on a pubkey/secret mismatch.
    pub fn from_keystore(
        keystore: &Keystore,
        account_name: &str,
        password: &str,
    ) -> Result<Self, SdkError> {
        if keystore.version > KEYSTORE_VERSION {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported keystore version {}",
                keystore.version
            )));
        }
        let entry = keystore.accounts.get(account_name).ok_or_else(|| {
            SdkError::InvalidArgument(format!("no account named {account_name:?} in keystore"))
        })?;
        let (pubkey, secret) = decrypt_entry(entry, password)?;
        Self::from_keys(pubkey, secret)
    }

    /// Decrypt an account from raw keystore JSON, accepting both the
    /// canonical multi-account vault and the older nested
    /// single-account keystore this SDK wrote at `0.1.0`.
    ///
    /// For the canonical format, `account_name` selects the entry. For
    /// the legacy single-account format there is only one key, so
    /// `account_name` is ignored.
    ///
    /// # Errors
    /// [`SdkError::InvalidArgument`] if the JSON matches neither format
    /// or decryption fails; [`SdkError::Signing`] on key mismatch.
    pub fn from_keystore_json(
        json: &str,
        account_name: &str,
        password: &str,
    ) -> Result<Self, SdkError> {
        // Canonical vault: a non-empty `accounts` map is the tell.
        if let Ok(keystore) = serde_json::from_str::<Keystore>(json) {
            if !keystore.accounts.is_empty() {
                return Self::from_keystore(&keystore, account_name, password);
            }
        }
        // Legacy nested single-account keystore (this SDK, 0.1.0).
        if let Ok(legacy) = serde_json::from_str::<LegacyKeystore>(json) {
            return Self::from_legacy(&legacy, password);
        }
        Err(SdkError::InvalidArgument(
            "unrecognized keystore format (neither canonical vault nor legacy 0.1.0)".into(),
        ))
    }

    /// Decrypt the legacy nested single-account keystore written by
    /// this SDK at `0.1.0` (`kdf.params.{m,t,p}` + nested `cipher`
    /// object + address bound as AEAD associated data).
    fn from_legacy(legacy: &LegacyKeystore, password: &str) -> Result<Self, SdkError> {
        if legacy.version != 1 {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported legacy keystore version {}",
                legacy.version
            )));
        }
        if legacy.kdf.name != "argon2id" {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported KDF: {}",
                legacy.kdf.name
            )));
        }
        if legacy.cipher.name != "aes-256-gcm" {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported cipher: {}",
                legacy.cipher.name
            )));
        }
        let salt = decode_hex(&legacy.kdf.params.salt, SALT_LEN, "salt")?;
        let nonce_bytes = decode_hex(&legacy.cipher.nonce, AES_NONCE_LEN, "nonce")?;
        let ciphertext = decode_hex_var(&legacy.cipher.ciphertext, "ciphertext")?;
        let pubkey = FalconPubkey::from_hex(&legacy.pubkey)?;
        let address = Address::from_hex(&legacy.address)?;
        if Address::from_pubkey(&pubkey) != address {
            return Err(SdkError::InvalidArgument(
                "keystore address does not match pubkey".into(),
            ));
        }
        let mut key = derive_key(
            password,
            &salt,
            legacy.kdf.params.m,
            legacy.kdf.params.t,
            legacy.kdf.params.p,
        )?;
        // 0.1.0 bound the address hex as AEAD associated data.
        let aad = address.to_hex().into_bytes();
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| SdkError::Other(format!("AES key init failed: {e}")))?;
        let plaintext = cipher
            .decrypt(
                Nonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                SdkError::InvalidArgument(
                    "decrypt failed — bad password or corrupt keystore".into(),
                )
            });
        key.zeroize();
        let secret = secret_from_plaintext(plaintext?)?;
        Self::from_keys(pubkey, secret)
    }
}

#[async_trait]
impl Signer for Wallet {
    fn address(&self) -> Address {
        self.signer.address()
    }

    fn pubkey(&self) -> FalconPubkey {
        self.signer.pubkey()
    }

    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError> {
        self.signer.sign_hash(hash).await
    }

    async fn sign_tx(&self, tx: &mut Tx) -> Result<(), SdkError> {
        self.signer.sign_tx(tx).await
    }
}

impl std::fmt::Debug for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallet")
            .field("address", &self.signer.address())
            .field("pubkey", &self.signer.pubkey())
            .finish()
    }
}

// ── Canonical keystore format ──────────────────────────────────

/// Current keystore schema version. Bumped only on breaking format
/// changes; additive fields use serde defaults and never bump it.
pub const KEYSTORE_VERSION: u32 = 1;

/// Argon2id memory cost in KiB (64 MiB) — the pinned floor.
pub const ARGON2_MEMORY_KB: u32 = 64 * 1024;

/// Argon2id time cost (iterations) — the pinned floor.
pub const ARGON2_ITERATIONS: u32 = 3;

/// Argon2id parallelism (lanes) — the pinned floor.
pub const ARGON2_PARALLELISM: u32 = 4;

/// Anti-DoS upper bound on `memory_kb` accepted from an untrusted
/// keystore (1 GiB). Matches the reference implementations' clamp.
pub const ARGON2_MAX_MEMORY_KB: u32 = 1_048_576;

/// Anti-DoS upper bound on `iterations` accepted from an untrusted
/// keystore. Matches the reference implementations' clamp.
pub const ARGON2_MAX_ITERATIONS: u32 = 16;

/// Anti-DoS upper bound on `parallelism` accepted from an untrusted
/// keystore. Matches the reference implementations' clamp.
pub const ARGON2_MAX_PARALLELISM: u32 = 16;

/// AES-256-GCM key length in bytes.
const AES_KEY_LEN: usize = 32;

/// AES-256-GCM / ChaCha20-Poly1305 nonce length in bytes.
const AES_NONCE_LEN: usize = 12;

/// Argon2id salt length in bytes.
const SALT_LEN: usize = 16;

fn default_cipher_aes() -> String {
    "aes-256-gcm".to_string()
}

/// The canonical Pyde account keystore: a multi-account JSON vault.
///
/// Shared byte-for-byte with `otigen-wallet`, `pyde-ts-sdk`, the
/// playground, and the wallet. Single-account tools still write this
/// envelope with one entry, which is what makes "one file opens
/// everywhere" hold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keystore {
    /// Envelope version. Currently [`KEYSTORE_VERSION`].
    pub version: u32,
    /// Encrypted accounts, keyed by account name.
    #[serde(default)]
    pub accounts: BTreeMap<String, KeystoreEntry>,
}

impl Default for Keystore {
    fn default() -> Self {
        Self::new()
    }
}

impl Keystore {
    /// A fresh, empty vault at the current version.
    #[must_use]
    pub fn new() -> Self {
        Self {
            version: KEYSTORE_VERSION,
            accounts: BTreeMap::new(),
        }
    }

    /// Names of the accounts held in this vault.
    #[must_use]
    pub fn account_names(&self) -> Vec<&str> {
        self.accounts.keys().map(String::as_str).collect()
    }
}

/// One password-encrypted account entry in a [`Keystore`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeystoreEntry {
    /// `0x` + 64 hex chars — the account address.
    pub address: String,
    /// `0x` + hex of the FALCON-512 public-key bytes.
    pub pubkey: String,
    /// `0x` + hex of `AES-256-GCM(secret_key)` with the 16-byte tag
    /// appended.
    pub ciphertext: String,
    /// `0x` + 32 hex chars (16 bytes) — Argon2id salt.
    pub salt: String,
    /// `0x` + 24 hex chars (12 bytes) — AEAD nonce.
    pub nonce: String,
    /// Cipher-suite id. Writers emit `"aes-256-gcm"`; absent means
    /// `"aes-256-gcm"` (back-compat with keystores that omit it).
    #[serde(default = "default_cipher_aes")]
    pub cipher: String,
    /// KDF parameters, embedded per entry.
    pub kdf: KdfParams,
}

/// Argon2id parameters for a keystore entry. Flat; the salt lives at
/// the entry level, not nested here.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// KDF name — always `"argon2id"`.
    pub name: String,
    /// Memory cost in KiB.
    pub memory_kb: u32,
    /// Time cost (iterations).
    pub iterations: u32,
    /// Parallelism (lanes).
    pub parallelism: u32,
}

impl KdfParams {
    /// The current pinned parameters embedded into every new entry.
    #[must_use]
    pub fn current() -> Self {
        Self {
            name: "argon2id".into(),
            memory_kb: ARGON2_MEMORY_KB,
            iterations: ARGON2_ITERATIONS,
            parallelism: ARGON2_PARALLELISM,
        }
    }
}

// ── Encrypt / decrypt ──────────────────────────────────────────

/// Encrypt a wallet's secret key into a canonical [`KeystoreEntry`]
/// (AES-256-GCM, no AAD, current Argon2id params).
fn encrypt_entry(
    pubkey: &FalconPubkey,
    secret: &FalconSecret,
    password: &str,
) -> Result<KeystoreEntry, SdkError> {
    let mut rng = rand::thread_rng();
    let mut salt = [0u8; SALT_LEN];
    rng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; AES_NONCE_LEN];
    rng.fill_bytes(&mut nonce_bytes);

    let mut key = derive_key(
        password,
        &salt,
        ARGON2_MEMORY_KB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
    )?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| SdkError::Other(format!("AES key init failed: {e}")))?;
    // No associated data — cross-implementation interchange.
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), &secret.as_bytes()[..])
        .map_err(|e| SdkError::Other(format!("AES encrypt failed: {e}")))?;
    key.zeroize();

    let address = Address::from_pubkey(pubkey);
    Ok(KeystoreEntry {
        address: address.to_hex(),
        pubkey: pubkey.to_hex(),
        ciphertext: format!("0x{}", hex::encode(&ciphertext)),
        salt: format!("0x{}", hex::encode(salt)),
        nonce: format!("0x{}", hex::encode(nonce_bytes)),
        cipher: default_cipher_aes(),
        kdf: KdfParams::current(),
    })
}

/// Decrypt a canonical [`KeystoreEntry`]: enforce the Argon2id floor,
/// dispatch on the allowlisted cipher suite, return the pubkey +
/// secret.
fn decrypt_entry(
    entry: &KeystoreEntry,
    password: &str,
) -> Result<(FalconPubkey, FalconSecret), SdkError> {
    // Only Argon2id is recognised.
    if entry.kdf.name != "argon2id" {
        return Err(SdkError::InvalidArgument(format!(
            "unsupported KDF: {}",
            entry.kdf.name
        )));
    }
    // Anti-DoS upper clamp on params read from an untrusted file — a
    // crafted `memory_kb` would otherwise force a multi-GiB allocation.
    // There is deliberately NO lower floor reject: bricking a
    // legitimately-owned below-floor keystore helps nobody, and writers
    // always emit the floor (64 MiB / 3 / 4). Bounds match the reference
    // implementations.
    if entry.kdf.memory_kb > ARGON2_MAX_MEMORY_KB
        || entry.kdf.iterations > ARGON2_MAX_ITERATIONS
        || entry.kdf.parallelism > ARGON2_MAX_PARALLELISM
    {
        return Err(SdkError::InvalidArgument(
            "keystore KDF parameters exceed the accepted upper bound".into(),
        ));
    }

    let salt = decode_hex(&entry.salt, SALT_LEN, "salt")?;
    let nonce_bytes = decode_hex(&entry.nonce, AES_NONCE_LEN, "nonce")?;
    let ciphertext = decode_hex_var(&entry.ciphertext, "ciphertext")?;
    let pubkey = FalconPubkey::from_hex(&entry.pubkey)?;
    let address = Address::from_hex(&entry.address)?;
    if Address::from_pubkey(&pubkey) != address {
        return Err(SdkError::InvalidArgument(
            "keystore address does not match pubkey".into(),
        ));
    }

    let mut key = derive_key(
        password,
        &salt,
        entry.kdf.memory_kb,
        entry.kdf.iterations,
        entry.kdf.parallelism,
    )?;
    // Only AES-256-GCM is accepted (the sole cipher any conformant tool
    // writes). Single error for wrong-password AND tamper (no oracle).
    // No AAD.
    if entry.cipher != "aes-256-gcm" {
        key.zeroize();
        return Err(SdkError::InvalidArgument(format!(
            "unsupported cipher suite: {}",
            entry.cipher
        )));
    }
    let plaintext = aes_decrypt(&key, &nonce_bytes, &ciphertext);
    key.zeroize();
    let secret = secret_from_plaintext(plaintext?)?;
    Ok((pubkey, secret))
}

/// AES-256-GCM decrypt with no associated data. Single error variant.
fn aes_decrypt(
    key: &[u8; AES_KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, SdkError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| SdkError::Other(format!("AES key init failed: {e}")))?;
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| {
            SdkError::InvalidArgument("decrypt failed — bad password or corrupt keystore".into())
        })
}

/// Convert a decrypted plaintext buffer into a [`FalconSecret`],
/// checking the length and scrubbing the buffer.
fn secret_from_plaintext(plaintext: Vec<u8>) -> Result<FalconSecret, SdkError> {
    if plaintext.len() != FALCON_SECRET_LEN {
        return Err(SdkError::InvalidArgument(format!(
            "decrypted secret is {} bytes, expected {FALCON_SECRET_LEN}",
            plaintext.len()
        )));
    }
    let mut sk_bytes = [0u8; FALCON_SECRET_LEN];
    sk_bytes.copy_from_slice(&plaintext);
    let mut zeroable = plaintext;
    zeroable.zeroize();
    Ok(FalconSecret::new(sk_bytes))
}

/// Derive a 32-byte key from `password` + `salt` via Argon2id (version
/// 0x13) under the given parameters. Byte-identical to
/// `otigen-wallet::kdf::derive` for the pinned parameters.
fn derive_key(
    password: &str,
    salt: &[u8],
    m_kib: u32,
    t: u32,
    p: u32,
) -> Result<[u8; AES_KEY_LEN], SdkError> {
    let params = Params::new(m_kib, t, p, Some(AES_KEY_LEN))
        .map_err(|e| SdkError::InvalidArgument(format!("argon2 params invalid: {e}")))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; AES_KEY_LEN];
    argon2
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| SdkError::Other(format!("argon2 KDF failed: {e}")))?;
    Ok(key)
}

fn decode_hex(s: &str, expected_len: usize, label: &str) -> Result<Vec<u8>, SdkError> {
    let stripped = s.trim_start_matches("0x");
    let bytes = hex::decode(stripped)
        .map_err(|e| SdkError::InvalidArgument(format!("bad {label} hex: {e}")))?;
    if bytes.len() != expected_len {
        return Err(SdkError::InvalidArgument(format!(
            "{label}: expected {expected_len} bytes, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn decode_hex_var(s: &str, label: &str) -> Result<Vec<u8>, SdkError> {
    let stripped = s.trim_start_matches("0x");
    hex::decode(stripped).map_err(|e| SdkError::InvalidArgument(format!("bad {label} hex: {e}")))
}

// ── Legacy nested keystore (this SDK, 0.1.0) ───────────────────

/// The nested single-account keystore this SDK wrote at `0.1.0`. Read
/// only, for migration. Distinct shape: `kdf.params.{m,t,p,salt}` and a
/// nested `cipher` object, with the address bound as AEAD AAD.
#[derive(Deserialize)]
struct LegacyKeystore {
    version: u32,
    address: String,
    pubkey: String,
    kdf: LegacyKdf,
    cipher: LegacyCipher,
}

#[derive(Deserialize)]
struct LegacyKdf {
    name: String,
    params: LegacyArgon2Params,
}

#[derive(Deserialize)]
struct LegacyArgon2Params {
    m: u32,
    t: u32,
    p: u32,
    salt: String,
}

#[derive(Deserialize)]
struct LegacyCipher {
    name: String,
    nonce: String,
    ciphertext: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::tx::{tx_hash, TxBuilder};

    #[test]
    fn wallet_drop_wipes_secret() {
        assert!(core::mem::needs_drop::<Wallet>());
        assert!(core::mem::needs_drop::<LocalSigner>());
        assert!(core::mem::needs_drop::<FalconSecret>());
        for _ in 0..16 {
            let w = Wallet::generate().unwrap();
            let _addr = w.address();
            drop(w);
        }
    }

    #[tokio::test]
    async fn wallet_signs_a_tx_end_to_end() {
        let wallet = Wallet::generate().unwrap();
        let to = Address::new([0x55; 32]);
        let mut tx = TxBuilder::new()
            .from(wallet.address())
            .transfer(to, 1_500_000_000)
            .build()
            .unwrap();
        let pre = tx_hash(&tx);
        wallet.sign_tx(&mut tx).await.unwrap();
        let post = tx_hash(&tx);
        assert_eq!(pre, post, "signature must not affect tx_hash");
        assert!(!tx.signature.is_empty());
    }

    #[test]
    fn keystore_round_trip() {
        let wallet = Wallet::generate().unwrap();
        let address = wallet.address();
        let pubkey = wallet.pubkey();
        let pw = "correct horse battery staple";
        let ks = wallet.to_keystore("acct", pw).unwrap();
        let restored = Wallet::from_keystore(&ks, "acct", pw).unwrap();
        assert_eq!(restored.address(), address);
        assert_eq!(restored.pubkey(), pubkey);
    }

    #[test]
    fn keystore_uses_canonical_params_and_shape() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("acct", "pw").unwrap();
        assert_eq!(ks.version, KEYSTORE_VERSION);
        let entry = ks.accounts.get("acct").unwrap();
        assert_eq!(entry.cipher, "aes-256-gcm");
        assert_eq!(entry.kdf.name, "argon2id");
        assert_eq!(entry.kdf.memory_kb, 65_536);
        assert_eq!(entry.kdf.iterations, 3);
        assert_eq!(entry.kdf.parallelism, 4);
        assert!(entry.salt.starts_with("0x"));
        assert!(entry.nonce.starts_with("0x"));
        assert!(entry.ciphertext.starts_with("0x"));
    }

    #[test]
    fn keystore_wrong_password_fails() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("acct", "right password").unwrap();
        let err = Wallet::from_keystore(&ks, "acct", "wrong password").unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(_)));
    }

    #[test]
    fn keystore_unknown_account_fails() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("acct", "pw").unwrap();
        assert!(Wallet::from_keystore(&ks, "nope", "pw").is_err());
    }

    #[test]
    fn keystore_json_round_trip() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("acct", "pw").unwrap();
        let json = serde_json::to_string(&ks).unwrap();
        let restored = Wallet::from_keystore_json(&json, "acct", "pw").unwrap();
        assert_eq!(restored.address(), wallet.address());
    }

    #[test]
    fn multi_account_vault() {
        let a = Wallet::generate().unwrap();
        let b = Wallet::generate().unwrap();
        let mut ks = a.to_keystore("a", "pw").unwrap();
        b.add_to_keystore(&mut ks, "b", "pw").unwrap();
        assert_eq!(ks.account_names().len(), 2);
        assert_eq!(
            Wallet::from_keystore(&ks, "a", "pw").unwrap().address(),
            a.address()
        );
        assert_eq!(
            Wallet::from_keystore(&ks, "b", "pw").unwrap().address(),
            b.address()
        );
    }

    #[test]
    fn keystore_different_writes_differ() {
        let wallet = Wallet::generate().unwrap();
        let a = wallet.to_keystore("x", "pw").unwrap();
        let b = wallet.to_keystore("x", "pw").unwrap();
        let ea = a.accounts.get("x").unwrap();
        let eb = b.accounts.get("x").unwrap();
        assert_ne!(ea.ciphertext, eb.ciphertext);
        assert_ne!(ea.salt, eb.salt);
        assert_ne!(ea.nonce, eb.nonce);
    }

    #[test]
    fn rejects_above_clamp_kdf() {
        let wallet = Wallet::generate().unwrap();
        let mut ks = wallet.to_keystore("acct", "pw").unwrap();
        // Above the anti-DoS upper bound (2 GiB > 1 GiB) — must be
        // rejected before any allocation.
        ks.accounts.get_mut("acct").unwrap().kdf.memory_kb = 2_097_152;
        let err = Wallet::from_keystore(&ks, "acct", "pw").unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(ref m) if m.contains("upper bound")));
    }

    #[test]
    fn accepts_below_floor_keystore() {
        // A legitimately-owned keystore sealed with below-floor params
        // (m=8 MiB, t=1, p=1) must still open — no lower floor reject.
        let wallet = Wallet::generate().unwrap();
        let pw = "pw";
        let (m, t, p) = (8 * 1024, 1, 1);
        let mut salt = [0u8; SALT_LEN];
        let mut nonce = [0u8; AES_NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut salt);
        rand::thread_rng().fill_bytes(&mut nonce);
        let key = derive_key(pw, &salt, m, t, p).unwrap();
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let ct = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                &wallet.signer().secret().as_bytes()[..],
            )
            .unwrap();
        let addr = wallet.address();
        let entry = KeystoreEntry {
            address: addr.to_hex(),
            pubkey: wallet.pubkey().to_hex(),
            ciphertext: format!("0x{}", hex::encode(&ct)),
            salt: format!("0x{}", hex::encode(salt)),
            nonce: format!("0x{}", hex::encode(nonce)),
            cipher: "aes-256-gcm".into(),
            kdf: KdfParams {
                name: "argon2id".into(),
                memory_kb: m,
                iterations: t,
                parallelism: p,
            },
        };
        let mut ks = Keystore::new();
        ks.accounts.insert("acct".into(), entry);
        let restored = Wallet::from_keystore(&ks, "acct", pw).unwrap();
        assert_eq!(restored.address(), addr);
    }

    #[test]
    fn unsupported_version_rejected() {
        let wallet = Wallet::generate().unwrap();
        let mut ks = wallet.to_keystore("acct", "pw").unwrap();
        ks.version = 999;
        assert!(Wallet::from_keystore(&ks, "acct", "pw").is_err());
    }
}
