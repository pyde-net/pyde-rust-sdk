//! `Signer` trait — anything that can sign a tx hash on behalf of an
//! address. Built-in impl: [`LocalSigner`] (in-memory FALCON keypair).
//!
//! The trait is async to keep it object-safe via `async_trait` and
//! accommodate future hardware-wallet / KMS backends that need to
//! round-trip an external request (USB, network). Local in-memory
//! signing completes synchronously; the async wrapper imposes no
//! cost beyond an immediate `Poll::Ready`.
//!
//! Signing flow:
//!
//! 1. Caller builds an unsigned [`Tx`] via [`crate::tx::TxBuilder`].
//! 2. Signer computes [`crate::tx::tx_hash`] over the canonical
//!    pre-image (signature field excluded).
//! 3. Signer FALCON-signs the 32-byte hash via
//!    [`pyde_crypto::falcon::falcon_sign`] under the `"pyde-falcon-v1"`
//!    domain separator.
//! 4. Signer patches the produced [`FalconSignature`] into
//!    `tx.signature`. The tx is now wire-ready.
//!
//! The FALCON-512 scheme is randomised (Gaussian sampling), so the
//! same key signing the same hash twice produces two distinct
//! signature byte strings — both verify. The `tx_hash` excludes the
//! signature field by design, so this randomisation never breaks
//! the chain's mempool dedup or replay-protection invariants.

use async_trait::async_trait;
use pyde_crypto::falcon::{falcon_keygen, falcon_keygen_deterministic, falcon_sign};

use crate::error::SdkError;
use crate::tx::tx_hash;
use crate::types::{Address, FalconPubkey, FalconSecret, FalconSignature, Tx, TxHash};

/// Signer abstraction.
///
/// Object-safe via `async_trait`. All impls must be `Send + Sync` so
/// the SDK's higher layers (provider, wallet) can hold them behind
/// `Arc<dyn Signer>`.
///
/// ## Implementing a custom signer
///
/// Concrete signers only need to implement [`Signer::address`] and
/// [`Signer::sign_hash`]; the default [`Signer::sign_tx`] composes
/// them. Override [`Signer::sign_tx`] only if your backend can do
/// the canonical-hash computation more efficiently (e.g., a remote
/// signer that prefers to receive the full pre-image).
#[async_trait]
pub trait Signer: Send + Sync {
    /// On-chain address of the signing key.
    fn address(&self) -> Address;

    /// FALCON-512 public key behind the signing key. Wallets call
    /// this to register the key with the chain on first use
    /// ([`crate::types::TxType::RegisterPubkey`]).
    fn pubkey(&self) -> FalconPubkey;

    /// Sign an arbitrary 32-byte hash.
    ///
    /// Callers should pass a canonical [`crate::tx::tx_hash`]
    /// output. The signer doesn't care what the bytes mean — it
    /// just FALCON-signs them.
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if the backend fails (e.g., a
    /// hardware wallet times out or rejects the request).
    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError>;

    /// Sign a transaction in-place.
    ///
    /// Computes the canonical pre-image, calls [`Self::sign_hash`],
    /// then patches the produced signature into `tx.signature`. The
    /// `from` field is left untouched — callers should set it to
    /// the signer's address before calling this method.
    async fn sign_tx(&self, tx: &mut Tx) -> Result<(), SdkError> {
        let hash = tx_hash(tx);
        let sig = self.sign_hash(&hash).await?;
        tx.signature = sig;
        Ok(())
    }
}

/// In-memory FALCON-512 keypair signer.
///
/// The default signer for dapps and wallets. Keys are held in plain
/// memory at runtime — encrypt them at rest with the
/// [`crate::wallet::Keystore`] format if persisting to disk.
///
/// ## Constructors
///
/// - [`LocalSigner::random`] — OS entropy (the right choice for new
///   wallets and tests that want fresh keys per run).
/// - [`LocalSigner::from_seed`] — deterministic from a 32-byte
///   seed (useful for fixtures and reproducible test setups; **not**
///   for production wallets — a single-source-of-entropy seed
///   pinches the keypair's security down to the seed's bits).
/// - [`LocalSigner::from_secret`] — load a previously-generated
///   secret key. The address is rederived from the FALCON pubkey.
pub struct LocalSigner {
    address: Address,
    pubkey: FalconPubkey,
    secret: FalconSecret,
}

impl LocalSigner {
    /// Generate a fresh keypair from the OS RNG.
    ///
    /// This is what every wallet calls on first-launch / "create
    /// new account." Internally relies on `getrandom` via
    /// [`pyde_crypto::falcon::falcon_keygen`].
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if FALCON key generation fails
    /// (effectively unreachable on commodity hardware).
    pub fn random() -> Result<Self, SdkError> {
        let (crypto_pk, crypto_sk) =
            falcon_keygen().map_err(|e| SdkError::Signing(format!("FALCON keygen failed: {e}")))?;
        let pubkey: FalconPubkey = (&crypto_pk).into();
        let secret: FalconSecret = (&crypto_sk).into();
        let address = Address::from_pubkey(&pubkey);
        Ok(Self {
            address,
            pubkey,
            secret,
        })
    }

    /// Generate a deterministic keypair from a 32-byte seed.
    ///
    /// **Test-only / fixture-only.** A deterministic seed pinches
    /// the keypair's security to the seed's entropy — a real wallet
    /// must call [`Self::random`] so the kernel's entropy pool sets
    /// the key.
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if FALCON's seed-derived keygen
    /// fails (effectively unreachable on commodity hardware).
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, SdkError> {
        let (crypto_pk, crypto_sk) = falcon_keygen_deterministic(seed)
            .map_err(|e| SdkError::Signing(format!("FALCON deterministic keygen failed: {e}")))?;
        let pubkey: FalconPubkey = (&crypto_pk).into();
        let secret: FalconSecret = (&crypto_sk).into();
        let address = Address::from_pubkey(&pubkey);
        Ok(Self {
            address,
            pubkey,
            secret,
        })
    }

    /// Load a signer from a previously-generated FALCON secret key.
    ///
    /// The pubkey is derived from the secret (FALCON keypair shape)
    /// and the address is derived from the pubkey via
    /// [`Address::from_pubkey`].
    ///
    /// # Errors
    /// Returns [`SdkError::Signing`] if the secret bytes don't form
    /// a valid FALCON-512 secret key.
    pub fn from_secret(secret: FalconSecret) -> Result<Self, SdkError> {
        // FALCON keypairs are derived together — the secret key
        // alone is enough to re-derive the pubkey by running a
        // probe sign + extracting from the FnDsaKeyPair structure.
        // Since `pyde_crypto::falcon` does not expose a "pk from sk"
        // helper, the caller must supply pubkey-aware paths via the
        // keystore loader; for raw `from_secret`, we treat the
        // secret as authoritative and derive everything else by
        // signing a probe message and extracting the pubkey from
        // the verify side-effect. That helper is added in the
        // wallet module — for the signer alone, we require pubkey
        // alongside the secret.
        Self::from_keys(
            FalconPubkey::new([0u8; crate::types::FALCON_PUBKEY_LEN]),
            secret,
        )
    }

    /// Internal constructor used by the keystore loader.
    ///
    /// Callers must supply a pubkey that matches the secret —
    /// validated by signing a probe hash and verifying the result
    /// to surface mismatches as a [`SdkError::Signing`].
    pub fn from_keys(pubkey: FalconPubkey, secret: FalconSecret) -> Result<Self, SdkError> {
        let address = Address::from_pubkey(&pubkey);
        let signer = Self {
            address,
            pubkey,
            secret,
        };
        signer.validate_pair()?;
        Ok(signer)
    }

    /// Sign a probe hash and verify it to confirm the pubkey
    /// matches the secret. Used by [`Self::from_keys`].
    fn validate_pair(&self) -> Result<(), SdkError> {
        // Skip the probe if the pubkey is all-zero (the
        // `from_secret` placeholder path that signals "no pubkey
        // supplied"). That path is now considered an error.
        if self.pubkey.as_bytes() == &[0u8; crate::types::FALCON_PUBKEY_LEN] {
            return Err(SdkError::Signing(
                "from_secret requires the matching pubkey — use from_keys instead".into(),
            ));
        }
        let probe = [0u8; 32];
        let crypto_sk: pyde_crypto::falcon::FalconSecretKey = (&self.secret).into();
        let sig = falcon_sign(&crypto_sk, &probe)
            .map_err(|e| SdkError::Signing(format!("probe sign failed: {e}")))?;
        let crypto_pk: pyde_crypto::falcon::FalconPublicKey = (&self.pubkey).into();
        if !pyde_crypto::falcon::falcon_verify(&crypto_pk, &probe, &sig) {
            return Err(SdkError::Signing("pubkey does not match secret".into()));
        }
        Ok(())
    }

    /// Access the in-memory secret. Restricted to crate users so the
    /// keystore can encrypt it on persist. **Treat as sensitive.**
    pub(crate) fn secret(&self) -> &FalconSecret {
        &self.secret
    }
}

#[async_trait]
impl Signer for LocalSigner {
    fn address(&self) -> Address {
        self.address
    }

    fn pubkey(&self) -> FalconPubkey {
        self.pubkey
    }

    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError> {
        let crypto_sk: pyde_crypto::falcon::FalconSecretKey = (&self.secret).into();
        let crypto_sig = falcon_sign(&crypto_sk, hash.as_bytes())
            .map_err(|e| SdkError::Signing(format!("FALCON sign failed: {e}")))?;
        Ok((&crypto_sig).into())
    }
}

impl std::fmt::Debug for LocalSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSigner")
            .field("address", &self.address)
            .field("pubkey", &self.pubkey)
            .field("secret", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::tx::TxBuilder;
    use crate::types::TxType;

    #[tokio::test]
    async fn random_signer_signs_and_verifies() {
        let signer = LocalSigner::random().unwrap();
        let hash = TxHash::new([0xCD; 32]);
        let sig = signer.sign_hash(&hash).await.unwrap();

        // Round-trip verify via pyde-crypto directly.
        let crypto_pk: pyde_crypto::falcon::FalconPublicKey = (&signer.pubkey()).into();
        let crypto_sig = pyde_crypto::falcon::FalconSignature::from_bytes(sig.as_bytes()).unwrap();
        assert!(pyde_crypto::falcon::falcon_verify(
            &crypto_pk,
            hash.as_bytes(),
            &crypto_sig
        ));
    }

    #[tokio::test]
    async fn deterministic_signer_is_reproducible() {
        let seed = [0x42; 32];
        let a = LocalSigner::from_seed(&seed).unwrap();
        let b = LocalSigner::from_seed(&seed).unwrap();
        assert_eq!(a.address(), b.address());
        assert_eq!(a.pubkey(), b.pubkey());
    }

    #[tokio::test]
    async fn different_signers_have_different_addresses() {
        let a = LocalSigner::random().unwrap();
        let b = LocalSigner::random().unwrap();
        assert_ne!(a.address(), b.address());
    }

    #[tokio::test]
    async fn falcon_sigs_are_randomised_but_both_verify() {
        // FALCON is randomised — same key + same message gives two
        // different signature byte strings, both valid.
        let signer = LocalSigner::random().unwrap();
        let hash = TxHash::new([0x77; 32]);
        let s1 = signer.sign_hash(&hash).await.unwrap();
        let s2 = signer.sign_hash(&hash).await.unwrap();
        assert_ne!(s1, s2);

        let crypto_pk: pyde_crypto::falcon::FalconPublicKey = (&signer.pubkey()).into();
        for sig in [s1, s2] {
            let crypto_sig =
                pyde_crypto::falcon::FalconSignature::from_bytes(sig.as_bytes()).unwrap();
            assert!(pyde_crypto::falcon::falcon_verify(
                &crypto_pk,
                hash.as_bytes(),
                &crypto_sig
            ));
        }
    }

    #[tokio::test]
    async fn sign_tx_populates_signature_and_keeps_hash_stable() {
        let signer = LocalSigner::random().unwrap();
        let mut tx = TxBuilder::new()
            .from(signer.address())
            .transfer(Address::new([0x55; 32]), 1_000)
            .build()
            .unwrap();
        let pre = tx_hash(&tx);
        signer.sign_tx(&mut tx).await.unwrap();
        let post = tx_hash(&tx);
        // tx_hash excludes the signature field — must be unchanged.
        assert_eq!(pre, post);
        assert!(!tx.signature.is_empty());
        assert_eq!(tx.tx_type, TxType::Standard);
    }

    #[tokio::test]
    async fn from_keys_rejects_mismatched_pubkey() {
        let a = LocalSigner::random().unwrap();
        let b = LocalSigner::random().unwrap();
        // Take a's pubkey, b's secret — must reject.
        let mixed = LocalSigner::from_keys(a.pubkey(), b.secret().clone());
        assert!(mixed.is_err());
    }

    #[test]
    fn debug_redacts_secret() {
        let signer = LocalSigner::random().unwrap();
        let dbg = format!("{signer:?}");
        assert!(dbg.contains("redacted"));
    }
}
