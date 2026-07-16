//! Conventional gas + timing constants for the v1 SDK surface.
//!
//! These are educated defaults, **not** chain constants — they
//! reflect what typical dapps need on a healthy devnet/mainnet
//! after observing the live E2E example flows. Tune per workload;
//! they're exposed here so callers don't bake magic numbers into
//! their code.

/// Gas budget for a vanilla PYDE transfer ([`crate::types::TxType::Standard`],
/// empty calldata). Slightly above the engine's [`crate::types::MIN_GAS_LIMIT`]
/// (21,000) to absorb hashing + signature-verify costs.
pub const GAS_TRANSFER: u64 = 100_000;

/// Default gas budget for a fungible-token (pts-f/1) state-mutating
/// call (transfer, approve, transfer_from). Suits the canonical
/// otigen `fungible-token` template.
pub const GAS_TOKEN_CALL: u64 = 500_000;

/// Default gas budget for an NFT (pts-n/1) state-mutating call
/// (mint, transfer_from, approve, set_approval_for_all). Higher than
/// the fungible budget because per-id ownership mutates more slots.
pub const GAS_NFT_CALL: u64 = 1_000_000;

/// Default gas budget for a contract deployment. Engine caps at
/// the block gas limit; this is the typical comfort margin for a
/// 15-30 KiB WASM bundle.
pub const GAS_DEPLOY: u64 = 10_000_000;

/// Default gas budget for a marketplace-style cross-contract
/// orchestrator call (`buy(listing_id)` style). Higher than a
/// single ERC operation because the entry chains 2-3 cross_calls.
pub const GAS_CROSS_CALL_ORCHESTRATOR: u64 = 2_000_000;
