//! Untyped contract runtime — `Contract<P>` wraps a provider +
//! contract address + parsed ABI, and exposes dynamic
//! `call(name, args)` / `send(name, args, signer)` / event filter
//! helpers.
//!
//! The `pyde_abi!` macro generates a typed shell around this
//! runtime; dapps that build calldata by hand can use this module
//! directly.

use std::sync::Arc;

use crate::error::SdkError;
use crate::provider::{PendingTx, Provider};
use crate::signer::Signer;
use crate::tx::TxBuilder;
use crate::types::{
    Address, CallOverrides, CallPayload, CallRequest, ContractAbi, Event, EventAbi, FunctionAbi,
    FunctionAttrs, LogFilter, Tx, TxType,
};

use super::codec::{decode_return, encode_calldata, Value};

/// Untyped contract handle — provider + address + ABI.
///
/// Built from a parsed [`ContractAbi`] (typically via
/// [`crate::abi::extract_abi`] on the deployed bytecode, or from
/// the inline ABI shipped with a `pyde_abi!` invocation).
pub struct Contract {
    address: Address,
    abi: ContractAbi,
    provider: Arc<dyn Provider>,
}

impl Contract {
    /// Build a handle from address + ABI + provider.
    #[must_use]
    pub fn new(address: Address, abi: ContractAbi, provider: Arc<dyn Provider>) -> Self {
        Self {
            address,
            abi,
            provider,
        }
    }

    /// Resolve a contract by name via [`Provider::resolve_name`] and
    /// fetch its ABI via [`Provider::get_contract_code`] +
    /// [`crate::abi::extract_abi`].
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] if the name doesn't resolve.
    /// - Any error from the underlying provider calls or the ABI
    ///   extraction.
    pub async fn load(name: &str, provider: Arc<dyn Provider>) -> Result<Self, SdkError> {
        let address = provider.resolve_name(name).await?.ok_or_else(|| {
            SdkError::InvalidArgument(format!("contract {name:?} is not registered"))
        })?;
        Self::load_at(address, provider).await
    }

    /// Load a contract at a known address. Fetches the bytecode
    /// and parses the embedded `pyde.abi`.
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] if no contract is deployed
    ///   at `address`.
    /// - Any error from [`Provider::get_contract_code`] or
    ///   [`crate::abi::extract_abi`].
    pub async fn load_at(address: Address, provider: Arc<dyn Provider>) -> Result<Self, SdkError> {
        let bytecode = provider.get_contract_code(&address).await?;
        if bytecode.is_empty() {
            return Err(SdkError::InvalidArgument(format!(
                "no contract code at {address}"
            )));
        }
        let abi = crate::abi::extract_abi(&bytecode)?;
        Ok(Self::new(address, abi, provider))
    }

    /// Contract address.
    #[must_use]
    pub fn address(&self) -> Address {
        self.address
    }

    /// Parsed ABI.
    #[must_use]
    pub fn abi(&self) -> &ContractAbi {
        &self.abi
    }

    /// Underlying provider.
    #[must_use]
    pub fn provider(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.provider)
    }

    // ── View call ───────────────────────────────────────────────

    /// Execute a read-only call.
    ///
    /// Builds a [`CallRequest`] from the function name + args,
    /// dispatches `pyde_call`, and decodes the return data per the
    /// ABI's `returns` field.
    ///
    /// # Errors
    /// - [`SdkError::InvalidArgument`] for unknown function names,
    ///   arity mismatches, calling a non-view function.
    /// - Any error from [`Provider::call`] or the return decoder.
    pub async fn call(&self, name: &str, args: Vec<Value>) -> Result<Option<Value>, SdkError> {
        self.call_with(name, args, CallOverrides::default()).await
    }

    /// Execute a read-only call with overrides on `from`, attached
    /// value, and gas budget.
    ///
    /// # Errors
    /// Same as [`Self::call`].
    pub async fn call_with(
        &self,
        name: &str,
        args: Vec<Value>,
        overrides: CallOverrides,
    ) -> Result<Option<Value>, SdkError> {
        let function = self.lookup_function(name)?;
        let calldata = encode_calldata(&function.params, &args)?;
        let payload = CallPayload {
            function: function.name.clone(),
            calldata,
        };
        let payload_bytes = borsh::to_vec(&payload)
            .map_err(|e| SdkError::Other(format!("borsh encode CallPayload: {e}")))?;
        let req = CallRequest {
            to: self.address.to_hex(),
            data: format!("0x{}", hex::encode(&payload_bytes)),
            from: overrides.from.map(|a| a.to_hex()),
            value: overrides.value.map(|v| format!("0x{v:x}")),
            gas: overrides.gas_limit.map(|g| format!("0x{g:x}")),
        };
        let return_bytes = self.provider.call(&req).await?;
        decode_return(&function.returns, &return_bytes)
    }

    // ── State-mutating send ────────────────────────────────────

    /// Build the unsigned [`Tx`] for a state-mutating call.
    ///
    /// Useful when the caller wants to sign + submit the tx out-of-
    /// band; for the common path use [`Self::send`] instead.
    ///
    /// # Errors
    /// Same arity / function-lookup errors as [`Self::call`].
    //
    // 8 args by design — every field is independently meaningful
    // for a chain-submitted Tx and there's no useful grouping.
    // Callers that want to reuse defaults should drive the call via
    // the typed `pyde_abi!`-generated wrappers or via [`Self::send`].
    #[allow(clippy::too_many_arguments)]
    pub fn build_tx(
        &self,
        from: Address,
        name: &str,
        args: Vec<Value>,
        chain_id: u64,
        nonce: u64,
        gas_limit: u64,
        value: u128,
    ) -> Result<Tx, SdkError> {
        let function = self.lookup_function(name)?;
        if value > 0 && !function.attrs.is_payable() {
            return Err(SdkError::InvalidArgument(format!(
                "function {name:?} is not payable; cannot attach value"
            )));
        }
        let calldata = encode_calldata(&function.params, &args)?;
        let payload = CallPayload {
            function: function.name.clone(),
            calldata,
        };
        let data = borsh::to_vec(&payload)
            .map_err(|e| SdkError::Other(format!("borsh encode CallPayload: {e}")))?;
        TxBuilder::new()
            .from(from)
            .to(self.address)
            .data(data)
            .gas_limit(gas_limit)
            .nonce(nonce)
            .chain_id(chain_id)
            .value(value)
            .tx_type(TxType::Standard)
            .build()
    }

    /// Sign + submit a state-mutating call.
    ///
    /// Looks up the function in the ABI, builds calldata, fetches
    /// `chain_id` + `nonce` from the provider, signs via `signer`,
    /// and submits — returning a [`PendingTx`] handle for the
    /// caller to await.
    ///
    /// # Errors
    /// - All errors from [`Self::build_tx`].
    /// - Any error from the provider or the signer.
    pub async fn send(
        &self,
        signer: &dyn Signer,
        name: &str,
        args: Vec<Value>,
        gas_limit: u64,
        value: u128,
    ) -> Result<PendingTx, SdkError> {
        let chain_id = self.provider.chain_id().await?;
        let nonce = self.provider.get_nonce(&signer.address()).await?;
        let mut tx = self.build_tx(
            signer.address(),
            name,
            args,
            chain_id,
            nonce,
            gas_limit,
            value,
        )?;
        signer.sign_tx(&mut tx).await?;
        let hash = self.provider.send_raw_transaction(&tx).await?;
        Ok(PendingTx::new(hash, Arc::clone(&self.provider)))
    }

    // ── Event helpers ──────────────────────────────────────────

    /// Build a [`LogFilter`] scoped to this contract's address.
    /// Pass to [`Provider::get_logs`] or
    /// [`crate::ws::WsProvider::subscribe_logs`].
    #[must_use]
    pub fn event_filter(&self) -> LogFilter {
        LogFilter {
            contracts: vec![self.address.to_hex()],
            ..Default::default()
        }
    }

    /// Build a [`LogFilter`] scoped to a single named event on this
    /// contract. The event signature topic is placed at position 0
    /// of the `topics` filter array; additional indexed-parameter
    /// constraints can be set by the caller after the filter is
    /// returned.
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidArgument`] for unknown event names.
    pub fn event_filter_for(&self, event_name: &str) -> Result<LogFilter, SdkError> {
        let ev = self
            .abi
            .event_by_name(event_name)
            .ok_or_else(|| SdkError::InvalidArgument(format!("event {event_name:?} not in ABI")))?;
        let topic0 = event_signature_topic(ev);
        Ok(LogFilter {
            contracts: vec![self.address.to_hex()],
            topics: vec![Some(vec![format!("0x{}", hex::encode(topic0))])],
            ..Default::default()
        })
    }

    /// Look up a function in the ABI, returning a clear error for
    /// unknown names + view-mode violations.
    fn lookup_function(&self, name: &str) -> Result<&FunctionAbi, SdkError> {
        let f = self
            .abi
            .function_by_name(name)
            .ok_or_else(|| SdkError::InvalidArgument(format!("function {name:?} not in ABI")))?;
        if f.attrs.is_constructor() && !f.attrs.has(FunctionAttrs::ENTRY) {
            return Err(SdkError::InvalidArgument(format!(
                "function {name:?} is the constructor — deploy-time only"
            )));
        }
        Ok(f)
    }
}

impl std::fmt::Debug for Contract {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Contract")
            .field("address", &self.address)
            .field("name", &self.abi.name)
            .field("functions", &self.abi.functions.len())
            .finish()
    }
}

// ── Event signature topic ──────────────────────────────────────

/// Compute the topic-0 hash for an event signature.
///
/// Signature shape: `"name(ty1,ty2,…)"` (parameter type names
/// joined by commas). Hash via Blake3 (Pyde's high-volume native
/// path per the dual-hash strategy).
#[must_use]
pub fn event_signature_topic(event: &EventAbi) -> [u8; 32] {
    let mut sig = String::new();
    sig.push_str(&event.name);
    sig.push('(');
    for (i, p) in event.params.iter().enumerate() {
        if i > 0 {
            sig.push(',');
        }
        write_canonical_type(&mut sig, &p.ty);
    }
    sig.push(')');
    *blake3::hash(sig.as_bytes()).as_bytes()
}

fn write_canonical_type(out: &mut String, ty: &crate::types::ParamType) {
    use crate::types::ParamType as P;
    match ty {
        P::U8 => out.push_str("u8"),
        P::U16 => out.push_str("u16"),
        P::U32 => out.push_str("u32"),
        P::U64 => out.push_str("u64"),
        P::U128 => out.push_str("u128"),
        P::I8 => out.push_str("i8"),
        P::I16 => out.push_str("i16"),
        P::I32 => out.push_str("i32"),
        P::I64 => out.push_str("i64"),
        P::I128 => out.push_str("i128"),
        P::Bool => out.push_str("bool"),
        P::Address => out.push_str("address"),
        P::Bytes => out.push_str("bytes"),
        P::String => out.push_str("string"),
        P::FixedBytes(n) => {
            out.push_str("bytes");
            out.push_str(&n.to_string());
        }
        P::Vec(inner) => {
            write_canonical_type(out, inner);
            out.push_str("[]");
        }
        P::Map { key, value } => {
            out.push_str("map<");
            write_canonical_type(out, key);
            out.push(',');
            write_canonical_type(out, value);
            out.push('>');
        }
        P::Option(inner) => {
            out.push_str("option<");
            write_canonical_type(out, inner);
            out.push('>');
        }
        P::Custom(name) => {
            // Custom types refer to a named struct/enum; the canonical
            // signature uses the name as-is.
            out.push_str(name);
        }
    }
}

// ── Decoded event ──────────────────────────────────────────────

/// Decoded event — typed name + (topic, data) split per ABI.
///
/// Returned by [`Contract::decode_event`]. Indexed parameters are
/// preserved in `topics` (one per indexed parameter, in declaration
/// order — *after* topic 0, the event signature). Non-indexed
/// parameters are Borsh-decoded from `data`.
#[derive(Debug, Clone)]
pub struct DecodedEvent {
    /// Event name as declared in the ABI.
    pub name: String,
    /// Indexed parameter values in declaration order. v1 stores
    /// each indexed value as a raw 32-byte topic — the caller's
    /// typed wrapper is responsible for interpreting the bytes
    /// (the SDK doesn't yet attempt to decode arbitrary types from
    /// 32-byte topic words).
    pub indexed: Vec<[u8; 32]>,
    /// Non-indexed parameter values, in declaration order, decoded
    /// from the event `data` payload.
    pub data: Vec<Value>,
}

impl Contract {
    /// Decode a raw [`Event`] log into a typed [`DecodedEvent`] by
    /// matching the topic-0 signature against this contract's ABI.
    ///
    /// # Errors
    /// - [`SdkError::InvalidResponse`] if the log's address doesn't
    ///   match this contract, if topic 0 is missing, if no event
    ///   signature matches, or if `data` doesn't decode.
    pub fn decode_event(&self, log: &Event) -> Result<DecodedEvent, SdkError> {
        let log_addr = Address::from_hex(&log.contract_addr)?;
        if log_addr != self.address {
            return Err(SdkError::InvalidResponse(format!(
                "event from {log_addr}, expected {}",
                self.address
            )));
        }
        let topic0_hex = log.topics.first().ok_or_else(|| {
            SdkError::InvalidResponse("event has no topic 0 — cannot decode".into())
        })?;
        let topic0_bytes = hex::decode(topic0_hex.trim_start_matches("0x"))
            .map_err(|e| SdkError::InvalidResponse(format!("topic 0 hex: {e}")))?;
        if topic0_bytes.len() != 32 {
            return Err(SdkError::InvalidResponse(format!(
                "topic 0 must be 32 bytes; got {}",
                topic0_bytes.len()
            )));
        }
        let mut topic0_arr = [0u8; 32];
        topic0_arr.copy_from_slice(&topic0_bytes);

        let ev_abi = self
            .abi
            .events
            .iter()
            .find(|e| event_signature_topic(e) == topic0_arr)
            .ok_or_else(|| {
                SdkError::InvalidResponse(format!("no event in ABI matches topic 0 {topic0_hex}"))
            })?;

        // Collect indexed-parameter topics (everything after topic 0).
        let mut indexed = Vec::with_capacity(ev_abi.topic_count() as usize);
        for t_hex in log.topics.iter().skip(1) {
            let bytes = hex::decode(t_hex.trim_start_matches("0x"))
                .map_err(|e| SdkError::InvalidResponse(format!("topic hex: {e}")))?;
            if bytes.len() != 32 {
                return Err(SdkError::InvalidResponse(format!(
                    "topic must be 32 bytes; got {}",
                    bytes.len()
                )));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            indexed.push(arr);
        }

        // Decode non-indexed parameters from `data`.
        let data_bytes = log.data_bytes();
        let mut slice = data_bytes.as_slice();
        let mut data_values = Vec::new();
        for (i, p) in ev_abi.params.iter().enumerate() {
            if ev_abi.is_indexed(i as u8) {
                continue;
            }
            let v = super::codec::decode_value_inplace(&p.ty, &mut slice)?;
            data_values.push(v);
        }
        if !slice.is_empty() {
            return Err(SdkError::InvalidResponse(format!(
                "trailing {} bytes after decoding event data",
                slice.len()
            )));
        }

        Ok(DecodedEvent {
            name: ev_abi.name.clone(),
            indexed,
            data: data_values,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::{
        ContractType, EventAbi, FunctionAbi, FunctionAttrs, ParamAbi, ParamType, StateSchema,
    };

    fn sample_abi() -> ContractAbi {
        ContractAbi {
            pyde_abi_version: ContractAbi::V1_2,
            contract_type: ContractType::Contract,
            functions: vec![
                FunctionAbi {
                    selector: [1, 2, 3, 4],
                    name: "get_balance".into(),
                    attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY | FunctionAttrs::VIEW),
                    params: vec![ParamAbi {
                        name: "of".into(),
                        ty: ParamType::Address,
                    }],
                    returns: Some(ParamType::U128),
                },
                FunctionAbi {
                    selector: [5, 6, 7, 8],
                    name: "transfer".into(),
                    attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY),
                    params: vec![
                        ParamAbi {
                            name: "to".into(),
                            ty: ParamType::Address,
                        },
                        ParamAbi {
                            name: "amount".into(),
                            ty: ParamType::U128,
                        },
                    ],
                    returns: None,
                },
            ],
            state_schema_hash: [0u8; 32],
            constructor_index: None,
            fallback_index: None,
            receive_index: None,
            name: "Token".into(),
            version: "0.1.0".into(),
            events: vec![EventAbi {
                name: "Transferred".into(),
                params: vec![
                    ParamAbi {
                        name: "from".into(),
                        ty: ParamType::Address,
                    },
                    ParamAbi {
                        name: "to".into(),
                        ty: ParamType::Address,
                    },
                    ParamAbi {
                        name: "amount".into(),
                        ty: ParamType::U128,
                    },
                ],
                indexed_mask: 0b011,
            }],
            parachain_imports: vec![],
            state_schema: StateSchema::empty(),
            types: vec![],
        }
    }

    #[test]
    fn event_signature_topic_is_stable() {
        let ev = EventAbi {
            name: "Transfer".into(),
            params: vec![
                ParamAbi {
                    name: "from".into(),
                    ty: ParamType::Address,
                },
                ParamAbi {
                    name: "to".into(),
                    ty: ParamType::Address,
                },
                ParamAbi {
                    name: "amount".into(),
                    ty: ParamType::U128,
                },
            ],
            indexed_mask: 0b011,
        };
        let topic = event_signature_topic(&ev);
        let expected = blake3::hash(b"Transfer(address,address,u128)");
        assert_eq!(topic, *expected.as_bytes());
    }

    #[test]
    fn function_lookup_rejects_unknown() {
        // We don't need a live provider for this — build directly.
        let abi = sample_abi();
        let bad = abi.function_by_name("nope");
        assert!(bad.is_none());
        let good = abi.function_by_name("transfer");
        assert!(good.is_some());
    }

    #[test]
    fn canonical_event_signature_format() {
        let mut s = String::new();
        write_canonical_type(&mut s, &ParamType::Vec(Box::new(ParamType::U64)));
        assert_eq!(s, "u64[]");
        let mut s = String::new();
        write_canonical_type(
            &mut s,
            &ParamType::Map {
                key: Box::new(ParamType::String),
                value: Box::new(ParamType::U128),
            },
        );
        assert_eq!(s, "map<string,u128>");
        let mut s = String::new();
        write_canonical_type(&mut s, &ParamType::FixedBytes(8));
        assert_eq!(s, "bytes8");
        let mut s = String::new();
        write_canonical_type(&mut s, &ParamType::Option(Box::new(ParamType::Bool)));
        assert_eq!(s, "option<bool>");
    }
}
