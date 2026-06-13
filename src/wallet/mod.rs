//! `Wallet` + encrypted [`Keystore`].
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
//! // Persist the wallet, password-protected.
//! let keystore = wallet.to_keystore("correct horse battery staple")?;
//! std::fs::write("wallet.json", serde_json::to_vec_pretty(&keystore)?)?;
//!
//! // Load it back.
//! let stored: pyde_rust_sdk::wallet::Keystore =
//!     serde_json::from_slice(&std::fs::read("wallet.json")?)?;
//! let wallet = Wallet::from_keystore(&stored, "correct horse battery staple")?;
//! ```
//!
//! ## Keystore format
//!
//! JSON envelope, version-tagged so future cipher / KDF upgrades
//! can be additive:
//!
//! ```json
//! {
//!   "version": 1,
//!   "address": "0x…",
//!   "pubkey": "0x…",
//!   "kdf": { "name": "argon2id", "params": { "m":65536,"t":3,"p":1,"salt":"0x…" } },
//!   "cipher": { "name":"aes-256-gcm","nonce":"0x…","ciphertext":"0x…" }
//! }
//! ```
//!
//! - **KDF**: argon2id with `t=3, m=64 MiB, p=1` — current OWASP
//!   guidance.
//! - **Cipher**: AES-256-GCM. 12-byte random nonce per write; the
//!   GCM tag is appended to the ciphertext.
//! - **Wire-compatible with `pyde-ts-sdk`** so a wallet generated
//!   in the browser SDK loads here unchanged.

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
pub struct Wallet {
    signer: LocalSigner,
}

impl Wallet {
    /// Generate a fresh wallet from OS entropy.
    ///
    /// This is the default "create new account" path. Internally
    /// uses `getrandom` via [`pyde_crypto::falcon::falcon_keygen`].
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
    /// Typically called by [`Self::from_keystore`] after decrypting
    /// the secret bytes. Validates that the pubkey matches the
    /// secret by signing a probe hash and verifying it.
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

    /// Encrypt the secret key under `password` and produce a
    /// JSON-serialisable [`Keystore`].
    ///
    /// Uses fresh random salt (16 B) + nonce (12 B). Same wallet
    /// + same password produces a *different* keystore each call.
    ///
    /// # Errors
    /// Returns [`SdkError::Other`] only on RNG / cipher failures
    /// (effectively unreachable on commodity hardware).
    pub fn to_keystore(&self, password: &str) -> Result<Keystore, SdkError> {
        Keystore::encrypt(&self.pubkey(), self.signer.secret(), password)
    }

    /// Decrypt a [`Keystore`] with `password` and load the wallet.
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidArgument`] on bad-password / KDF
    /// failure, [`SdkError::Signing`] on a pubkey/secret mismatch
    /// (corrupt keystore).
    pub fn from_keystore(keystore: &Keystore, password: &str) -> Result<Self, SdkError> {
        let (pubkey, secret) = keystore.decrypt(password)?;
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

// ── Keystore ───────────────────────────────────────────────────

/// Current keystore format version. Bump when changing the JSON
/// shape in a non-backward-compatible way.
pub const KEYSTORE_VERSION: u32 = 1;

/// Argon2id memory cost in KiB — 64 MiB. OWASP-recommended floor
/// for password-encrypted secrets.
pub const ARGON2_M_KIB: u32 = 64 * 1024;

/// Argon2id time cost (iterations).
pub const ARGON2_T: u32 = 3;

/// Argon2id parallelism (lanes).
pub const ARGON2_P: u32 = 1;

/// Argon2id salt length in bytes.
pub const ARGON2_SALT_LEN: usize = 16;

/// AES-256-GCM key length in bytes.
const AES_KEY_LEN: usize = 32;

/// AES-256-GCM nonce length in bytes.
const AES_NONCE_LEN: usize = 12;

/// JSON envelope for an encrypted wallet on disk.
///
/// Field shape is wire-compatible with `pyde-ts-sdk`'s keystore so a
/// wallet generated in one SDK loads cleanly in the other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keystore {
    /// Envelope version. Currently [`KEYSTORE_VERSION`].
    pub version: u32,
    /// Wallet address, hex.
    pub address: String,
    /// FALCON-512 pubkey, hex — 897 bytes.
    pub pubkey: String,
    /// KDF parameters used to derive the AES key.
    pub kdf: KdfParams,
    /// Cipher parameters and ciphertext.
    pub cipher: CipherParams,
}

/// KDF parameters for the keystore (argon2id today).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// KDF name — always `"argon2id"` for v1 keystores.
    pub name: String,
    /// Algorithm-specific parameters.
    pub params: Argon2Params,
}

/// Argon2id parameter set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Argon2Params {
    /// Memory cost in KiB.
    pub m: u32,
    /// Time cost (iterations).
    pub t: u32,
    /// Parallelism (lanes).
    pub p: u32,
    /// Salt bytes, hex. 16 bytes.
    pub salt: String,
}

/// Cipher parameters + ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CipherParams {
    /// Cipher name — always `"aes-256-gcm"` for v1 keystores.
    pub name: String,
    /// AES-GCM nonce, hex. 12 bytes.
    pub nonce: String,
    /// Ciphertext including the appended GCM tag, hex.
    pub ciphertext: String,
}

impl Keystore {
    /// Encrypt `secret` under `password` and produce a keystore.
    ///
    /// # Errors
    /// Returns [`SdkError::Other`] on KDF / cipher failure.
    pub fn encrypt(
        pubkey: &FalconPubkey,
        secret: &FalconSecret,
        password: &str,
    ) -> Result<Self, SdkError> {
        let mut rng = rand::thread_rng();
        let mut salt = [0u8; ARGON2_SALT_LEN];
        rng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; AES_NONCE_LEN];
        rng.fill_bytes(&mut nonce_bytes);

        let mut key = derive_aes_key(password, &salt)?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| SdkError::Other(format!("AES key init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        // Bind the pubkey hex into the AEAD as associated data so
        // tampering with `pubkey` after encryption invalidates the
        // tag.
        let address = Address::from_pubkey(pubkey);
        let aad = address.to_hex().into_bytes();

        let ciphertext = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: secret.as_bytes(),
                    aad: &aad,
                },
            )
            .map_err(|e| SdkError::Other(format!("AES encrypt failed: {e}")))?;

        key.zeroize();

        Ok(Self {
            version: KEYSTORE_VERSION,
            address: address.to_hex(),
            pubkey: pubkey.to_hex(),
            kdf: KdfParams {
                name: "argon2id".into(),
                params: Argon2Params {
                    m: ARGON2_M_KIB,
                    t: ARGON2_T,
                    p: ARGON2_P,
                    salt: format!("0x{}", hex::encode(salt)),
                },
            },
            cipher: CipherParams {
                name: "aes-256-gcm".into(),
                nonce: format!("0x{}", hex::encode(nonce_bytes)),
                ciphertext: format!("0x{}", hex::encode(&ciphertext)),
            },
        })
    }

    /// Decrypt the keystore under `password` and return the pubkey
    /// + secret.
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] on unknown version, unknown
    ///   KDF / cipher name, or malformed hex.
    /// - [`SdkError::InvalidArgument`] on a wrong password (the GCM
    ///   tag fails to verify).
    pub fn decrypt(&self, password: &str) -> Result<(FalconPubkey, FalconSecret), SdkError> {
        if self.version != KEYSTORE_VERSION {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported keystore version {}",
                self.version
            )));
        }
        if self.kdf.name != "argon2id" {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported KDF: {}",
                self.kdf.name
            )));
        }
        if self.cipher.name != "aes-256-gcm" {
            return Err(SdkError::InvalidArgument(format!(
                "unsupported cipher: {}",
                self.cipher.name
            )));
        }

        let salt = decode_hex(&self.kdf.params.salt, ARGON2_SALT_LEN, "salt")?;
        let nonce_bytes = decode_hex(&self.cipher.nonce, AES_NONCE_LEN, "nonce")?;
        let ciphertext = decode_hex_var(&self.cipher.ciphertext, "ciphertext")?;
        let pubkey = FalconPubkey::from_hex(&self.pubkey)?;
        let address = Address::from_hex(&self.address)?;
        if Address::from_pubkey(&pubkey) != address {
            return Err(SdkError::InvalidArgument(
                "keystore address does not match pubkey".into(),
            ));
        }
        let aad = address.to_hex().into_bytes();

        // Allow the keystore to override KDF params (for forward
        // compat with stronger defaults shipped later) but validate
        // they're sane.
        let m = self.kdf.params.m;
        let t = self.kdf.params.t;
        let p = self.kdf.params.p;
        let mut key = derive_aes_key_with_params(password, &salt, m, t, p)?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|e| SdkError::Other(format!("AES key init failed: {e}")))?;
        let nonce = Nonce::from_slice(&nonce_bytes);

        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                SdkError::InvalidArgument(
                    "decrypt failed — bad password or corrupt keystore".into(),
                )
            })?;

        key.zeroize();

        if plaintext.len() != FALCON_SECRET_LEN {
            return Err(SdkError::InvalidArgument(format!(
                "decrypted secret is {} bytes, expected {FALCON_SECRET_LEN}",
                plaintext.len()
            )));
        }
        let mut sk_bytes = [0u8; FALCON_SECRET_LEN];
        sk_bytes.copy_from_slice(&plaintext);
        // Don't leave the plaintext buffer holding secret bytes.
        let mut zeroable = plaintext;
        zeroable.zeroize();

        Ok((pubkey, FalconSecret::new(sk_bytes)))
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Derive a 32-byte AES key from `password` + `salt` via argon2id
/// with the default parameters ([`ARGON2_M_KIB`], [`ARGON2_T`],
/// [`ARGON2_P`]).
fn derive_aes_key(password: &str, salt: &[u8]) -> Result<[u8; AES_KEY_LEN], SdkError> {
    derive_aes_key_with_params(password, salt, ARGON2_M_KIB, ARGON2_T, ARGON2_P)
}

/// Derive a 32-byte AES key from `password` + `salt` under the given
/// argon2id parameters. Used by the keystore's decrypt path so a
/// keystore stamped with different params still decrypts.
fn derive_aes_key_with_params(
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::tx::{tx_hash, TxBuilder};

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
        let ks = wallet.to_keystore(pw).unwrap();
        let restored = Wallet::from_keystore(&ks, pw).unwrap();
        assert_eq!(restored.address(), address);
        assert_eq!(restored.pubkey(), pubkey);
    }

    #[test]
    fn keystore_wrong_password_fails() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("right password").unwrap();
        let err = Wallet::from_keystore(&ks, "wrong password").unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(_)));
    }

    #[test]
    fn keystore_tamper_address_fails() {
        let wallet = Wallet::generate().unwrap();
        let mut ks = wallet.to_keystore("pw").unwrap();
        // Flip a hex digit in the address — GCM AAD binding makes
        // this fail at decrypt time.
        let mut chars: Vec<char> = ks.address.chars().collect();
        let idx = chars.len() - 4;
        chars[idx] = if chars[idx] == '0' { '1' } else { '0' };
        ks.address = chars.into_iter().collect();
        assert!(Wallet::from_keystore(&ks, "pw").is_err());
    }

    #[test]
    fn keystore_json_round_trip() {
        let wallet = Wallet::generate().unwrap();
        let ks = wallet.to_keystore("pw").unwrap();
        let json = serde_json::to_string(&ks).unwrap();
        let parsed: Keystore = serde_json::from_str(&json).unwrap();
        let restored = Wallet::from_keystore(&parsed, "pw").unwrap();
        assert_eq!(restored.address(), wallet.address());
    }

    #[test]
    fn keystore_different_writes_differ() {
        let wallet = Wallet::generate().unwrap();
        let a = wallet.to_keystore("pw").unwrap();
        let b = wallet.to_keystore("pw").unwrap();
        // Fresh random salt + nonce per write — ciphertexts differ.
        assert_ne!(a.cipher.ciphertext, b.cipher.ciphertext);
        assert_ne!(a.kdf.params.salt, b.kdf.params.salt);
        assert_ne!(a.cipher.nonce, b.cipher.nonce);
    }

    #[test]
    fn unsupported_version_rejected() {
        let wallet = Wallet::generate().unwrap();
        let mut ks = wallet.to_keystore("pw").unwrap();
        ks.version = 999;
        assert!(Wallet::from_keystore(&ks, "pw").is_err());
    }
}
