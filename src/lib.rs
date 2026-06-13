//! # pyde-rust-sdk
//!
//! Rust SDK for building dapps + wallets on the [Pyde](https://pyde.network)
//! blockchain. Comprehensive surface — account generation, FALCON-512
//! signing, transaction construction, RPC client (HTTP + WebSocket),
//! event subscriptions, typed contract interaction via the
//! [`pyde_abi!`] macro, and the small utility helpers every
//! wallet/dapp needs.
//!
//! Sister SDK in TypeScript: [`pyde-ts-sdk`]. Both mirror the same
//! conceptual surface; both share the canonical Pyde wire format
//! through different cryptographic paths (this crate uses
//! [`pyde_crypto`] directly; the TS SDK uses `pyde-crypto-wasm`).
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use pyde_rust_sdk::{Wallet, util, tx::TxBuilder, types::Address};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let wallet = Wallet::generate()?;
//!     let recipient = Address::from_hex("0xaa…")?;
//!
//!     let mut tx = TxBuilder::new()
//!         .from(wallet.address())
//!         .transfer(recipient, util::parse_quanta("1.5")?)
//!         .build()?;
//!     wallet.sign_tx(&mut tx).await?;
//!     // Provider lands in T9 — submit via `provider.send_raw_transaction(tx)`.
//!     Ok(())
//! }
//! ```
//!
//! See [`PROPOSAL.md`](https://github.com/pyde-net/pyde-rust-sdk/blob/main/PROPOSAL.md)
//! for the full v1 surface plan + phased build schedule.

// ── Module declarations ──────────────────────────────────────────────

pub mod abi;
pub mod contract;
pub mod error;
pub mod provider;
pub mod signer;
pub mod tx;
pub mod types;
pub mod util;
pub mod wallet;
pub mod ws;

// ── Top-level re-exports — what 95% of users reach for. ──────────────

pub use error::{Result, SdkError};
pub use signer::{LocalSigner, Signer};
pub use tx::TxBuilder;
pub use types::{
    AccessEntry, AccessType, AccountType, Address, AuthKeys, BlockHeader, CallOverrides,
    FalconPubkey, FalconSignature, FeeData, FeePayer, Log, LogFilter, Receipt, Tx, TxHash, TxType,
};
pub use wallet::{Keystore, Wallet};
