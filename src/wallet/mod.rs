//! Wallet + encrypted keystore.
//!
//! ## T7 stub
//!
//! Real implementation lands in T8 (Phase A1). This module will own:
//!
//! - [`Wallet`] — concrete [`crate::signer::Signer`] impl wrapping a
//!   FALCON-512 keypair plus convenience methods (`Wallet::generate`,
//!   `Wallet::from_secret`, `wallet.address()`, `wallet.transfer(...)`,
//!   etc.). `Wallet::generate` uses OS entropy via `getrandom`;
//!   `Wallet::generate_from_seed([u8; 32])` is the opt-in deterministic
//!   variant for tests.
//! - [`Keystore`] — encrypted at-rest secret-key storage. JSON envelope
//!   compatible with `pyde-ts-sdk` so wallets can move across SDKs.
//!   KDF: argon2id with sane defaults (t=3, m=64MB, p=1). Cipher:
//!   AES-256-GCM with random nonce. Salt and nonce stored in the
//!   envelope. Zeroized in memory after decrypt.
//!
//! Pre-pivot version had a working Keystore — the cryptographic shape
//! is correct, only the Tx-signing surface needs to be rebuilt against
//! the new [`crate::signer::Signer`] async trait.
