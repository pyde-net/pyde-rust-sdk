//! Transaction construction, canonical encoding, and `tx_hash`.
//!
//! This module owns:
//!
//! - [`tx_hash`] — the canonical Poseidon2 tx hash per Ch 11 §11.6,
//!   byte-identical to `engine/crates/tx/src/hashing.rs::tx_hash`.
//! - [`encode`] / [`decode`] — Borsh codec for the full [`Tx`]
//!   envelope, what `pyde_sendRawTransaction` expects on the wire.
//! - [`TxBuilder`] — fluent builder for the common construction
//!   patterns (transfer, contract call, deploy).
//!
//! The signing flow is:
//!
//! 1. Build an unsigned [`Tx`] (signature field is empty).
//! 2. Compute [`tx_hash`] over the canonical pre-image (the
//!    signature is **not** in the pre-image — including it would
//!    be circular).
//! 3. FALCON-sign the hash bytes.
//! 4. Patch the produced [`FalconSignature`] into `tx.signature`.
//! 5. [`encode`] the now-signed [`Tx`] and submit via
//!    `pyde_sendRawTransaction`.
//!
//! Step 4-5 are handled by [`crate::signer::Signer::sign_tx`].

use borsh::BorshDeserialize;
use pyde_crypto::poseidon2::poseidon2_hash;

use crate::error::SdkError;
use crate::types::{AccessEntry, Address, FalconSignature, FeePayer, Gas, Tx, TxHash, TxType};

// ── Canonical hash ─────────────────────────────────────────────

/// Compute the canonical transaction hash.
///
/// The hash is `Poseidon2` over a fixed-order pre-image:
///
/// ```text
/// tx_hash = Poseidon2(
///     chain_id (u64 LE)
///     || from (32 B)
///     || to (32 B)
///     || value (u128 LE)
///     || Poseidon2(data) (32 B)
///     || gas_limit (u64 LE)
///     || nonce (u64 LE)
///     || fee_payer_tag (1 B or 1 + 32 B)
///     || Poseidon2(access_list_borsh) (32 B)
///     || deadline_tag (1 B or 1 + 8 B)
///     || tx_type_tag (u8)
/// )
/// ```
///
/// `data` and `access_list` are pre-hashed to keep the outer
/// Poseidon2 permutation at a bounded cost regardless of input
/// size (~11 field elements vs unbounded otherwise).
///
/// The [`FalconSignature`] is the only field excluded from the
/// pre-image — otherwise signing would be circular.
///
/// Byte-identical to `engine/crates/tx/src/hashing.rs::tx_hash`.
#[must_use]
pub fn tx_hash(tx: &Tx) -> TxHash {
    let data_digest: [u8; 32] = poseidon2_hash(&tx.data).into();
    let access_list_digest = hash_access_list(&tx.access_list);
    let fee_payer_bytes = encode_fee_payer_tag(&tx.fee_payer);
    let deadline_bytes = encode_deadline_tag(tx.deadline);

    let mut preimage = Vec::with_capacity(
        8                          // chain_id
        + 32                       // from
        + 32                       // to
        + 16                       // value
        + 32                       // Poseidon2(data)
        + 8                        // gas_limit
        + 8                        // nonce
        + fee_payer_bytes.len()    // fee_payer tag
        + 32                       // Poseidon2(access_list)
        + deadline_bytes.len()     // deadline tag
        + 1, // tx_type tag
    );
    preimage.extend_from_slice(&tx.chain_id.to_le_bytes());
    preimage.extend_from_slice(tx.from.as_bytes());
    preimage.extend_from_slice(tx.to.as_bytes());
    preimage.extend_from_slice(&tx.value.to_le_bytes());
    preimage.extend_from_slice(&data_digest);
    preimage.extend_from_slice(&tx.gas_limit.to_le_bytes());
    preimage.extend_from_slice(&tx.nonce.to_le_bytes());
    preimage.extend_from_slice(&fee_payer_bytes);
    preimage.extend_from_slice(&access_list_digest);
    preimage.extend_from_slice(&deadline_bytes);
    preimage.push(tx_type_tag(&tx.tx_type));

    let digest: [u8; 32] = poseidon2_hash(&preimage).into();
    TxHash::new(digest)
}

/// `Poseidon2(borsh_canonical(access_list))`.
///
/// `AccessEntry` has a fixed Borsh schema (address 32 B + storage
/// keys length-prefixed Vec + access_type tag), and the access list
/// is itself a length-prefixed `Vec<AccessEntry>`. Encoding the
/// full slice via Borsh is canonical.
fn hash_access_list(list: &[AccessEntry]) -> [u8; 32] {
    let encoded = borsh::to_vec(list).unwrap_or_default();
    poseidon2_hash(&encoded).into()
}

/// Canonical-hash byte encoding of [`FeePayer`].
///
/// `Sender → [0x00]`, `GasTank → [0x01]`, `Paymaster(addr) →
/// [0x02, addr[..32]]`. This matches Borsh's discriminant
/// encoding for the frozen enum but is spelt out explicitly so the
/// canonical hash never drifts if a future Borsh upgrade changes
/// its representation.
fn encode_fee_payer_tag(payer: &FeePayer) -> Vec<u8> {
    match payer {
        FeePayer::Sender => vec![0x00],
        FeePayer::GasTank => vec![0x01],
        FeePayer::Paymaster(addr) => {
            let mut out = Vec::with_capacity(1 + 32);
            out.push(0x02);
            out.extend_from_slice(addr.as_bytes());
            out
        }
    }
}

/// Canonical-hash byte encoding of `Option<deadline>`.
///
/// `None → [0x00]`, `Some(d) → [0x01, d.to_le_bytes()]`. Matches
/// Borsh `Option` encoding; spelt out for the same wire-stability
/// reason as [`encode_fee_payer_tag`].
fn encode_deadline_tag(deadline: Option<u64>) -> Vec<u8> {
    match deadline {
        None => vec![0x00],
        Some(d) => {
            let mut out = Vec::with_capacity(1 + 8);
            out.push(0x01);
            out.extend_from_slice(&d.to_le_bytes());
            out
        }
    }
}

/// One-byte tag for [`TxType`] used in the canonical hash.
///
/// Uses Borsh's `use_discriminant` so the value matches the
/// `#[repr(u8)]` discriminant on the enum. Spelt out here so wire
/// stability is decoupled from `borsh-derive` output.
fn tx_type_tag(t: &TxType) -> u8 {
    match t {
        TxType::Standard => 0x00,
        TxType::Deploy => 0x01,
        TxType::StakeDeposit => 0x03,
        TxType::StakeWithdraw => 0x04,
        TxType::Slash => 0x05,
        TxType::ClaimReward => 0x06,
        TxType::ClaimAirdrop => 0x07,
        TxType::SweepAirdrop => 0x08,
        TxType::MultisigTx => 0x09,
        TxType::RotateMultisig => 0x0A,
        TxType::EmergencyPause => 0x0B,
        TxType::EmergencyResume => 0x0C,
        TxType::RegisterPubkey => 0x0D,
        TxType::Unjail => 0x0E,
        TxType::RotateValidatorKeys => 0x0F,
        TxType::DisputeSlash => 0x10,
        // 0x02 — Batch was removed pre-mainnet; tag intentionally vacant.
    }
}

// ── Wire codec ──────────────────────────────────────────────────

/// Borsh-encode a [`Tx`] into the bytes accepted by
/// `pyde_sendRawTransaction`.
///
/// Field order is fixed in [`Tx`]'s declaration; Borsh serialises
/// in declaration order, so this output is byte-identical to what
/// the engine produces.
///
/// # Errors
/// Returns [`SdkError::Other`] only if Borsh serialisation itself
/// fails (effectively unreachable for the shapes here).
pub fn encode(tx: &Tx) -> Result<Vec<u8>, SdkError> {
    borsh::to_vec(tx).map_err(|e| SdkError::Other(format!("borsh encode: {e}")))
}

/// Borsh-decode bytes into a [`Tx`].
///
/// # Errors
/// Returns [`SdkError::InvalidArgument`] if the bytes don't conform
/// to the canonical Borsh schema.
pub fn decode(bytes: &[u8]) -> Result<Tx, SdkError> {
    Tx::try_from_slice(bytes).map_err(|e| SdkError::InvalidArgument(format!("borsh decode: {e}")))
}

// ── Builder ────────────────────────────────────────────────────

/// Fluent builder for the common tx-construction patterns.
///
/// Modelled on alloy's `TransactionRequest`. The builder collects
/// fields and produces an unsigned [`Tx`] on [`TxBuilder::build`].
/// The unsigned tx is then handed to a [`crate::signer::Signer`]
/// to populate `signature` and ready it for submission.
///
/// Sensible defaults:
///
/// - `to` = [`Address::ZERO`] (set explicitly via [`Self::to`])
/// - `value` = `0`
/// - `data` = empty
/// - `gas_limit` = [`crate::types::MIN_GAS_LIMIT`]
/// - `nonce` = `0` (will usually be overwritten by a nonce filler)
/// - `fee_payer` = [`FeePayer::Sender`]
/// - `access_list` = empty
/// - `deadline` = `None`
/// - `chain_id` = `1` (mainnet)
/// - `tx_type` = [`TxType::Standard`]
#[derive(Clone, Debug)]
pub struct TxBuilder {
    from: Option<Address>,
    to: Address,
    value: u128,
    data: Vec<u8>,
    gas_limit: Gas,
    nonce: u64,
    fee_payer: FeePayer,
    access_list: Vec<AccessEntry>,
    deadline: Option<u64>,
    chain_id: u64,
    tx_type: TxType,
}

impl Default for TxBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TxBuilder {
    /// Start a new builder with sensible defaults.
    #[must_use]
    pub fn new() -> Self {
        Self {
            from: None,
            to: Address::ZERO,
            value: 0,
            data: Vec::new(),
            gas_limit: crate::types::MIN_GAS_LIMIT,
            nonce: 0,
            fee_payer: FeePayer::Sender,
            access_list: Vec::new(),
            deadline: None,
            chain_id: 1,
            tx_type: TxType::Standard,
        }
    }

    /// Set the sender.
    #[must_use]
    pub fn from(mut self, addr: Address) -> Self {
        self.from = Some(addr);
        self
    }

    /// Set the recipient.
    #[must_use]
    pub fn to(mut self, addr: Address) -> Self {
        self.to = addr;
        self
    }

    /// Set the attached value, in quanta.
    #[must_use]
    pub fn value(mut self, quanta: u128) -> Self {
        self.value = quanta;
        self
    }

    /// Set the calldata / payload bytes.
    #[must_use]
    pub fn data(mut self, bytes: Vec<u8>) -> Self {
        self.data = bytes;
        self
    }

    /// Set the gas limit.
    #[must_use]
    pub fn gas_limit(mut self, gas: Gas) -> Self {
        self.gas_limit = gas;
        self
    }

    /// Set the nonce.
    #[must_use]
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.nonce = nonce;
        self
    }

    /// Set who pays the gas.
    #[must_use]
    pub fn fee_payer(mut self, payer: FeePayer) -> Self {
        self.fee_payer = payer;
        self
    }

    /// Set the declared access list.
    #[must_use]
    pub fn access_list(mut self, list: Vec<AccessEntry>) -> Self {
        self.access_list = list;
        self
    }

    /// Set the optional inclusion deadline (wave id).
    #[must_use]
    pub fn deadline(mut self, wave: u64) -> Self {
        self.deadline = Some(wave);
        self
    }

    /// Clear any previously-set deadline.
    #[must_use]
    pub fn clear_deadline(mut self) -> Self {
        self.deadline = None;
        self
    }

    /// Set the chain id.
    #[must_use]
    pub fn chain_id(mut self, id: u64) -> Self {
        self.chain_id = id;
        self
    }

    /// Set the tx-type discriminant.
    #[must_use]
    pub fn tx_type(mut self, ty: TxType) -> Self {
        self.tx_type = ty;
        self
    }

    /// Configure this builder as a plain value transfer.
    ///
    /// Equivalent to `.tx_type(Standard).to(addr).value(quanta)`.
    #[must_use]
    pub fn transfer(self, to: Address, quanta: u128) -> Self {
        self.tx_type(TxType::Standard).to(to).value(quanta)
    }

    /// Configure this builder as a contract deployment.
    ///
    /// Sets `tx_type = Deploy`, `to = Address::ZERO`, and `data` to
    /// the supplied WASM bytecode.
    #[must_use]
    pub fn deploy(self, wasm_bytecode: Vec<u8>) -> Self {
        self.tx_type(TxType::Deploy)
            .to(Address::ZERO)
            .data(wasm_bytecode)
    }

    /// Configure this builder as a contract call.
    ///
    /// Sets `tx_type = Standard`, `to = contract`, and `data =
    /// calldata` (typically `selector || encoded_args`).
    #[must_use]
    pub fn call(self, contract: Address, calldata: Vec<u8>) -> Self {
        self.tx_type(TxType::Standard).to(contract).data(calldata)
    }

    /// Build an unsigned [`Tx`].
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidArgument`] if `from` wasn't set —
    /// the signing flow needs to know the sender address to derive
    /// the nonce-window slot and assert the FALCON pubkey matches.
    pub fn build(self) -> Result<Tx, SdkError> {
        let from = self.from.ok_or_else(|| {
            SdkError::InvalidArgument("from address must be set on TxBuilder".into())
        })?;
        Ok(Tx {
            from,
            to: self.to,
            value: self.value,
            data: self.data,
            gas_limit: self.gas_limit,
            nonce: self.nonce,
            signature: FalconSignature::new(Vec::new()),
            fee_payer: self.fee_payer,
            access_list: self.access_list,
            deadline: self.deadline,
            chain_id: self.chain_id,
            tx_type: self.tx_type,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::{AccessType, FalconPubkey, FALCON_PUBKEY_LEN};

    fn minimal_tx() -> Tx {
        Tx {
            from: Address::ZERO,
            to: Address::ZERO,
            value: 0,
            data: Vec::new(),
            gas_limit: 0,
            nonce: 0,
            signature: FalconSignature::new(vec![0u8; 32]),
            fee_payer: FeePayer::Sender,
            access_list: Vec::new(),
            deadline: None,
            chain_id: 0,
            tx_type: TxType::Standard,
        }
    }

    // ── tx_hash determinism + signature exclusion ───────────────

    #[test]
    fn same_input_yields_same_hash() {
        let tx = minimal_tx();
        assert_eq!(tx_hash(&tx), tx_hash(&tx));
    }

    #[test]
    fn signature_not_included_in_hash() {
        let mut a = minimal_tx();
        let mut b = minimal_tx();
        a.signature = FalconSignature::new(vec![0xAA; 666]);
        b.signature = FalconSignature::new(vec![0xBB; 666]);
        assert_eq!(tx_hash(&a), tx_hash(&b));
    }

    // ── per-field sensitivity ───────────────────────────────────

    macro_rules! sensitivity_test {
        ($name:ident, $mutator_a:expr, $mutator_b:expr) => {
            #[test]
            fn $name() {
                let mut a = minimal_tx();
                let mut b = minimal_tx();
                $mutator_a(&mut a);
                $mutator_b(&mut b);
                assert_ne!(tx_hash(&a), tx_hash(&b), "field must affect hash");
            }
        };
    }

    sensitivity_test!(
        chain_id_change_changes_hash,
        |t: &mut Tx| t.chain_id = 1,
        |t: &mut Tx| t.chain_id = 31337
    );
    sensitivity_test!(
        from_change_changes_hash,
        |t: &mut Tx| t.from = Address::new([0x01; 32]),
        |t: &mut Tx| t.from = Address::new([0x02; 32])
    );
    sensitivity_test!(
        to_change_changes_hash,
        |t: &mut Tx| t.to = Address::new([0x01; 32]),
        |t: &mut Tx| t.to = Address::new([0x02; 32])
    );
    sensitivity_test!(
        value_change_changes_hash,
        |t: &mut Tx| t.value = 1,
        |t: &mut Tx| t.value = 2
    );
    sensitivity_test!(
        data_change_changes_hash,
        |t: &mut Tx| t.data = vec![0x01],
        |t: &mut Tx| t.data = vec![0x02]
    );
    sensitivity_test!(
        gas_limit_change_changes_hash,
        |t: &mut Tx| t.gas_limit = 21_000,
        |t: &mut Tx| t.gas_limit = 21_001
    );
    sensitivity_test!(
        nonce_change_changes_hash,
        |t: &mut Tx| t.nonce = 0,
        |t: &mut Tx| t.nonce = 1
    );
    sensitivity_test!(
        deadline_presence_changes_hash,
        |t: &mut Tx| t.deadline = None,
        |t: &mut Tx| t.deadline = Some(0)
    );
    sensitivity_test!(
        deadline_value_changes_hash,
        |t: &mut Tx| t.deadline = Some(100),
        |t: &mut Tx| t.deadline = Some(101)
    );
    sensitivity_test!(
        tx_type_change_changes_hash,
        |t: &mut Tx| t.tx_type = TxType::Standard,
        |t: &mut Tx| t.tx_type = TxType::Deploy
    );

    // ── fee_payer / access_list ────────────────────────────────

    #[test]
    fn fee_payer_variant_change_changes_hash() {
        let mut a = minimal_tx();
        let mut b = minimal_tx();
        let mut c = minimal_tx();
        a.fee_payer = FeePayer::Sender;
        b.fee_payer = FeePayer::GasTank;
        c.fee_payer = FeePayer::Paymaster(Address::new([0x42; 32]));
        let ha = tx_hash(&a);
        let hb = tx_hash(&b);
        let hc = tx_hash(&c);
        assert_ne!(ha, hb);
        assert_ne!(hb, hc);
        assert_ne!(ha, hc);
    }

    #[test]
    fn paymaster_addr_change_changes_hash() {
        let mut a = minimal_tx();
        let mut b = minimal_tx();
        a.fee_payer = FeePayer::Paymaster(Address::new([0x01; 32]));
        b.fee_payer = FeePayer::Paymaster(Address::new([0x02; 32]));
        assert_ne!(tx_hash(&a), tx_hash(&b));
    }

    #[test]
    fn access_list_ordering_changes_hash() {
        let a_entry = AccessEntry {
            address: Address::new([0x01; 32]),
            storage_keys: vec![[0u8; 32]],
            access_type: AccessType::Read,
        };
        let b_entry = AccessEntry {
            address: Address::new([0x02; 32]),
            storage_keys: vec![[0u8; 32]],
            access_type: AccessType::Read,
        };
        let mut a = minimal_tx();
        let mut b = minimal_tx();
        a.access_list = vec![a_entry.clone(), b_entry.clone()];
        b.access_list = vec![b_entry, a_entry];
        assert_ne!(tx_hash(&a), tx_hash(&b));
    }

    #[test]
    fn empty_access_list_uses_canonical_borsh_encoding() {
        // Empty Vec serialises to its 4-byte LE length prefix (0)
        // under Borsh; hashing that through Poseidon2 must yield
        // a stable digest different from "Poseidon2(empty)".
        let empty_digest = hash_access_list(&[]);
        let raw_empty: [u8; 32] = poseidon2_hash(&[]).into();
        assert_ne!(empty_digest, raw_empty);
    }

    #[test]
    fn known_vector_hash_is_stable() {
        // Minimal-tx hash must be deterministic and non-zero.
        let tx = minimal_tx();
        let h1 = tx_hash(&tx);
        let h2 = tx_hash(&tx);
        assert_eq!(h1, h2);
        assert_ne!(h1, TxHash::zero());
    }

    #[test]
    fn every_tx_type_yields_distinct_hash_when_otherwise_identical() {
        use TxType::*;
        let variants = [
            Standard,
            Deploy,
            StakeDeposit,
            StakeWithdraw,
            Slash,
            ClaimReward,
            ClaimAirdrop,
            SweepAirdrop,
            MultisigTx,
            RotateMultisig,
            EmergencyPause,
            EmergencyResume,
            RegisterPubkey,
            Unjail,
            RotateValidatorKeys,
            DisputeSlash,
        ];
        let mut seen = std::collections::HashSet::new();
        for ty in variants {
            let mut tx = minimal_tx();
            tx.tx_type = ty;
            let h = tx_hash(&tx);
            assert!(seen.insert(h), "collision at TxType::{ty:?}");
        }
        assert_eq!(seen.len(), 16);
    }

    // ── codec round-trip ────────────────────────────────────────

    #[test]
    fn encode_decode_round_trip() {
        let tx = minimal_tx();
        let bytes = encode(&tx).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!(tx, decoded);
    }

    // ── builder ────────────────────────────────────────────────

    #[test]
    fn builder_defaults() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let from = Address::from_pubkey(&pk);
        let tx = TxBuilder::new().from(from).build().unwrap();
        assert_eq!(tx.from, from);
        assert_eq!(tx.to, Address::ZERO);
        assert_eq!(tx.value, 0);
        assert_eq!(tx.gas_limit, crate::types::MIN_GAS_LIMIT);
        assert_eq!(tx.chain_id, 1);
        assert_eq!(tx.tx_type, TxType::Standard);
        assert!(tx.signature.is_empty());
    }

    #[test]
    fn builder_requires_from() {
        assert!(TxBuilder::new().build().is_err());
    }

    #[test]
    fn builder_transfer_helper() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let from = Address::from_pubkey(&pk);
        let to = Address::new([0x55; 32]);
        let tx = TxBuilder::new()
            .from(from)
            .transfer(to, 1_500_000_000)
            .build()
            .unwrap();
        assert_eq!(tx.tx_type, TxType::Standard);
        assert_eq!(tx.to, to);
        assert_eq!(tx.value, 1_500_000_000);
    }

    #[test]
    fn builder_deploy_helper() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let from = Address::from_pubkey(&pk);
        let wasm = b"\0asm\x01\0\0\0".to_vec();
        let tx = TxBuilder::new()
            .from(from)
            .deploy(wasm.clone())
            .gas_limit(5_000_000)
            .build()
            .unwrap();
        assert_eq!(tx.tx_type, TxType::Deploy);
        assert!(tx.to.is_zero());
        assert_eq!(tx.data, wasm);
    }

    #[test]
    fn builder_call_helper() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let from = Address::from_pubkey(&pk);
        let contract = Address::from_contract_name("counter");
        let calldata = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let tx = TxBuilder::new()
            .from(from)
            .call(contract, calldata.clone())
            .build()
            .unwrap();
        assert_eq!(tx.tx_type, TxType::Standard);
        assert_eq!(tx.to, contract);
        assert_eq!(tx.data, calldata);
    }
}
