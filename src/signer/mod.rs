//! `Signer` trait — the abstraction over anything that can sign a tx
//! or a 32-byte hash on behalf of an address.
//!
//! ## T7 stub
//!
//! Real implementation lands in T8 (Phase A1). The shape:
//!
//! ```ignore
//! #[async_trait::async_trait]
//! pub trait Signer: Send + Sync {
//!     fn address(&self) -> Address;
//!     async fn sign_hash(&self, hash: &TxHash) -> Result<FalconSignature>;
//!     async fn sign_tx(&self, tx: &mut Tx) -> Result<()>;
//! }
//! ```
//!
//! Async per alloy convention — keeps the trait object-safe (via
//! `async_trait`) and accommodates future hardware-wallet impls
//! that need to round-trip a USB / network call.
//!
//! Built-in impls (T8): [`LocalSigner`] (in-memory FALCON keypair).
//! Future (Phase B2 follow-up): trait scaffolding for `LedgerSigner`,
//! `AwsKmsSigner`, etc. — per-backend crates per alloy convention.
