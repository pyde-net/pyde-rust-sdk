//! JSON-RPC client (`Provider`) + HTTP transport + middleware fillers.
//!
//! ## T7 stub
//!
//! Real implementation lands in T9 (Phase A2). This module will own:
//!
//! - [`Provider`] trait — the SDK-wide RPC abstraction.
//! - [`HttpProvider`] — concrete HTTP transport built on `reqwest`.
//! - All 23 RPC method wrappers from the T2 survey:
//!     * **chain info**: `chain_id`, `wave_id`, `get_node_info`, `get_metrics`
//!     * **state**: `get_balance`, `get_nonce`, `get_account`,
//!       `get_contract_code`, `get_storage_slot`, `resolve_name`
//!     * **tx submission**: `send_raw_transaction`, `call`,
//!       `simulate_transaction`
//!     * **receipts**: `get_receipt`, `get_transaction_receipt`,
//!       `get_tx`, `get_wave`
//!     * **events**: `get_events`, `get_logs`
//!     * **validators**: `get_validator`, `get_operator_validators`
//!     * **state sync**: `get_snapshot`, `get_snapshot_manifest`
//! - [`PendingTx`] — alloy-style handle returned by `send_*` methods,
//!   with `.await_committed()` and `.await_finalized()` stages.
//! - **Fillers** (tower-style middleware) — composable layers on top
//!   of a `RootProvider`: `NonceFiller` auto-supplies the next nonce
//!   from the 16-slot window, `GasFiller` simulates to estimate, etc.
//!   Pattern stolen from alloy's `ProviderBuilder`.
//!
//! Pre-pivot had a working 24-method client; ~16 of those map cleanly,
//! 4 must be dropped (no longer exist node-side), 4 renamed
//! (block→wave terminology), 8 new to add. See PROPOSAL.md §5.
