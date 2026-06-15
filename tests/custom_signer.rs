//! `Signer` trait conformance tests for non-`LocalSigner`/`Wallet`
//! implementations.
//!
//! These tests pin the trait contract for the path that motivates the
//! abstraction in the first place: HSM-backed signers, remote signing
//! services, hardware wallets. The SDK itself only ships
//! `LocalSigner` + `Wallet` (which both hold the FALCON secret in
//! process memory) — but the whole point of the trait is that
//! out-of-process secret-key custody works just as well, with the
//! same call sites.
//!
//! A future refactor that accidentally adds `'static + Sized` bounds
//! to the trait, or that bakes in a `LocalSigner`-specific assumption
//! in the default `sign_tx` impl, would break every HSM integration
//! silently. These tests catch that drift before it ships.
//!
//! The `MockRemoteSigner` here uses a `LocalSigner` under the hood
//! purely to keep the test self-contained (no HSM in CI). A real
//! `HsmSigner` would replace `inner.sign_hash(hash)` with a network
//! / USB / PKCS#11 round-trip instead.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use pyde_rust_sdk::error::SdkError;
use pyde_rust_sdk::signer::{LocalSigner, Signer};
use pyde_rust_sdk::tx::tx_hash;
use pyde_rust_sdk::types::{Address, FalconPubkey, FalconSignature, TxHash};
use pyde_rust_sdk::TxBuilder;

/// A non-`LocalSigner`/`Wallet` `Signer` implementation. Stands in for
/// a real HSM/remote-signer backend. Carries a call counter so tests
/// can assert the trait surface is actually exercised (not the inner
/// `LocalSigner` being detected and bypassed).
struct MockRemoteSigner {
    address: Address,
    pubkey: FalconPubkey,
    /// Count of `sign_hash` invocations. The trait's default
    /// `sign_tx` impl calls `sign_hash` exactly once per tx; an HSM
    /// override would too.
    sign_calls: AtomicUsize,
    /// Backend. Real HSM impl would substitute a network/USB client.
    inner: LocalSigner,
}

impl MockRemoteSigner {
    fn new() -> Self {
        let inner = LocalSigner::random().unwrap();
        let address = inner.address();
        let pubkey = inner.pubkey();
        Self {
            address,
            pubkey,
            sign_calls: AtomicUsize::new(0),
            inner,
        }
    }

    fn sign_calls(&self) -> usize {
        self.sign_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Signer for MockRemoteSigner {
    fn address(&self) -> Address {
        self.address
    }

    fn pubkey(&self) -> FalconPubkey {
        self.pubkey
    }

    async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature, SdkError> {
        // In a real HSM: round-trip to the device + wait for sig.
        // Here: delegate to the inner LocalSigner.
        self.sign_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.sign_hash(hash).await
    }

    // NOTE: deliberately NOT overriding `sign_tx`. The default impl
    // (compute `tx_hash`, call `sign_hash`, patch the result back into
    // `tx.signature`) is what 95% of `Signer` impls rely on. Tests
    // below pin that this default impl works for non-`LocalSigner`
    // implementors.
}

// ── address + pubkey accessors ──────────────────────────────────────

#[test]
fn custom_signer_address_and_pubkey_are_stable() {
    let signer = MockRemoteSigner::new();
    let addr_a = signer.address();
    let addr_b = signer.address();
    assert_eq!(addr_a, addr_b);
    assert_eq!(signer.pubkey().as_bytes(), signer.pubkey().as_bytes());
    assert_eq!(signer.pubkey().as_bytes().len(), 897);
}

// ── sign_hash produces a verifiable FALCON-512 signature ────────────

#[tokio::test]
async fn custom_signer_sign_hash_signature_verifies() {
    use pyde_crypto::falcon::{
        falcon_verify, FalconPublicKey as CryptoPk, FalconSignature as CryptoSig,
    };

    let signer = MockRemoteSigner::new();
    let hash = TxHash::new([0xAB; 32]);
    let sig = signer.sign_hash(&hash).await.unwrap();

    let pk = CryptoPk::from_bytes(signer.pubkey().as_bytes()).expect("valid pubkey");
    let crypto_sig = CryptoSig::from_bytes(sig.as_bytes()).expect("valid sig");

    assert!(
        falcon_verify(&pk, hash.as_bytes(), &crypto_sig),
        "custom signer must produce signatures that FALCON-verify",
    );
    assert_eq!(signer.sign_calls(), 1);
}

// ── default sign_tx impl wires through correctly ────────────────────

#[tokio::test]
async fn custom_signer_default_sign_tx_calls_sign_hash_once() {
    let signer = MockRemoteSigner::new();
    let mut tx = TxBuilder::new()
        .from(signer.address())
        .chain_id(31337)
        .nonce(0)
        .gas_limit(100_000)
        .transfer(Address::ZERO, 1)
        .build()
        .unwrap();

    assert_eq!(signer.sign_calls(), 0);
    signer.sign_tx(&mut tx).await.unwrap();
    assert_eq!(
        signer.sign_calls(),
        1,
        "default sign_tx must call sign_hash exactly once per tx",
    );
    assert!(
        !tx.signature.as_bytes().is_empty(),
        "default sign_tx must patch the signature into tx.signature",
    );
}

#[tokio::test]
async fn custom_signer_sign_tx_excludes_signature_from_canonical_hash() {
    let signer = MockRemoteSigner::new();
    let mut tx = TxBuilder::new()
        .from(signer.address())
        .chain_id(31337)
        .nonce(0)
        .gas_limit(100_000)
        .transfer(Address::ZERO, 1)
        .build()
        .unwrap();

    let pre_sign_hash = tx_hash(&tx);
    signer.sign_tx(&mut tx).await.unwrap();
    let post_sign_hash = tx_hash(&tx);

    assert_eq!(
        pre_sign_hash, post_sign_hash,
        "tx_hash must be identical before + after signing — signature \
         is excluded from the canonical pre-image",
    );
}

// ── object safety: trait holds as Box<dyn> + Arc<dyn> ────────────────

#[tokio::test]
async fn custom_signer_works_as_boxed_trait_object() {
    let signer: Box<dyn Signer> = Box::new(MockRemoteSigner::new());
    let mut tx = TxBuilder::new()
        .from(signer.address())
        .chain_id(31337)
        .nonce(0)
        .gas_limit(100_000)
        .transfer(Address::ZERO, 1)
        .build()
        .unwrap();
    signer.sign_tx(&mut tx).await.unwrap();
    assert!(!tx.signature.as_bytes().is_empty());
}

#[tokio::test]
async fn custom_signer_works_as_arc_trait_object_across_tasks() {
    let signer: Arc<dyn Signer> = Arc::new(MockRemoteSigner::new());

    // Sign two tx concurrently in spawned tasks — pins that the
    // trait is `Send + Sync` and Arc-shareable as advertised.
    let sa = signer.clone();
    let sb = signer.clone();

    let task_a = tokio::spawn(async move {
        let mut tx = TxBuilder::new()
            .from(sa.address())
            .chain_id(31337)
            .nonce(0)
            .gas_limit(100_000)
            .transfer(Address::ZERO, 1)
            .build()
            .unwrap();
        sa.sign_tx(&mut tx).await.unwrap();
        tx
    });
    let task_b = tokio::spawn(async move {
        let mut tx = TxBuilder::new()
            .from(sb.address())
            .chain_id(31337)
            .nonce(1)
            .gas_limit(100_000)
            .transfer(Address::ZERO, 2)
            .build()
            .unwrap();
        sb.sign_tx(&mut tx).await.unwrap();
        tx
    });

    let (tx_a, tx_b) = (task_a.await.unwrap(), task_b.await.unwrap());
    assert!(!tx_a.signature.as_bytes().is_empty());
    assert!(!tx_b.signature.as_bytes().is_empty());
    // Different signatures (FALCON is randomised) even from the same
    // signer key — proves both calls actually went through.
    assert_ne!(
        tx_a.signature.as_bytes(),
        tx_b.signature.as_bytes(),
        "two FALCON sigs over different txs must differ",
    );
}

// ── multiple signs from the same signer all verify ──────────────────

#[tokio::test]
async fn custom_signer_handles_repeated_signing() {
    let signer = MockRemoteSigner::new();
    for i in 0..5_u64 {
        let mut tx = TxBuilder::new()
            .from(signer.address())
            .chain_id(31337)
            .nonce(i)
            .gas_limit(100_000)
            .transfer(Address::ZERO, 1)
            .build()
            .unwrap();
        signer.sign_tx(&mut tx).await.unwrap();
        assert!(!tx.signature.as_bytes().is_empty());
    }
    assert_eq!(
        signer.sign_calls(),
        5,
        "five tx → five sign_hash invocations through the trait",
    );
}
