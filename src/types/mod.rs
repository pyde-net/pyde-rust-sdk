//! Public types — `Address`, `TxHash`, receipts, logs, block headers.
//!
//! T7 scope: declare the type aliases + ship the small RPC response
//! shapes that don't depend on tx canonical encoding. The richer
//! definitions ([`Tx`], [`FeePayer`], [`AccessEntry`], [`AuthKeys`],
//! [`AccountType`], [`TxType`]) land in T8 alongside the canonical
//! encoder, since they need to be byte-identical to the engine's
//! `crates/tx/src/types.rs` wire format.

use serde::{Deserialize, Serialize};

/// 32-byte Pyde address. Derived via Poseidon2 (see Ch 11 §11.2):
///
/// - **EOA**: `Poseidon2(falcon_pubkey_bytes)`.
/// - **CREATE**: `Poseidon2(deployer_address ‖ nonce_bytes)`.
/// - **CREATE2**: `Poseidon2(0xFF ‖ deployer_address ‖ salt ‖ code_hash)`.
///
/// Stored, transmitted, and compared as a fixed 32-byte array. The
/// canonical text form is a lower-case `0x`-prefixed 64-character hex
/// string — see [`crate::util::format_address`] / [`crate::util::parse_address`].
pub type Address = [u8; 32];

/// 32-byte transaction hash. Output of Poseidon2 over the canonical
/// transaction pre-image (see Ch 11 §11.6 — chain_id, from, to, value,
/// pre-hashed data, gas_limit, nonce, fee_payer_tag, pre-hashed access
/// list, deadline, tx_type). The signature is NOT in the pre-image —
/// two different signatures over the same fields produce the same
/// `TxHash`, so the mempool catches duplicates regardless of which
/// signature attempt arrived.
pub type TxHash = [u8; 32];

/// Receipt for a committed transaction, as returned by
/// `pyde_getTransactionReceipt` / `pyde_getReceipt`.
///
/// All numeric fields arrive as hex strings on the wire (standard
/// JSON-RPC convention); use the inherent conversion methods to access
/// typed values.
///
/// T8 will revisit the field shape to match the exact response schema
/// of the current node (the pre-pivot shape may have drifted).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    /// `0x`-prefixed tx hash this receipt belongs to.
    pub tx_hash: String,
    /// `true` iff the tx executed without reverting or running out of gas.
    pub success: bool,
    /// Gas actually charged, hex-encoded. Use [`Receipt::gas`] to get u64.
    pub gas_used: String,
    /// Effective gas price applied (base_fee at commit time), hex.
    pub effective_gas: String,
    /// Total fee paid in quanta, hex.
    pub fee_paid: String,
    /// Portion of the fee burned per Pyde's EIP-1559 model, hex.
    pub fee_burned: String,
    /// Portion of the fee paid to the validator pool, hex.
    pub fee_validator: String,
    /// Return data from `pyde::return(...)`, hex. Empty for non-success
    /// outcomes that didn't explicitly return.
    #[serde(default)]
    pub return_data: String,
    /// Event logs emitted during execution (empty for reverts).
    #[serde(default)]
    pub logs: Vec<Log>,
}

impl Receipt {
    /// Decode `gas_used` from its hex string into a u64.
    ///
    /// Returns 0 on malformed input — the receipt object is built by
    /// the SDK from a node response, so a bad hex here would already
    /// have surfaced as [`crate::SdkError::InvalidResponse`].
    pub fn gas(&self) -> u64 {
        u64::from_str_radix(self.gas_used.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `return_data` as raw bytes.
    pub fn return_bytes(&self) -> Vec<u8> {
        let hex = self.return_data.trim_start_matches("0x");
        hex::decode(hex).unwrap_or_default()
    }

    /// For Deploy receipts, extract the contract's derived CREATE
    /// address from `return_data`. Returns `None` if the return data
    /// isn't exactly 32 bytes (i.e. not a Deploy receipt).
    pub fn contract_address(&self) -> Option<Address> {
        let bytes = self.return_bytes();
        if bytes.len() == 32 {
            let mut addr = [0u8; 32];
            addr.copy_from_slice(&bytes);
            Some(addr)
        } else {
            None
        }
    }
}

/// One event log entry on a [`Receipt`].
///
/// `topics[0]` is the event signature hash by convention; `topics[1..]`
/// are the indexed parameters (max 4 topics per event per
/// [`HOST_FN_ABI_SPEC §15.3`]). `data` carries the non-indexed
/// parameter payload, hex-encoded.
#[derive(Debug, Clone, Deserialize)]
pub struct Log {
    /// Address of the contract that emitted the event, hex.
    pub address: String,
    /// Topic hashes — `topics[0]` is the event signature.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Non-indexed event payload bytes, hex.
    #[serde(default)]
    pub data: String,
}

/// Filter for querying historical event logs via `pyde_getLogs`.
///
/// All fields are optional; omitted fields mean "no constraint on this
/// dimension." `topics` mirrors the EVM semantics: each slot is either
/// `None` (match any topic at that position) or `Some(vec_of_topics)`
/// (OR-match against any topic in the list).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFilter {
    /// Lower-bound wave id, inclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_block: Option<u64>,
    /// Upper-bound wave id, inclusive.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_block: Option<u64>,
    /// Filter to logs emitted by this contract address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Topic filters — see struct docs for semantics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topics: Option<Vec<Option<Vec<String>>>>,
}

/// Header info for a single wave (Pyde's equivalent of a "block"), as
/// returned by `pyde_getWave(wave_id)`.
///
/// Field shape is preliminary — T9 will reconcile with the current
/// node's wire format.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockHeader {
    /// Wave id (block number in Ethereum vocabulary), hex.
    pub slot: String,
    /// Wall-clock timestamp at commit, hex (seconds since Unix epoch).
    pub timestamp: String,
    /// Wave proposer address, hex.
    pub proposer: String,
    /// Post-commit JMT state root, hex.
    #[serde(default)]
    pub state_root: String,
    /// Number of transactions committed in this wave, hex.
    #[serde(default)]
    pub tx_count: String,
}

/// Optional overrides for [`crate::Provider::call`] /
/// [`crate::Provider::simulate_transaction`].
///
/// All fields default to "use the wallet/network defaults." Set
/// explicitly to simulate from a non-default sender, override the
/// attached value, or cap the gas budget for a simulation.
#[derive(Debug, Clone, Default)]
pub struct CallOverrides {
    /// Address to attribute the call to (defaults to the wallet's
    /// address, or zero if no wallet is bound).
    pub from: Option<Address>,
    /// PYDE value to "attach" to the simulated call (quanta).
    pub value: Option<u128>,
    /// Maximum gas the simulation is allowed to burn.
    pub gas_limit: Option<u64>,
}

/// Current fee snapshot from the network, as returned by future
/// `Provider::get_fee_data` (T9).
///
/// Pyde follows the EIP-1559 base-fee model but does NOT have user-set
/// priority tips in v1 — see Ch 11 §11.6. Both fields therefore carry
/// the same value today; the dual struct field is preserved for forward
/// compatibility when a tip dimension is introduced.
#[derive(Debug, Clone)]
pub struct FeeData {
    /// Effective gas price — base fee per gas unit. No tips in v1.
    pub gas_price: u128,
    /// Base fee per gas unit at the latest committed wave.
    pub base_fee: u128,
}
