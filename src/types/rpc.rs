//! JSON-RPC wire shapes — receipts, events, call inputs, simulation,
//! node info, log pagination.
//!
//! These types ride the JSON-RPC wire, not Borsh. Numeric fields
//! arrive as `0x`-prefixed hex strings per Ethereum-style convention;
//! the accessor methods decode them lazily.
//!
//! All shapes are byte-identical to what
//! `engine/crates/node/src/rpc.rs` emits — wave-not-block field
//! names included.

use serde::{Deserialize, Serialize};

use super::Address;

// ── Receipt + Event ────────────────────────────────────────────

/// Outcome class for a committed transaction.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    /// Tx executed without reverting or running out of gas.
    Success,
    /// Tx reverted via `pyde::revert(...)` or an executor error.
    Reverted,
    /// Tx hit its `gas_limit`.
    OutOfGas,
}

/// Which layer rejected a reverted transaction. Mirrors the engine's
/// `pyde_engine_types::tx::RevertCategory` byte-for-byte.
///
/// Branch on the category — never on the human-readable message —
/// since the engine reserves the right to refine wording across
/// releases. Unknown categories deserialise as
/// [`RevertCategory::Other`] for forward compatibility; check
/// [`Self::is_known`] if you need to detect the unknown case.
///
/// Because [`Self::Other`] is the forward-compat catch-all, callers
/// exhaustively matching on this enum must include a wildcard arm;
/// use [`Self::is_known`] to detect the unknown-variant case.
// The wire shape is lowercase snake_case (`"engine_validation"`,
// `"contract"`, `"vm"`) per the engine's `RevertCategory::category()`
// discriminant; deserialise into PascalCase variants by renaming.
// Without the rename every wire string falls through to `Other(...)`
// and `is_known()`/`is_contract_revert()` etc. all return `false`.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevertCategory {
    /// Engine-side pre-execution checks: nonce window, signature,
    /// fee payment, balance for `fee + value`, access-list
    /// violation, dispatch decode. The tx never reached contract /
    /// transfer commit logic; state is unchanged.
    EngineValidation,
    /// Contract-emitted revert: explicit `revert(msg)` from contract
    /// code (or a contract-level abort the VM caught). `message`
    /// is the contract's revert string, empty if the contract
    /// didn't supply one.
    Contract,
    /// VM-level failure: wasmtime trap, memory out-of-bounds, gas
    /// exhausted inside the executor, host-fn rejection. Distinct
    /// from [`Self::Contract`] — the contract didn't *choose* to
    /// revert, the VM had to stop it.
    Vm,
    /// Forward-compat catch-all for a category the SDK doesn't
    /// know yet (engine shipped a new variant before the SDK was
    /// updated). The original wire string is preserved so logs /
    /// telemetry can still surface it.
    #[serde(untagged)]
    Other(String),
}

impl RevertCategory {
    /// `true` if this is one of the engine's documented categories
    /// (i.e. not [`Self::Other`]).
    #[must_use]
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

/// Structured carrier for the `revert_reason` field on a [`Receipt`].
/// Pairs a category (for UI badging / control flow) with the engine's
/// human-readable message.
///
/// Populated on every `status: "reverted"` receipt the node emits
/// when its engine carries the structured reason. Receipts that
/// omit the field deserialise as `None` via `#[serde(default)]`;
/// callers fall back to [`crate::error::SdkError::revert_reason`] which
/// decodes from `return_data`.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevertReason {
    /// Which layer rejected the tx. Drives explorer badging +
    /// SDK error-variant selection.
    pub category: RevertCategory,
    /// Human-readable reason. Informational — branch on
    /// [`Self::category`], not the string.
    pub message: String,
}

/// Receipt for a committed transaction.
///
/// Returned by `pyde_getReceipt` and `pyde_getTransactionReceipt`.
/// Numeric fields are `0x`-prefixed hex on the wire — accessors
/// (`gas`, `fee_paid_quanta`, `tx_index_u32`, `wave_id_u64`) decode
/// lazily.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// `0x`-prefixed tx hash this receipt belongs to.
    pub tx_hash: String,
    /// Wave id at which the tx committed, hex.
    pub wave_id: String,
    /// Position within the wave, hex.
    pub tx_index: String,
    /// Outcome class.
    pub status: ReceiptStatus,
    /// Gas actually charged, hex.
    pub gas_used: String,
    /// Total fee paid in quanta, hex. Pyde v1 has no priority tips —
    /// `fee_paid = gas_used × base_fee_at_commit`.
    pub fee_paid: String,
    /// Return data from `pyde::return(...)`, hex. Empty for
    /// non-success outcomes that didn't explicitly return.
    #[serde(default)]
    pub return_data: String,
    /// Events emitted during execution (empty for reverts).
    #[serde(default)]
    pub events: Vec<Event>,
    /// Structured revert reason. Populated when `status == Reverted`
    /// and the node emits a structured reason; omitted on success /
    /// out-of-gas receipts and on nodes that don't emit it (defaults
    /// to `None`). When `None`, fall back to
    /// [`crate::SdkError::revert_reason`] which decodes from
    /// `return_data` bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<RevertReason>,
}

/// Strict decode helper for hex `u64` fields.
fn strict_u64_from_hex(s: &str, field: &str) -> Result<u64, crate::SdkError> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| crate::SdkError::InvalidResponse(format!("{field}: {e}")))
}

/// Strict decode helper for hex `u128` fields.
fn strict_u128_from_hex(s: &str, field: &str) -> Result<u128, crate::SdkError> {
    u128::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| crate::SdkError::InvalidResponse(format!("{field}: {e}")))
}

/// Strict decode helper for hex `u32` fields.
fn strict_u32_from_hex(s: &str, field: &str) -> Result<u32, crate::SdkError> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16)
        .map_err(|e| crate::SdkError::InvalidResponse(format!("{field}: {e}")))
}

/// Strict decode helper for hex byte-array fields.
fn strict_bytes_from_hex(s: &str, field: &str) -> Result<Vec<u8>, crate::SdkError> {
    hex::decode(s.trim_start_matches("0x"))
        .map_err(|e| crate::SdkError::InvalidResponse(format!("{field}: {e}")))
}

impl Receipt {
    /// `true` iff `status == Success`.
    #[must_use]
    pub fn is_success(&self) -> bool {
        matches!(self.status, ReceiptStatus::Success)
    }

    /// `true` iff this receipt has `category == EngineValidation`
    /// in its structured revert reason. Returns `false` when the receipt has no structured revert reason (whether because the tx didn't revert or the node didn't supply one).
    #[must_use]
    pub fn is_engine_validation_revert(&self) -> bool {
        matches!(
            &self.revert_reason,
            Some(r) if matches!(r.category, RevertCategory::EngineValidation)
        )
    }

    /// `true` iff this receipt has `category == Contract`. Returns
    /// `false` when the receipt has no structured revert reason.
    #[must_use]
    pub fn is_contract_revert(&self) -> bool {
        matches!(
            &self.revert_reason,
            Some(r) if matches!(r.category, RevertCategory::Contract)
        )
    }

    /// `true` iff this receipt has `category == Vm`. Returns
    /// `false` when the receipt has no structured revert reason.
    #[must_use]
    pub fn is_vm_trap(&self) -> bool {
        matches!(
            &self.revert_reason,
            Some(r) if matches!(r.category, RevertCategory::Vm)
        )
    }

    /// Decode `gas_used` to `u64`. Returns `0` on malformed input;
    /// use [`Self::try_gas`] for strict error handling.
    #[must_use]
    pub fn gas(&self) -> u64 {
        self.try_gas().unwrap_or(0)
    }

    /// Strict variant of [`Self::gas`] — returns
    /// [`crate::SdkError::InvalidResponse`] if the hex is malformed
    /// rather than silently yielding zero.
    ///
    /// # Errors
    /// [`crate::SdkError::InvalidResponse`] on bad hex.
    pub fn try_gas(&self) -> Result<u64, crate::SdkError> {
        strict_u64_from_hex(&self.gas_used, "gas_used")
    }

    /// Decode `fee_paid` to `u128` quanta. Returns `0` on malformed
    /// input; use [`Self::try_fee_paid_quanta`] for strict handling.
    #[must_use]
    pub fn fee_paid_quanta(&self) -> u128 {
        self.try_fee_paid_quanta().unwrap_or(0)
    }

    /// Strict variant of [`Self::fee_paid_quanta`].
    ///
    /// # Errors
    /// [`crate::SdkError::InvalidResponse`] on bad hex.
    pub fn try_fee_paid_quanta(&self) -> Result<u128, crate::SdkError> {
        strict_u128_from_hex(&self.fee_paid, "fee_paid")
    }

    /// Decode `wave_id` to `u64`. Returns `0` on malformed input.
    #[must_use]
    pub fn wave_id_u64(&self) -> u64 {
        self.try_wave_id_u64().unwrap_or(0)
    }

    /// Strict variant of [`Self::wave_id_u64`].
    ///
    /// # Errors
    /// [`crate::SdkError::InvalidResponse`] on bad hex.
    pub fn try_wave_id_u64(&self) -> Result<u64, crate::SdkError> {
        strict_u64_from_hex(&self.wave_id, "wave_id")
    }

    /// Decode `tx_index` to `u32`. Returns `0` on malformed input.
    #[must_use]
    pub fn tx_index_u32(&self) -> u32 {
        self.try_tx_index_u32().unwrap_or(0)
    }

    /// Strict variant of [`Self::tx_index_u32`].
    ///
    /// # Errors
    /// [`crate::SdkError::InvalidResponse`] on bad hex.
    pub fn try_tx_index_u32(&self) -> Result<u32, crate::SdkError> {
        strict_u32_from_hex(&self.tx_index, "tx_index")
    }

    /// Decode `return_data` as raw bytes. Returns empty on malformed
    /// input; use [`Self::try_return_bytes`] for strict handling.
    #[must_use]
    pub fn return_bytes(&self) -> Vec<u8> {
        self.try_return_bytes().unwrap_or_default()
    }

    /// Strict variant of [`Self::return_bytes`].
    ///
    /// # Errors
    /// [`crate::SdkError::InvalidResponse`] on bad hex.
    pub fn try_return_bytes(&self) -> Result<Vec<u8>, crate::SdkError> {
        strict_bytes_from_hex(&self.return_data, "return_data")
    }

    /// For `Deploy` receipts, the derived contract address.
    /// Returns `None` if the return data isn't exactly 32 bytes.
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

// ── ThresholdPublicKey — pyde_getThresholdPublicKey result ────────────

/// Threshold-decryption public key for the encrypted-mempool path.
///
/// Returned by [`crate::Provider::get_threshold_public_key`]. Wallets
/// encrypt a `Tx` under `public_key` before submitting via
/// [`crate::Provider::send_raw_encrypted_transaction`] for MEV
/// protection.
///
/// ## v1 mock-DKG warning
///
/// V1 ships with `MockDkg` — `scheme: "mock"` and a deterministic
/// `public_key`. Encrypted txs submitted under the mock pubkey
/// **will sit unprocessed** until real Kyber-768 + threshold-sig
/// crypto lands. v1 dapps wanting MEV protection should treat
/// `scheme != "kyber-768"` as "encrypted path not yet ready, fall
/// back to plaintext."
///
/// Per epoch — refresh on every encrypted submit (cheap, no
/// consensus round-trip).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdPublicKey {
    /// DKG epoch this pubkey is valid for, hex string.
    pub epoch: String,
    /// `"mock"` (v1 default) or `"kyber-768"` (post-real-crypto).
    pub scheme: String,
    /// The pubkey bytes, hex string. Length depends on `scheme`.
    pub public_key: String,
}

// ── Event ─────────────────────────────────────────────────────────────

/// Pyde-native shape — note `contract_addr` (not `address` per the
/// Ethereum convention) and the wave/tx/event triple positional
/// identity. `topics` are 32-byte hashes per
/// [HOST_FN_ABI §15.3](https://book.pyde.network/companion/HOST_FN_ABI_SPEC#153-event-emission);
/// `data` is the non-indexed payload bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Wave id where the event was emitted, hex.
    pub wave_id: String,
    /// Position of the emitting tx within the wave, hex.
    pub tx_index: String,
    /// Position of this event within the tx, hex.
    pub event_index: String,
    /// Address of the contract that emitted the event, hex.
    pub contract_addr: String,
    /// 32-byte topic hashes. `topics[0]` is the event signature.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Non-indexed event payload bytes, hex.
    #[serde(default)]
    pub data: String,
}

impl Event {
    /// Decode `wave_id` to `u64`.
    #[must_use]
    pub fn wave_id_u64(&self) -> u64 {
        u64::from_str_radix(self.wave_id.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `tx_index` to `u32`.
    #[must_use]
    pub fn tx_index_u32(&self) -> u32 {
        u32::from_str_radix(self.tx_index.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `event_index` to `u32`.
    #[must_use]
    pub fn event_index_u32(&self) -> u32 {
        u32::from_str_radix(self.event_index.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// Decode `data` as raw bytes.
    #[must_use]
    pub fn data_bytes(&self) -> Vec<u8> {
        hex::decode(self.data.trim_start_matches("0x")).unwrap_or_default()
    }
}

// ── Account info ───────────────────────────────────────────────

/// Account record returned by `pyde_getAccount`.
///
/// Fields use snake_case on the wire (matching the engine's JSON
/// output). The `account_type` is `"eoa"`, `"contract"`, or
/// `"system"` per Ch 11 §11.3.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    /// Account address, hex.
    pub address: String,
    /// `"eoa"`, `"contract"`, or `"system"`.
    pub account_type: String,
    /// Balance in quanta, hex.
    pub balance: String,
    /// Next accepted nonce (window base + trailing-ones offset).
    pub nonce: u64,
    /// Poseidon2 code hash, hex. Zeroed for non-contracts.
    pub code_hash: String,
    /// Contract storage root, hex. Zeroed for EOAs.
    pub state_root: String,
}

impl AccountInfo {
    /// Decode `balance` to `u128` quanta.
    #[must_use]
    pub fn balance_quanta(&self) -> u128 {
        u128::from_str_radix(self.balance.trim_start_matches("0x"), 16).unwrap_or(0)
    }

    /// `true` iff this is a contract account.
    #[must_use]
    pub fn is_contract(&self) -> bool {
        self.account_type == "contract"
    }
}

// ── Node info ──────────────────────────────────────────────────

/// Node identity returned by `pyde_getNodeInfo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    /// Libp2p peer id, hex.
    pub peer_id: String,
    /// Validator FALCON-512 pubkey, hex.
    pub falcon_pubkey: String,
    /// Multiaddrs the node listens on.
    #[serde(default)]
    pub listen_addrs: Vec<String>,
    /// Agent version string (e.g. `"pyde-node/0.1.0"`).
    pub agent_version: String,
    /// Wire-protocol version string (e.g. `"pyde/1"`).
    pub protocol_version: String,
}

// ── Call + simulation ──────────────────────────────────────────

/// Request body for `pyde_call`.
///
/// `data` MUST be the Borsh-encoded
/// [`crate::types::CallPayload`] shape — `{function, calldata}`.
/// The SDK's contract layer (T10) populates this automatically when
/// dispatching typed calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRequest {
    /// Target contract address, hex.
    pub to: String,
    /// Borsh-encoded `CallPayload`, hex.
    pub data: String,
    /// Caller address attribution. Optional; defaults to zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// PYDE value attached to the call, quanta hex. Optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Gas budget for the view call, hex. Optional; node default
    /// is 10,000,000 if omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas: Option<String>,
}

/// Result of `pyde_simulateTransaction` — receipt-that-would-be plus
/// the observed access list.
///
/// `receipt: None` means the executor routed to a no-op (system tx
/// type, or `Standard` to an address with no code).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    /// Predicted receipt, or `None` for routed-to-no-op.
    #[serde(default)]
    pub receipt: Option<SimulationReceipt>,
    /// Access pattern observed during simulation. Use to populate
    /// the real submission's `access_list` so the scheduler can
    /// schedule the tx in parallel.
    pub access_list: SimulationAccessList,
}

/// Simulation receipt — a stripped subset of the on-chain
/// [`Receipt`] (no events, no wave/tx position yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReceipt {
    /// `"Success"` | `"Reverted"` | `"OutOfGas"` (note: simulation
    /// status uses Title-cased strings, not the snake_case the
    /// post-commit receipt uses).
    pub status: String,
    /// Gas charged, hex.
    pub gas_used: String,
    /// Fee paid, quanta hex.
    pub fee_paid: String,
    /// Return data, hex.
    pub return_data: String,
}

/// Access pattern observed during simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationAccessList {
    /// Slots read. Each entry may carry a `tx_index`/`attempt`
    /// observation when the read collided with a concurrent write.
    pub reads: Vec<SimulationRead>,
    /// Slots written, as 32-byte hex slot hashes.
    pub writes: Vec<String>,
}

/// One observed read during simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationRead {
    /// 32-byte slot hash, hex.
    pub slot: String,
    /// Version observed for this slot, or `None` if the read was
    /// from the committed base.
    #[serde(default)]
    pub observed_version: Option<SimulationReadVersion>,
}

/// Concurrent-write attribution attached to a [`SimulationRead`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationReadVersion {
    /// Index of the writing tx in the wave.
    pub tx_index: u32,
    /// Attempt number (Block-STM retry counter).
    pub attempt: u32,
}

// ── Log filter + page ──────────────────────────────────────────

/// Filter for `pyde_getLogs` and `subscribe_logs`.
///
/// Multi-contract OR + position-sensitive topic
/// AND-across-positions, OR-within-position (mirrors EVM semantics).
/// Empty / unset means "no constraint."
///
/// The wave-range and cursor fields apply only to `pyde_getLogs`;
/// the streaming subscription ignores them (subscriptions are
/// always tip-following).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogFilter {
    /// Lower-bound wave id, hex. Default `"0x0"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_wave: Option<String>,
    /// Upper-bound wave id, hex. Default = current wave.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_wave: Option<String>,
    /// OR-match: contract addresses, hex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contracts: Vec<String>,
    /// AND-across-positions, OR-within-position. Each position is
    /// either `None` (match any) or `Some(vec)` (match one of the
    /// listed 32-byte topic hashes).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Option<Vec<String>>>,
    /// Resume from a prior page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<LogCursor>,
    /// Max entries per page (server-capped at 5000; default 500).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Cursor identifying a specific event for pagination resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogCursor {
    /// Wave id, hex.
    pub wave_id: String,
    /// Tx index, hex.
    pub tx_index: String,
    /// Event index, hex.
    pub event_index: String,
}

/// Page returned by `pyde_getLogs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPage {
    /// Matching events.
    #[serde(default)]
    pub entries: Vec<Event>,
    /// Resume cursor — `None` when the result fits in one page.
    #[serde(default)]
    pub next_cursor: Option<LogCursor>,
}

// ── pyde_getEvents request shape ───────────────────────────────

/// Simple filter for `pyde_getEvents` (no pagination, no topic
/// matching — for that, use [`LogFilter`] + `pyde_getLogs`).
///
/// All fields optional. Server defaults: full history, all
/// contracts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFilter {
    /// Lower-bound wave id, hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_wave: Option<String>,
    /// Upper-bound wave id, hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_wave: Option<String>,
    /// Single contract address, hex. Use [`LogFilter::contracts`]
    /// for multi-contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
}

// ── Call overrides + fee data ──────────────────────────────────

/// Optional overrides for [`crate::Provider::call`] and
/// [`crate::Provider::simulate_transaction`] when building the
/// [`CallRequest`] from a typed contract call.
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

/// Current fee snapshot from the network — returned by
/// [`crate::Provider::get_fee_data`].
///
/// Pyde follows the EIP-1559 base-fee model but does NOT have
/// user-set priority tips in v1 (Ch 11 §11.6) — `suggested_tip` is
/// always `0`. The dual field is reserved for forward
/// compatibility when a tip dimension is introduced.
///
/// `recent_waves` carries the last 10 committed waves with their
/// gas-utilisation summaries so wallets can render a gas-price
/// slider or network-load chart in one round-trip.
#[derive(Debug, Clone)]
pub struct FeeData {
    /// Base fee per gas unit at the latest committed wave.
    pub base_fee: u128,
    /// Suggested priority tip — always `0` in v1 (no tip dimension).
    pub suggested_tip: u128,
    /// Wave id at which the snapshot was taken.
    pub wave_id: u64,
    /// Last 10 waves (most recent first) — `(wave_id, gas_used, gas_limit, utilisation)`.
    /// `utilisation = gas_used / GAS_TARGET`; `> 1.0` pushes base fee up next wave.
    pub recent_waves: Vec<RecentWaveSummary>,
}

/// One entry in [`FeeData::recent_waves`] — gas-utilisation
/// summary for a single committed wave.
#[derive(Debug, Clone, Copy)]
pub struct RecentWaveSummary {
    /// Wave id this summary covers.
    pub wave_id: u64,
    /// Total gas burned in the wave.
    pub gas_used: u64,
    /// Elasticity gas limit (`2 × GAS_TARGET`).
    pub gas_limit: u64,
    /// `gas_used / GAS_TARGET`; values near `1.0` mean the wave
    /// hit target, `> 1.0` overshot (pushes base fee up next wave).
    pub utilisation: f64,
}

// ── Wave header ────────────────────────────────────────────────

/// Header info for a committed wave, returned by `pyde_getWave`.
///
/// Field shape is opaque-ish — the engine emits the full
/// `WaveRecord` Borsh-shaped JSON; advanced callers should
/// `serde_json::from_value` if they need typed access to nested
/// fields. v1 surfaces the common header fields for ergonomic use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveHeader {
    /// Wave id (a.k.a. "block number" in Ethereum vocabulary), hex.
    #[serde(default)]
    pub wave_id: Option<String>,
    /// Wall-clock commit timestamp, hex (seconds since Unix epoch).
    #[serde(default)]
    pub timestamp: Option<String>,
    /// Wave proposer address, hex.
    #[serde(default)]
    pub proposer: Option<String>,
    /// Post-commit JMT state root, hex.
    #[serde(default)]
    pub state_root: Option<String>,
    /// Number of transactions committed in this wave, hex.
    #[serde(default)]
    pub tx_count: Option<String>,
}

/// Ethereum-vocabulary alias for [`WaveHeader`].
///
/// Pyde calls a committed batch of transactions a "wave"; if you're
/// porting code that uses the Ethereum-style `block` terminology,
/// this alias lets you keep the old name.
pub type BlockHeader = WaveHeader;

// ── Log alias for backward-compat ──────────────────────────────

/// Ethereum-vocabulary alias for [`Event`].
///
/// Pyde events are NOT identical to Ethereum "logs" — they're a
/// Pyde-native type with wave/tx/event positional identity — but
/// the alias keeps Ethereum-tooling muscle memory working. New
/// code should use [`Event`] directly.
pub type Log = Event;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // ── Receipt decoding ────────────────────────────────────────

    /// Regression: engine ships `revert_reason.category` lowercase
    /// snake_case (`"engine_validation"`, `"contract"`, `"vm"`). Without
    /// the `#[serde(rename_all = "snake_case")]` rename every variant
    /// falls through to `Other(...)` and `is_known()` /
    /// `is_contract_revert()` / etc. return `false`.
    #[test]
    fn revert_category_decodes_engine_snake_case() {
        let v: RevertCategory = serde_json::from_str(r#""engine_validation""#).unwrap();
        assert_eq!(v, RevertCategory::EngineValidation);
        assert!(v.is_known());
        let v: RevertCategory = serde_json::from_str(r#""contract""#).unwrap();
        assert_eq!(v, RevertCategory::Contract);
        let v: RevertCategory = serde_json::from_str(r#""vm""#).unwrap();
        assert_eq!(v, RevertCategory::Vm);
        // Forward-compat catch-all still works for an unknown category.
        let v: RevertCategory = serde_json::from_str(r#""future_category""#).unwrap();
        assert!(!v.is_known());
        assert!(matches!(v, RevertCategory::Other(ref s) if s == "future_category"));
    }

    #[test]
    fn receipt_decodes_engine_shape() {
        let raw = r#"{
            "tx_hash": "0xdead",
            "wave_id": "0x5",
            "tx_index": "0x2",
            "status": "success",
            "gas_used": "0x5208",
            "fee_paid": "0x12345",
            "return_data": "0x",
            "events": []
        }"#;
        let r: Receipt = serde_json::from_str(raw).unwrap();
        assert_eq!(r.gas(), 0x5208);
        assert_eq!(r.fee_paid_quanta(), 0x12345);
        assert_eq!(r.wave_id_u64(), 5);
        assert_eq!(r.tx_index_u32(), 2);
        assert!(r.is_success());
    }

    #[test]
    fn receipt_handles_status_variants() {
        let payload = |status: &str| {
            format!(
                r#"{{ "tx_hash": "0x00", "wave_id": "0x0", "tx_index": "0x0",
                  "status": "{status}", "gas_used": "0x0", "fee_paid": "0x0" }}"#
            )
        };
        let s: Receipt = serde_json::from_str(&payload("success")).unwrap();
        let r: Receipt = serde_json::from_str(&payload("reverted")).unwrap();
        let o: Receipt = serde_json::from_str(&payload("out_of_gas")).unwrap();
        assert_eq!(s.status, ReceiptStatus::Success);
        assert_eq!(r.status, ReceiptStatus::Reverted);
        assert_eq!(o.status, ReceiptStatus::OutOfGas);
    }

    #[test]
    fn contract_address_extracts_from_32_byte_return() {
        let bytes = [0xAB; 32];
        let r = Receipt {
            tx_hash: String::new(),
            wave_id: "0x0".into(),
            tx_index: "0x0".into(),
            status: ReceiptStatus::Success,
            gas_used: String::new(),
            fee_paid: String::new(),
            return_data: format!("0x{}", hex::encode(bytes)),
            events: vec![],
            revert_reason: None,
        };
        let addr = r.contract_address().unwrap();
        assert_eq!(addr.as_bytes(), &bytes);
    }

    // ── Event decoding ──────────────────────────────────────────

    #[test]
    fn event_decodes_engine_shape() {
        let raw = r#"{
            "wave_id": "0x10",
            "tx_index": "0x3",
            "event_index": "0x1",
            "contract_addr": "0xabcd",
            "topics": ["0x1111", "0x2222"],
            "data": "0xbeef"
        }"#;
        let e: Event = serde_json::from_str(raw).unwrap();
        assert_eq!(e.wave_id_u64(), 0x10);
        assert_eq!(e.tx_index_u32(), 3);
        assert_eq!(e.event_index_u32(), 1);
        assert_eq!(e.topics.len(), 2);
        assert_eq!(e.data_bytes(), vec![0xBE, 0xEF]);
    }

    // ── AccountInfo decoding ────────────────────────────────────

    #[test]
    fn account_info_decodes_engine_shape() {
        let raw = r#"{
            "address": "0xa1",
            "account_type": "eoa",
            "balance": "0x3b9aca00",
            "nonce": 7,
            "code_hash": "0x0000000000000000000000000000000000000000000000000000000000000000",
            "state_root": "0x0000000000000000000000000000000000000000000000000000000000000000"
        }"#;
        let a: AccountInfo = serde_json::from_str(raw).unwrap();
        assert_eq!(a.balance_quanta(), 1_000_000_000);
        assert_eq!(a.nonce, 7);
        assert!(!a.is_contract());
    }

    // ── LogFilter serialisation ────────────────────────────────

    #[test]
    fn log_filter_skips_empties() {
        let f = LogFilter {
            from_wave: Some("0x5".into()),
            contracts: vec!["0xabcd".into()],
            ..Default::default()
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("from_wave"));
        assert!(json.contains("contracts"));
        assert!(!json.contains("to_wave"));
        assert!(!json.contains("topics"));
        assert!(!json.contains("cursor"));
        assert!(!json.contains("limit"));
    }

    // ── Simulation shape ───────────────────────────────────────

    #[test]
    fn simulation_result_round_trip() {
        let raw = r#"{
            "receipt": {
                "status": "Success",
                "gas_used": "0x5208",
                "fee_paid": "0x12345",
                "return_data": "0x"
            },
            "access_list": {
                "reads": [
                    { "slot": "0xaa", "observed_version": null }
                ],
                "writes": ["0xbb"]
            }
        }"#;
        let s: SimulationResult = serde_json::from_str(raw).unwrap();
        let receipt = s.receipt.unwrap();
        assert_eq!(receipt.status, "Success");
        assert_eq!(s.access_list.reads.len(), 1);
        assert_eq!(s.access_list.writes.len(), 1);
    }

    // ── EventFilter ──────────────────────────────────────────

    #[test]
    fn event_filter_uses_camel_case() {
        let f = EventFilter {
            from_wave: Some("0x0".into()),
            to_wave: Some("0xff".into()),
            contract: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        // Engine `pyde_getEvents` uses camelCase wire names.
        assert!(json.contains("fromWave"));
        assert!(json.contains("toWave"));
        assert!(!json.contains("contract\""));
    }
}
