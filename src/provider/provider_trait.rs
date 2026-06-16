//! The [`Provider`] trait + the concrete [`RootProvider`] dispatcher.
//!
//! All 26 RPC methods the engine exposes today, plus convenience
//! sugar for typed Borsh payloads and PYDE↔quanta conversion at
//! the SDK boundary.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::error::SdkError;
use crate::types::{
    AccountInfo, Address, CallRequest, Event, EventFilter, FeeData, LogFilter, LogPage, NodeInfo,
    Receipt, RecentWaveSummary, SimulationResult, ThresholdPublicKey, Tx, TxHash,
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

    /// `pyde_getNonce` — the **next** acceptable nonce
    /// (sliding-window base + trailing-ones offset). Uses Pyde's
    /// 16-slot window under the hood; semantically equivalent to
    /// Ethereum's `getTransactionCount` but driven by the wave-
    /// scoped nonce window. The engine accepts both
    /// `pyde_getNonce` (canonical, Chapter 17.4) and
    /// `pyde_getTransactionCount` (pre-pivot alias); the SDK
    /// sends the canonical name.
    async fn get_nonce(&self, addr: &Address) -> Result<u64, SdkError>;

    /// `pyde_getAccount` — the full account record (type, balance,
    /// nonce, code hash, storage root).
    ///
    /// Returns `AccountInfo` (never `Option`) — for addresses the
    /// chain has never seen, the engine synthesises a fresh-EOA
    /// fallback record (`account_type: "eoa"`, `balance: 0`,
    /// `nonce: 0`, zero code/storage roots). Callers needing
    /// "is this a real on-chain account?" should check
    /// `balance > 0 || nonce > 0 || code_hash != ZERO` rather
    /// than expecting `None` for unseen addresses.
    async fn get_account(&self, addr: &Address) -> Result<AccountInfo, SdkError>;

    /// `pyde_getContractCode` — deployed WASM bytecode for the
    /// account at `addr`. Empty Vec for EOAs / un-deployed
    /// addresses.
    async fn get_contract_code(&self, addr: &Address) -> Result<Vec<u8>, SdkError>;

    /// `pyde_getStorageSlot` — raw storage-slot value.
    ///
    /// `slot` is the full 32-byte derived slot key. Returns `None`
    /// for an unset slot.
    ///
    /// Pyde stores state as a flat 32-byte-key → variable-value
    /// map; contracts derive the key themselves at `sstore` /
    /// `sload` time per HOST_FN_ABI §7.6. Two conventions are in
    /// active use:
    ///
    /// - **PIP-2 clustered** (otigen-compiled Rust contracts) —
    ///   `address[..16] || Poseidon2(disc || slot_index_le)[..16]`.
    /// - **Field-name Poseidon2** (hand-rolled Go / C contracts) —
    ///   `Poseidon2(contract_address || field_name || key)`.
    ///
    /// Explorers re-deriving the key need to know which convention
    /// a given contract uses. See `docs/06-providers.md` for the
    /// full layout + how to identify each via the bundle's ABI.
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

    /// `pyde_sendRawEncryptedTransaction` — submit a threshold-encrypted
    /// transaction envelope for MEV protection.
    ///
    /// The caller must first encrypt a plaintext `Tx` under the
    /// chain's current threshold pubkey (from
    /// [`Self::get_threshold_public_key`]) and produce a borsh-encoded
    /// `EncryptedTxEnvelope` (version byte + ciphertext). This method
    /// takes that envelope as hex.
    ///
    /// Returns the 32-byte Blake3 envelope hash. **NOT** the inner
    /// `tx_hash` — that lives on the plaintext and is only knowable
    /// post-decryption. Receipts go under the plaintext hash; use
    /// `pyde_getReceipt` polling on it.
    ///
    /// v1 mock-DKG warning: if [`Self::get_threshold_public_key`]
    /// reports `scheme: "mock"`, submitted envelopes won't be
    /// processed until real Kyber-768 crypto lands. Treat
    /// `scheme != "kyber-768"` as "encrypted path unavailable."
    ///
    /// Size limits (engine v1): min 1213 bytes, max 128 KiB.
    ///
    /// # Errors
    /// - `InvalidArgument` for hex / borsh decode failures.
    /// - `Rpc` for mempool admission failures
    ///   (`AlreadyKnown`, `PoolFull`, `UnsupportedVersion`,
    ///   `CiphertextTooSmall`, `CiphertextTooLarge`).
    async fn send_raw_encrypted_transaction(&self, envelope_hex: &str) -> Result<TxHash, SdkError>;

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
    /// archive (sourced from the longer-lived consensus_store vs
    /// the hot active-state map [`Self::get_transaction_receipt`]
    /// reads from).
    ///
    /// Same wire shape as [`Self::get_transaction_receipt`] — hex
    /// strings throughout — so the deserialised type is identical.
    /// Use this for archival queries (e.g., explorer back-pages
    /// past the hot-state TTL); for dapps + wallets that just
    /// want "did my tx commit?" use
    /// [`Self::get_transaction_receipt`] — same underlying data,
    /// hot path is faster.
    async fn get_receipt(&self, hash: &TxHash) -> Result<Option<Receipt>, SdkError>;

    /// `pyde_getTx` — full committed transaction by hash.
    ///
    /// Engine emits raw `serde_json::to_value(Tx)`; our derived
    /// `Deserialize` on [`Tx`] handles this because the embedded
    /// types ([`Address`], [`crate::types::FalconSignature`],
    /// `FeePayer`) all use derived serde that aligns. No separate
    /// type needed.
    async fn get_tx(&self, hash: &TxHash) -> Result<Option<Tx>, SdkError>;

    /// `pyde_getWave` — opaque wave record by id.
    ///
    /// Returned as raw [`Value`]; advanced callers
    /// `serde_json::from_value` into their own typed struct.
    async fn get_wave(&self, wave_id: u64) -> Result<Option<Value>, SdkError>;

    /// `pyde_getWave` (no-arg form) — the latest committed wave.
    ///
    /// Pairs with the engine's light-client head query: returns
    /// the highest-numbered committed wave in one round-trip
    /// instead of `wave_id()` + `get_wave(N)`.
    ///
    /// Result shape matches [`Self::get_wave`]. `None` only if
    /// the chain has never committed a wave (genesis-only / pre-
    /// boot state).
    async fn get_wave_head(&self) -> Result<Option<Value>, SdkError>;

    /// `pyde_getFeeData` — current fee snapshot + recent-wave
    /// utilisation summary.
    ///
    /// Single round-trip alternative to driving a wallet's
    /// gas-price slider with per-wave `get_wave` calls. Returns
    /// [`FeeData`] — `base_fee` + `suggested_tip` (always `0`
    /// in v1) + `wave_id` + last 10 waves' utilisation.
    async fn get_fee_data(&self) -> Result<FeeData, SdkError>;

    /// `pyde_getHardFinalityCert` — wave finality certificate.
    ///
    /// The bundle of ≥ `QUORUM` (85) validator signatures proving
    /// wave `wave_id` is hard-finalised. Used by light clients,
    /// cross-chain bridges, and zk-rollup verifiers that don't
    /// want to trust the RPC node's "this is final" claim.
    ///
    /// Returns `None` if the wave isn't finalised yet (or never
    /// existed). Result shape is raw-serde
    /// `HardFinalityCert { commit: WaveCommitRecord, signatures:
    /// Vec<(u32, Vec<u8>)> }`; returned as opaque [`Value`] today,
    /// callers `serde_json::from_value` into their own typed
    /// struct.
    ///
    /// `wave_id` is sent as a bare JSON number on the wire (not
    /// hex string) — engine quirk shared with `pyde_getWave`.
    async fn get_hard_finality_cert(&self, wave_id: u64) -> Result<Option<Value>, SdkError>;

    /// `pyde_getThresholdPublicKey` — threshold-decryption pubkey
    /// for the encrypted-mempool path.
    ///
    /// Wallets encrypt a plaintext `Tx` under `result.public_key`
    /// before submitting via
    /// [`Self::send_raw_encrypted_transaction`]. Per-epoch — refresh
    /// per encrypted submit (cheap, no consensus round-trip).
    ///
    /// Returns `None` if no DKG ceremony has run yet. v1 boot
    /// writes a deterministic mock pubkey (`scheme: "mock"`) so
    /// the encrypted-mempool path is reachable from the first
    /// wave; real Kyber-768 crypto overwrites it at the per-epoch
    /// combine. Treat `scheme != "kyber-768"` as "encrypted path
    /// not yet ready, fall back to plaintext."
    async fn get_threshold_public_key(&self) -> Result<Option<ThresholdPublicKey>, SdkError>;

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
            .send("pyde_getNonce", json!([addr.to_hex()]))
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
        decode_tx_hash_from_value(&v, "send_raw_transaction")
    }

    async fn send_raw_encrypted_transaction(&self, envelope_hex: &str) -> Result<TxHash, SdkError> {
        let payload = if envelope_hex.starts_with("0x") {
            envelope_hex.to_string()
        } else {
            format!("0x{envelope_hex}")
        };
        let v = self
            .transport
            .send("pyde_sendRawEncryptedTransaction", json!([payload]))
            .await?;
        decode_tx_hash_from_value(&v, "send_raw_encrypted_transaction")
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

    async fn get_wave_head(&self) -> Result<Option<Value>, SdkError> {
        let v = self.transport.send("pyde_getWave", json!([])).await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(v))
    }

    async fn get_fee_data(&self) -> Result<FeeData, SdkError> {
        let v = self.transport.send("pyde_getFeeData", json!([])).await?;
        let obj = v.as_object().ok_or_else(|| {
            SdkError::InvalidResponse(format!("get_fee_data: expected object, got {v}"))
        })?;
        let base_fee = decode_hex_u128(
            obj.get("base_fee").ok_or_else(|| {
                SdkError::InvalidResponse("get_fee_data: missing base_fee".into())
            })?,
            "base_fee",
        )?;
        let suggested_tip = decode_hex_u128(
            obj.get("suggested_tip").ok_or_else(|| {
                SdkError::InvalidResponse("get_fee_data: missing suggested_tip".into())
            })?,
            "suggested_tip",
        )?;
        let wave_id = decode_hex_u64(
            obj.get("wave_id")
                .ok_or_else(|| SdkError::InvalidResponse("get_fee_data: missing wave_id".into()))?,
            "wave_id",
        )?;
        let recent_arr = obj
            .get("recent_waves")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                SdkError::InvalidResponse("get_fee_data: missing recent_waves array".into())
            })?;
        let mut recent_waves = Vec::with_capacity(recent_arr.len());
        for (i, entry) in recent_arr.iter().enumerate() {
            let e = entry.as_object().ok_or_else(|| {
                SdkError::InvalidResponse(format!("recent_waves[{i}]: not an object"))
            })?;
            let wave_id = decode_hex_u64(
                e.get("wave_id").ok_or_else(|| {
                    SdkError::InvalidResponse(format!("recent_waves[{i}].wave_id missing"))
                })?,
                "recent_waves.wave_id",
            )?;
            let gas_used = decode_hex_u64(
                e.get("gas_used").ok_or_else(|| {
                    SdkError::InvalidResponse(format!("recent_waves[{i}].gas_used missing"))
                })?,
                "recent_waves.gas_used",
            )?;
            let gas_limit = decode_hex_u64(
                e.get("gas_limit").ok_or_else(|| {
                    SdkError::InvalidResponse(format!("recent_waves[{i}].gas_limit missing"))
                })?,
                "recent_waves.gas_limit",
            )?;
            let utilisation = e
                .get("utilisation")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    SdkError::InvalidResponse(format!(
                        "recent_waves[{i}].utilisation missing or not a string"
                    ))
                })?
                .parse::<f64>()
                .map_err(|err| {
                    SdkError::InvalidResponse(format!("recent_waves[{i}].utilisation: {err}"))
                })?;
            recent_waves.push(RecentWaveSummary {
                wave_id,
                gas_used,
                gas_limit,
                utilisation,
            });
        }
        Ok(FeeData {
            base_fee,
            suggested_tip,
            wave_id,
            recent_waves,
        })
    }

    async fn get_hard_finality_cert(&self, wave_id: u64) -> Result<Option<Value>, SdkError> {
        // Bare u64 number param — engine quirk shared with pyde_getWave.
        let v = self
            .transport
            .send("pyde_getHardFinalityCert", json!([wave_id]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        Ok(Some(v))
    }

    async fn get_threshold_public_key(&self) -> Result<Option<ThresholdPublicKey>, SdkError> {
        let v = self
            .transport
            .send("pyde_getThresholdPublicKey", json!([]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v)
            .map(Some)
            .map_err(|e| SdkError::InvalidResponse(format!("get_threshold_public_key: {e}")))
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

/// Decode an RPC response that's a 32-byte hex string into a [`TxHash`].
/// Used by both `send_raw_transaction` and `send_raw_encrypted_transaction`.
fn decode_tx_hash_from_value(v: &Value, ctx: &str) -> Result<TxHash, SdkError> {
    let s = v
        .as_str()
        .ok_or_else(|| SdkError::InvalidResponse(format!("{ctx}: expected hex string, got {v}")))?;
    let bytes = hex::decode(s.trim_start_matches("0x"))
        .map_err(|e| SdkError::InvalidResponse(format!("{ctx}: tx_hash hex: {e}")))?;
    if bytes.len() != 32 {
        return Err(SdkError::InvalidResponse(format!(
            "{ctx}: tx_hash expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(TxHash::new(arr))
}
