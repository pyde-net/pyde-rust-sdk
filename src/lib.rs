//! # pyde-rust-sdk
//!
//! Rust SDK for building dapps + wallets on the [Pyde](https://pyde.network)
//! blockchain. Comprehensive surface — account generation, FALCON-512
//! signing, transaction construction, RPC client (HTTP + WebSocket),
//! event subscriptions, typed contract interaction via the
//! [`pyde_abi!`] macro, and the small utility helpers every wallet/dapp
//! needs.
//!
//! Sister SDK in TypeScript: [`pyde-ts-sdk`]. Both mirror the same
//! conceptual surface; both share the canonical Pyde wire format
//! through different cryptographic paths (this crate uses
//! [`pyde-crypto`] directly; the TS SDK uses [`pyde-crypto-wasm`]).
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use pyde_rust_sdk::{Provider, Wallet, util};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let provider = Provider::http("http://127.0.0.1:8545")?;
//!     let wallet = Wallet::generate()?;
//!     let recipient = util::parse_address("0xaa...")?;
//!
//!     let pending = provider
//!         .transfer(&wallet, recipient, util::parse_quanta("1.5")?)
//!         .send()
//!         .await?;
//!     let receipt = pending.await_finalized().await?;
//!     println!("{:?}", receipt);
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

// ── Top-level re-exports — the things 95% of users touch ─────────────

pub use error::{Result, SdkError};
pub use types::{Address, BlockHeader, CallOverrides, FeeData, Log, LogFilter, Receipt, TxHash};
