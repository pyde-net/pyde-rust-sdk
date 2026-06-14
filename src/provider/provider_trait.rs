//! The [`Provider`] trait + the concrete [`RootProvider`] dispatcher.
//!
//! All 23 RPC methods the engine exposes today, plus convenience
//! sugar for typed Borsh payloads and PYDE↔quanta conversion at
//! the SDK boundary.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::SdkError;
use crate::types::{
    AccountInfo, Address, CallRequest, Event, EventFilter, LogFilter, LogPage, NodeInfo, Receipt,
    SimulationResult, Tx, TxHash,
};

use super::pending::PendingTx;
use super::transport::Transport;

// ── Provider trait ─────────────────────────────────────────────

/// The SDK's RPC abstraction.
///
/// Object-safe via `async_trait` so callers can hold
/// `Arc<dyn Provider>` and swap implementations behind a trait
/// object. Concrete impl: [`RootProvider`] over an
/// [`HttpTransport`][crate::provider::HttpTransport] or
/// `WsTransport` ([`crate::ws`]).
///
/// Method names match the engine's JSON-RPC catalog with the
/// `pyde_` prefix stripped and `snake_case`d (e.g.
/// `pyde_getBalance` → [`Provider::get_balance`]).
#[async_trait]
pub trait Provider: Send + Sync {
    // ── Chain info ──────────────────────────────────────────────

    /// `pyde_chainId` — the chain identifier this node serves.
    async fn chain_id(&self) -> Result<u64, SdkError>;

    /// `pyde_waveId` — the latest committed wave id.
    async fn wave_id(&self) -> Result<u64, SdkError>;

    /// `pyde_getNodeInfo` — node identity (peer id, FALCON pubkey,
    /// listen addresses, agent + protocol version).
    async fn get_node_info(&self) -> Result<NodeInfo, SdkError>;

    /// `pyde_getMetrics` — opaque metrics snapshot.
    ///
    /// Returned as raw [`Value`] for now — schema evolves
    /// independently of the SDK release cycle. Advanced callers
    /// `serde_json::from_value` into their own struct.
    async fn get_metrics(&self) -> Result<Value, SdkError>;

    // ── Account state ───────────────────────────────────────────

    /// `pyde_getBalance` — account balance in quanta.
    async fn get_balance(&self, addr: &Address) -> Result<u128, SdkError>;

    /// `pyde_getTransactionCount` — the **next** acceptable nonce
    /// (sliding-window base + trailing-ones offset). Matches the
    /// Ethereum getTransactionCount semantics but uses Pyde's
    /// 16-slot window under the hood.
    async fn get_nonce(&self, addr: &Address) -> Result<u64, SdkError>;

    /// `pyde_getAccount` — the full account record (type, balance,
    /// nonce, code hash, storage root).
    async fn get_account(&self, addr: &Address) -> Result<AccountInfo, SdkError>;

    /// `pyde_getContractCode` — deployed WASM bytecode for the
    /// account at `addr`. Empty Vec for EOAs / un-deployed
    /// addresses.
    async fn get_contract_code(&self, addr: &Address) -> Result<Vec<u8>, SdkError>;

    /// `pyde_getStorageSlot` — raw storage-slot value.
    ///
    /// `slot` is the full 32-byte PIP-2 clustered key. Returns
    /// `None` for an unset slot. Note: contracts in v1 supply the
    /// full slot key at `sstore` time per HOST_FN_ABI §7.6 — the
    /// SDK doesn't fabricate the key.
    async fn get_storage_slot(&self, slot: &[u8; 32]) -> Result<Option<Vec<u8>>, SdkError>;

    /// `pyde_resolveName` — name registry lookup.
    ///
    /// Returns `Some(addr)` for a registered name, `None` for an
    /// unregistered name. Invalid name formats are surfaced as
    /// [`SdkError::InvalidArgument`].
    async fn resolve_name(&self, name: &str) -> Result<Option<Address>, SdkError>;

    // ── Tx submission + execution ───────────────────────────────

    /// `pyde_sendRawTransaction` — submit a signed transaction.
    ///
    /// The SDK Borsh-encodes [`Tx`] internally, hex-prefixes the
    /// bytes, and submits. Returns the engine-computed [`TxHash`].
    async fn send_raw_transaction(&self, tx: &Tx) -> Result<TxHash, SdkError>;

    /// `pyde_call` — read-only contract view call.
    ///
    /// Returns the contract's `pyde::return(...)` bytes. Use the
    /// SDK's contract layer (T10) for typed args + return decoding.
    async fn call(&self, req: &CallRequest) -> Result<Vec<u8>, SdkError>;

    /// `pyde_simulateTransaction` — dry-run + observed access list.
    ///
    /// Returns the receipt-that-would-be plus the (reads, writes)
    /// pattern the executor saw. Use the access list to populate
    /// the real submission so the scheduler can run the tx in
    /// parallel under Block-STM.
    async fn simulate_transaction(&self, tx: &Tx) -> Result<SimulationResult, SdkError>;

    // ── Receipts + history ──────────────────────────────────────

    /// `pyde_getTransactionReceipt` — committed receipt by hash.
    ///
    /// Returns `None` if the tx hasn't committed yet (or never
    /// will). Use [`PendingTx::wait_for_receipt`] when you want to
    /// block until commitment.
    async fn get_transaction_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError>;

    /// `pyde_getReceipt` — committed receipt from the consensus
    /// store (same shape as [`Provider::get_transaction_receipt`]
    /// but sourced from the longer-lived consensus archive vs the
    /// active state's hot map).
    async fn get_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError>;

    /// `pyde_getTx` — full committed transaction by hash.
    async fn get_tx(&self, hash: &TxHash) -> Result<Option<Tx>, SdkError>;

    /// `pyde_getWave` — opaque wave record by id.
    ///
    /// Returned as raw [`Value`]; advanced callers
    /// `serde_json::from_value` into their own typed struct.
    async fn get_wave(&self, wave_id: u64) -> Result<Option<Value>, SdkError>;

    // ── Events ──────────────────────────────────────────────────

    /// `pyde_getEvents` — simple event lookup by wave range +
    /// optional single-contract filter.
    ///
    /// No pagination — use [`Provider::get_logs`] for multi-contract
    /// + topic-filtered + paged queries.
    async fn get_events(&self, filter: &EventFilter) -> Result<Vec<Event>, SdkError>;

    /// `pyde_getLogs` — full event-log query with topic matching,
    /// multi-contract OR, and cursor-resumable pagination.
    async fn get_logs(&self, filter: &LogFilter) -> Result<LogPage, SdkError>;

    // ── Validators ──────────────────────────────────────────────

    /// `pyde_getValidator` — validator record by validator address.
    async fn get_validator(&self, addr: &Address) -> Result<Option<Value>, SdkError>;

    /// `pyde_getOperatorValidators` — all validators owned by an
    /// operator.
    async fn get_operator_validators(&self, operator: &Address) -> Result<Vec<Value>, SdkError>;

    // ── Snapshots ───────────────────────────────────────────────

    /// `pyde_getSnapshot` — full state snapshot bundle (manifest +
    /// chunks). Heavy response; prefer [`Provider::get_snapshot_manifest`]
    /// when only the chunk-hash list is needed.
    async fn get_snapshot(&self) -> Result<Value, SdkError>;

    /// `pyde_getSnapshotManifest` — manifest-only response
    /// (`wave_id, state_root, chunk_size, chunk_count, chunk_hashes,
    /// total_keys`). Use as a weak-subjectivity checkpoint without
    /// shipping the chunk payloads.
    async fn get_snapshot_manifest(&self) -> Result<Value, SdkError>;
}

// ── RootProvider ───────────────────────────────────────────────

/// The default [`Provider`] dispatcher, generic over a [`Transport`].
///
/// Construction:
///
/// ```ignore
/// use std::sync::Arc;
/// use pyde_rust_sdk::provider::{HttpTransport, RootProvider};
///
/// let transport = HttpTransport::new("http://127.0.0.1:9933")?;
/// let provider = Arc::new(RootProvider::new(transport));
/// // Now usable as `Arc<dyn Provider>`:
/// let chain_id = provider.chain_id().await?;
/// ```
///
/// `RootProvider<T>` is `Clone` so it can be stamped behind multiple
/// `Arc`s if you need shared ownership across tasks.
#[derive(Clone)]
pub struct RootProvider<T: Transport> {
    transport: Arc<T>,
}

impl<T: Transport> RootProvider<T> {
    /// Wrap a transport.
    #[must_use]
    pub fn new(transport: T) -> Self {
        Self {
            transport: Arc::new(transport),
        }
    }

    /// Borrow the underlying transport — useful for low-level calls
    /// the trait surface doesn't cover (e.g. one-off probing).
    #[must_use]
    pub fn transport(&self) -> &T {
        &self.transport
    }
}

impl<T: Transport + 'static> RootProvider<T> {
    /// Submit a signed transaction and return a [`PendingTx`] handle
    /// that can be awaited for the on-chain receipt.
    ///
    /// Equivalent to `send_raw_transaction(tx) + PendingTx::new`
    /// but wraps the boilerplate in one call.
    ///
    /// # Errors
    /// Surface whatever [`Provider::send_raw_transaction`] surfaces.
    pub async fn send_transaction(self: &Arc<Self>, tx: &Tx) -> Result<PendingTx, SdkError> {
        let hash = self.send_raw_transaction(tx).await?;
        let provider: Arc<dyn Provider> = self.clone();
        Ok(PendingTx::new(hash, provider))
    }
}

impl<T: Transport> std::fmt::Debug for RootProvider<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RootProvider").finish_non_exhaustive()
    }
}

// ── RootProvider as Provider ───────────────────────────────────

#[async_trait]
impl<T: Transport + 'static> Provider for RootProvider<T> {
    async fn chain_id(&self) -> Result<u64, SdkError> {
        let v = self.transport.send("pyde_chainId", json!([])).await?;
        decode_hex_u64(&v, "chain_id")
    }

    async fn wave_id(&self) -> Result<u64, SdkError> {
        let v = self.transport.send("pyde_waveId", json!([])).await?;
        decode_hex_u64(&v, "wave_id")
    }

    async fn get_node_info(&self) -> Result<NodeInfo, SdkError> {
        let v = self.transport.send("pyde_getNodeInfo", json!([])).await?;
        serde_json::from_value(v)
            .map_err(|e| SdkError::InvalidResponse(format!("get_node_info: {e}")))
    }

    async fn get_metrics(&self) -> Result<Value, SdkError> {
        self.transport.send("pyde_getMetrics", json!([])).await
    }

    async fn get_balance(&self, addr: &Address) -> Result<u128, SdkError> {
        let v = self
            .transport
            .send("pyde_getBalance", json!([addr.to_hex()]))
            .await?;
        decode_hex_u128(&v, "balance")
    }

    async fn get_nonce(&self, addr: &Address) -> Result<u64, SdkError> {
        let v = self
            .transport
            .send("pyde_getTransactionCount", json!([addr.to_hex()]))
            .await?;
        decode_hex_u64(&v, "nonce")
    }

    async fn get_account(&self, addr: &Address) -> Result<AccountInfo, SdkError> {
        let v = self
            .transport
            .send("pyde_getAccount", json!([addr.to_hex()]))
            .await?;
        serde_json::from_value(v)
            .map_err(|e| SdkError::InvalidResponse(format!("get_account: {e}")))
    }

    async fn get_contract_code(&self, addr: &Address) -> Result<Vec<u8>, SdkError> {
        let v = self
            .transport
            .send("pyde_getContractCode", json!([addr.to_hex()]))
            .await?;
        decode_hex_bytes(&v, "contract_code")
    }

    async fn get_storage_slot(&self, slot: &[u8; 32]) -> Result<Option<Vec<u8>>, SdkError> {
        let slot_hex = format!("0x{}", hex::encode(slot));
        let v = self
            .transport
            .send("pyde_getStorageSlot", json!([{"slot": slot_hex}]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        decode_hex_bytes(&v, "storage_slot").map(Some)
    }

    async fn resolve_name(&self, name: &str) -> Result<Option<Address>, SdkError> {
        let v = self
            .transport
            .send("pyde_resolveName", json!([name]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        let s = v.as_str().ok_or_else(|| {
            SdkError::InvalidResponse(format!("resolve_name: expected string, got {v}"))
        })?;
        Address::from_hex(s).map(Some)
    }

    async fn send_raw_transaction(&self, tx: &Tx) -> Result<TxHash, SdkError> {
        let bytes = crate::tx::encode(tx)?;
        let payload = format!("0x{}", hex::encode(&bytes));
        let v = self
            .transport
            .send("pyde_sendRawTransaction", json!([payload]))
            .await?;
        let s = v.as_str().ok_or_else(|| {
            SdkError::InvalidResponse(format!("send_raw_transaction: expected string, got {v}"))
        })?;
        let bytes = hex::decode(s.trim_start_matches("0x"))
            .map_err(|e| SdkError::InvalidResponse(format!("tx_hash hex: {e}")))?;
        if bytes.len() != 32 {
            return Err(SdkError::InvalidResponse(format!(
                "tx_hash: expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(TxHash::new(arr))
    }

    async fn call(&self, req: &CallRequest) -> Result<Vec<u8>, SdkError> {
        let v = self.transport.send("pyde_call", json!([req])).await?;
        decode_hex_bytes(&v, "call")
    }

    async fn simulate_transaction(&self, tx: &Tx) -> Result<SimulationResult, SdkError> {
        let bytes = crate::tx::encode(tx)?;
        let payload = format!("0x{}", hex::encode(&bytes));
        let v = self
            .transport
            .send("pyde_simulateTransaction", json!([payload]))
            .await?;
        serde_json::from_value(v)
            .map_err(|e| SdkError::InvalidResponse(format!("simulate_transaction: {e}")))
    }

    async fn get_transaction_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError> {
        let hash_hex = format!("0x{}", hex::encode(hash.as_bytes()));
        let v = self
            .transport
            .send("pyde_getTransactionReceipt", json!([hash_hex]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v)
            .map(Some)
            .map_err(|e| SdkError::InvalidResponse(format!("get_transaction_receipt: {e}")))
    }

    async fn get_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError> {
        let hash_hex = format!("0x{}", hex::encode(hash.as_bytes()));
        let v = self
            .transport
            .send("pyde_getReceipt", json!([hash_hex]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v)
            .map(Some)
            .map_err(|e| SdkError::InvalidResponse(format!("get_receipt: {e}")))
    }

    async fn get_tx(&self, hash: &TxHash) -> Result<Option<Tx>, SdkError> {
        let hash_hex = format!("0x{}", hex::encode(hash.as_bytes()));
        let v = self.transport.send("pyde_getTx", json!([hash_hex])).await?;
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v)
            .map(Some)
            .map_err(|e| SdkError::InvalidResponse(format!("get_tx: {e}")))
    }

    async fn get_wave(&self, wave_id: u64) -> Result<Option<Value>, SdkError> {
        let v = self
            .transport
            .send("pyde_getWave", json!([wave_id]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(v))
    }

    async fn get_events(&self, filter: &EventFilter) -> Result<Vec<Event>, SdkError> {
        let v = self
            .transport
            .send("pyde_getEvents", json!([filter]))
            .await?;
        serde_json::from_value(v).map_err(|e| SdkError::InvalidResponse(format!("get_events: {e}")))
    }

    async fn get_logs(&self, filter: &LogFilter) -> Result<LogPage, SdkError> {
        let v = self.transport.send("pyde_getLogs", json!([filter])).await?;
        serde_json::from_value(v).map_err(|e| SdkError::InvalidResponse(format!("get_logs: {e}")))
    }

    async fn get_validator(&self, addr: &Address) -> Result<Option<Value>, SdkError> {
        let v = self
            .transport
            .send("pyde_getValidator", json!([addr.to_hex()]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(v))
    }

    async fn get_operator_validators(&self, operator: &Address) -> Result<Vec<Value>, SdkError> {
        let v = self
            .transport
            .send("pyde_getOperatorValidators", json!([operator.to_hex()]))
            .await?;
        if v.is_null() {
            return Ok(Vec::new());
        }
        serde_json::from_value(v)
            .map_err(|e| SdkError::InvalidResponse(format!("get_operator_validators: {e}")))
    }

    async fn get_snapshot(&self) -> Result<Value, SdkError> {
        self.transport.send("pyde_getSnapshot", json!([])).await
    }

    async fn get_snapshot_manifest(&self) -> Result<Value, SdkError> {
        self.transport
            .send("pyde_getSnapshotManifest", json!([]))
            .await
    }
}

// ── Decode helpers ─────────────────────────────────────────────

fn decode_hex_u64(v: &Value, ctx: &str) -> Result<u64, SdkError> {
    let s = v
        .as_str()
        .ok_or_else(|| SdkError::InvalidResponse(format!("{ctx}: expected hex string, got {v}")))?;
    let stripped = s.trim_start_matches("0x");
    u64::from_str_radix(stripped, 16)
        .map_err(|e| SdkError::InvalidResponse(format!("{ctx}: bad hex u64 {s:?}: {e}")))
}

fn decode_hex_u128(v: &Value, ctx: &str) -> Result<u128, SdkError> {
    let s = v
        .as_str()
        .ok_or_else(|| SdkError::InvalidResponse(format!("{ctx}: expected hex string, got {v}")))?;
    let stripped = s.trim_start_matches("0x");
    u128::from_str_radix(stripped, 16)
        .map_err(|e| SdkError::InvalidResponse(format!("{ctx}: bad hex u128 {s:?}: {e}")))
}

fn decode_hex_bytes(v: &Value, ctx: &str) -> Result<Vec<u8>, SdkError> {
    let s = v
        .as_str()
        .ok_or_else(|| SdkError::InvalidResponse(format!("{ctx}: expected hex string, got {v}")))?;
    let stripped = s.trim_start_matches("0x");
    hex::decode(stripped)
        .map_err(|e| SdkError::InvalidResponse(format!("{ctx}: bad hex bytes: {e}")))
}
