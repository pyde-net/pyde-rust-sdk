//! RPC response shapes — receipts, logs, filters, wave headers.
//!
//! These types ride the JSON-RPC wire, not Borsh. Numeric fields
//! arrive as `0x`-prefixed hex strings per Ethereum-style JSON-RPC
//! convention; the inherent accessor methods decode them lazily.

use serde::{Deserialize, Serialize};

use super::Address;

/// Receipt for a committed transaction.
///
/// Returned by `pyde_getReceipt` and `pyde_getTransactionReceipt`.
/// All numeric fields are hex strings on the wire — use the
/// accessor methods to get typed values.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    /// `0x`-prefixed tx hash this receipt belongs to.
    pub tx_hash: String,
    /// `true` iff the tx executed without reverting or running out
    /// of gas.
    pub success: bool,
    /// Gas actually charged, hex.
    pub gas_used: String,
    /// Effective gas price (base_fee at commit time), hex.
    pub effective_gas: String,
    /// Total fee paid in quanta, hex.
    pub fee_paid: String,
    /// Portion of the fee burned per Pyde's EIP-1559 model, hex.
    pub fee_burned: String,
    /// Portion of the fee paid to the validator pool, hex.
    pub fee_validator: String,
    /// Return data from `pyde::return(...)`, hex. Empty for
    /// non-success outcomes that didn't explicitly return.
    #[serde(default)]
    pub return_data: String,
    /// Event logs emitted during execution (empty for reverts).
    #[serde(default)]
    pub logs: Vec<Log>,
}

impl Receipt {
    /// Decode `gas_used` to `u64`. Returns 0 on malformed input —
    /// the receipt itself was already validated as deserializable
    /// from a node response, so malformed hex here would have
    /// surfaced as [`crate::SdkError::InvalidResponse`].
    #[must_use]
    pub fn gas(&self) -> u64 {
        u64::from_str_radix(self.gas_used.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `effective_gas` to `u128`.
    #[must_use]
    pub fn effective_gas_price(&self) -> u128 {
        u128::from_str_radix(self.effective_gas.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `fee_paid` to `u128` quanta.
    #[must_use]
    pub fn fee_paid_quanta(&self) -> u128 {
        u128::from_str_radix(self.fee_paid.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `return_data` as raw bytes.
    #[must_use]
    pub fn return_bytes(&self) -> Vec<u8> {
        let hex_str = self.return_data.trim_start_matches("0x");
        hex::decode(hex_str).unwrap_or_default()
    }

    /// For `Deploy` receipts, the derived contract address.
    ///
    /// Returns `None` if the return data isn't exactly 32 bytes —
    /// i.e. this wasn't a Deploy receipt.
    #[must_use]
    pub fn contract_address(&self) -> Option<Address> {
        let bytes = self.return_bytes();
        if bytes.len() == 32 {
            let mut addr = [0u8; 32];
            addr.copy_from_slice(&bytes);
            Some(Address(addr))
        } else {
            None
        }
    }
}

/// One event log entry on a [`Receipt`].
///
/// `topics[0]` is the event signature hash by convention; later
/// topics are indexed parameters (max 4 topics per event per
/// HOST_FN_ABI §15.3). `data` carries the non-indexed payload,
/// hex-encoded.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// Omitted fields mean "no constraint on this dimension." `topics`
/// mirrors EVM semantics: each slot is either `None` (match any
/// topic at that position) or `Some(vec)` (OR-match against any
/// topic in the list).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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

/// Header info for a single wave (Pyde's equivalent of a "block"),
/// returned by `pyde_getWave`.
///
/// Field shape preliminary — T9 will reconcile against the running
/// node's wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockHeader {
    /// Wave id (block number in Ethereum vocabulary), hex.
    pub slot: String,
    /// Wall-clock commit timestamp, hex (seconds since Unix epoch).
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

/// Optional overrides for [`crate::Provider::call`] and
/// [`crate::Provider::simulate_transaction`].
///
/// All fields default to "use wallet/network defaults." Set
/// explicitly to simulate from a non-default sender, override the
/// attached value, or cap the simulation gas budget.
#[derive(Debug, Clone, Default)]
pub struct CallOverrides {
    /// Address to attribute the call to (defaults to the wallet's
    /// address, or zero if no wallet is bound).
    pub from: Option<Address>,
    /// PYDE value to "attach" to the simulated call, in quanta.
    pub value: Option<u128>,
    /// Maximum gas the simulation is allowed to burn.
    pub gas_limit: Option<u64>,
}

/// Current fee snapshot from the network — returned by future
/// `Provider::get_fee_data` (T9).
///
/// Pyde follows the EIP-1559 base-fee model but does NOT have
/// user-set priority tips in v1 (Ch 11 §11.6). Both fields carry
/// the same value today; the dual field is reserved for forward
/// compatibility when a tip dimension is introduced.
#[derive(Debug, Clone, Copy)]
pub struct FeeData {
    /// Effective gas price — base fee per gas unit. No tips in v1.
    pub gas_price: u128,
    /// Base fee per gas unit at the latest committed wave.
    pub base_fee: u128,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn receipt_decodes_hex_fields() {
        let raw = r#"{
            "txHash": "0xdead",
            "success": true,
            "gasUsed": "0x5208",
            "effectiveGas": "0x3b9aca00",
            "feePaid": "0x12345",
            "feeBurned": "0x0",
            "feeValidator": "0x0"
        }"#;
        let r: Receipt = serde_json::from_str(raw).unwrap();
        assert_eq!(r.gas(), 0x5208);
        assert_eq!(r.effective_gas_price(), 0x3b9aca00);
        assert_eq!(r.fee_paid_quanta(), 0x12345);
    }

    #[test]
    fn contract_address_extracts_from_32_byte_return() {
        let bytes = [0xAB; 32];
        let r = Receipt {
            tx_hash: String::new(),
            success: true,
            gas_used: String::new(),
            effective_gas: String::new(),
            fee_paid: String::new(),
            fee_burned: String::new(),
            fee_validator: String::new(),
            return_data: format!("0x{}", hex::encode(bytes)),
            logs: vec![],
        };
        let addr = r.contract_address().unwrap();
        assert_eq!(addr.as_bytes(), &bytes);
    }

    #[test]
    fn log_filter_serializes_omits_none() {
        let f = LogFilter {
            from_block: Some(100),
            to_block: None,
            address: None,
            topics: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("fromBlock"));
        assert!(!json.contains("toBlock"));
        assert!(!json.contains("address"));
    }
}
