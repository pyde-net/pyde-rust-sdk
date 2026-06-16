//! # pyde-rust-sdk
//!
//! Rust SDK for building dapps + wallets on the [Pyde](https://pyde.network)
//! blockchain. Comprehensive surface — account generation, FALCON-512
//! signing, transaction construction, RPC client (HTTP + WebSocket),
//! event subscriptions, typed contract interaction via the
//! [`pyde_abi!`] macro, and the small utility helpers every
//! wallet/dapp needs.
//!
//! Sister SDK in TypeScript: `pyde-ts-sdk`. Both target the same
//! chain wire format (Tx, TxType, AuthKeys, FALCON/Poseidon2 byte
//! shapes are pinned across both) but have SDK-specific extras —
//! notably the encrypted-keystore envelopes are not interchangeable
//! (this crate uses AES-256-GCM + nested envelope; TS uses
//! ChaCha20-Poly1305 + flat envelope). See `docs/12-compatibility.md`.
//!
//! ## Quick start
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
//! use pyde_rust_sdk::{Provider, Signer, TxBuilder, Wallet};
//! use pyde_rust_sdk::types::Address;
//! use pyde_rust_sdk::util::parse_quanta;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let transport = HttpTransport::new("http://127.0.0.1:9933")?;
//!     let provider = Arc::new(RootProvider::new(transport));
//!     let wallet = Wallet::generate()?;
//!     let recipient = Address::from_hex("0xaa…")?;
//!
//!     let chain_id = provider.chain_id().await?;
//!     let nonce = provider.get_nonce(&wallet.address()).await?;
//!     let mut tx = TxBuilder::new()
//!         .from(wallet.address())
//!         .chain_id(chain_id)
//!         .nonce(nonce)
//!         .transfer(recipient, parse_quanta("1.5")?)
//!         .build()?;
//!     wallet.sign_tx(&mut tx).await?;
//!     let pending = provider.send_transaction(&tx).await?;
//!     let receipt = pending.wait_for_receipt().await?;
//!     println!("committed in wave {}", receipt.wave_id_u64());
//!     Ok(())
//! }
//! ```
//!
//! See [`PROPOSAL.md`](https://github.com/pyde-net/pyde-rust-sdk/blob/main/PROPOSAL.md)
//! for the full v1 surface plan + phased build schedule.

// ── Module declarations ──────────────────────────────────────────────

pub mod abi;
pub mod constants;
pub mod contract;
pub mod error;
pub mod multisig;
pub mod provider;
pub mod signer;
pub mod tx;
pub mod types;
pub mod util;
pub mod wallet;
pub mod ws;

// ── Top-level re-exports — what 95% of users reach for. ──────────────

pub use contract::{pyde_abi, Contract, DecodedEvent};
pub use error::{Result, SdkError};
pub use provider::{HttpProvider, HttpTransport, PendingTx, Provider, RootProvider, Transport};
pub use signer::{LocalSigner, Signer};
pub use tx::TxBuilder;
pub use types::{
    AccessEntry, AccessType, AccountInfo, AccountType, Address, AuthKeys, BlockHeader,
    CallOverrides, CallPayload, CallRequest, Event, EventFilter, FalconPubkey, FalconSignature,
    FeeData, FeePayer, Log, LogCursor, LogFilter, LogPage, NodeInfo, Receipt, ReceiptStatus,
    SimulationResult, ThresholdPublicKey, Tx, TxHash, TxType, WaveHeader,
};
pub use wallet::{Keystore, Wallet};
pub use ws::{Subscription, WsProvider, WsTransport};
