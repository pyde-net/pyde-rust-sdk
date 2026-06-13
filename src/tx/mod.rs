//! Transaction construction, canonical encoding, and `tx_hash`.
//!
//! ## T7 stub
//!
//! Real implementation lands in T8 (Phase A1). This module will own:
//!
//! - [`Tx`] struct mirroring `engine/crates/tx/src/types.rs::Transaction`
//!   byte-for-byte (Borsh canonical encoding).
//! - [`FeePayer`] enum (`Sender` / `GasTank` / `Paymaster(Address)`).
//! - [`AccessEntry`] struct (`address`, `storage_keys`, `access_type`).
//! - [`TxType`] enum with all 13 spec'd variants (Standard=0, Deploy=1,
//!   …, RegisterPubkey=13; tag 2 vacant per Ch 11 §11.8).
//! - [`AuthKeys`] enum (`None`, `Single(pk)`, `MultiSig{keys, threshold}`,
//!   `Programmable` v2 reserved).
//! - [`TxBuilder`] — fluent builder, the user-facing construction API.
//! - `canonical_encode(&Tx) -> Vec<u8>` — Borsh of all signed fields.
//! - `tx_hash(&Tx) -> TxHash` — Poseidon2 over canonical pre-image
//!   (`chain_id ‖ from ‖ to ‖ value ‖ Poseidon2(data) ‖ gas_limit ‖
//!   nonce ‖ fee_payer_tag ‖ Poseidon2(access_list) ‖ deadline ‖
//!   tx_type`), exactly matching the engine's encoder.
//!
//! Cross-encoder parity tests against the engine's `pyde_engine_tx`
//! canonical pre-image (run on the dev machine, gated behind a feature
//! flag in CI since CI doesn't have the engine sibling repo).
