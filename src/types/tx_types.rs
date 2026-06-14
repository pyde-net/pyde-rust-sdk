//! Transaction wire types — [`Tx`], [`TxType`], [`FeePayer`],
//! [`AccessEntry`], [`AccessType`].
//!
//! Mirrors `engine/crates/types/src/tx.rs` byte-for-byte. The wire
//! field order is **load-bearing**: Borsh serialises in declaration
//! order, and the engine's canonical hash spec assumes the order
//! laid out in [Chapter 11 §11.6][spec]. Reordering or retagging
//! any of these types is a chain-wide hard fork.
//!
//! [spec]: https://book.pyde.network/chapters/11-account-model#116-transaction-wire-format

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::{Address, FalconSignature};

/// Gas limit type — `u64`, matches Ch 10 accounting.
pub type Gas = u64;

/// Actual gas consumed by a transaction. Same units as [`Gas`].
pub type GasUsed = u64;

/// Per-wave fee in quanta. `u128` accommodates compounding across
/// waves without overflow.
pub type FeeQuanta = u128;

/// Minimum gas accepted by ingress validation
/// (Ch 10 §10.9 — matches the cost of a plain `Transfer` handler).
pub const MIN_GAS_LIMIT: Gas = 21_000;

/// Maximum total tx size (Ch 10 §10.9). 128 KiB.
pub const MAX_TX_SIZE: usize = 128 * 1024;

/// Maximum calldata size (Ch 10 §10.9). 64 KiB — separate cap from
/// [`MAX_TX_SIZE`] so a tx can't burn the entire 128 KiB on
/// calldata and starve the access-list / envelope.
pub const MAX_CALLDATA: usize = 64 * 1024;

/// Discriminant for the 16 active + 1 reserved-vacant tx variants
/// in Ch 11 §11.8.
///
/// Tag values are **wire-load-bearing** — never reassign post-mainnet.
/// Tag `0x02` is intentionally vacant (`Batch` was prototyped pre-
/// mainnet and removed); keeping the gap means a forged
/// `tx_type = 2` fails decode rather than silently aliasing.
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum TxType {
    /// `0x00` — Value transfer or contract call. `to/value/data`
    /// carry the call target, value, and calldata.
    Standard = 0x00,
    /// `0x01` — Contract deployment. `to == Address::ZERO`; `data`
    /// holds the Borsh-encoded [`super::DeployData`] envelope
    /// (name, WASM bytes, contract type, init calldata).
    Deploy = 0x01,
    // 0x02 — reserved-as-vacant (Batch removed pre-mainnet).
    /// `0x03` — Lock `>= MIN_VALIDATOR_STAKE` (10,000 PYDE) and
    /// register as validator. `data` holds the 897-byte FALCON
    /// pubkey.
    StakeDeposit = 0x03,
    /// `0x04` — Begin 30-day unbonding.
    StakeWithdraw = 0x04,
    /// `0x05` — Submit double-sign / equivocation evidence.
    /// `data` holds the serialised evidence per `SLASHING.md`.
    Slash = 0x05,
    /// `0x06` — Claim accrued staking yield.
    ClaimReward = 0x06,
    /// `0x07` — Claim a genesis airdrop with a Merkle proof.
    /// `data` holds the proof per Ch 14.
    ClaimAirdrop = 0x07,
    /// `0x08` — Sweep unclaimed airdrop residue to treasury after
    /// the post-genesis deadline.
    SweepAirdrop = 0x08,
    /// `0x09` — Treasury spend with multisig sigs. `data` holds
    /// the (target ‖ amount ‖ multisig_sigs) envelope per Ch 15.
    MultisigTx = 0x09,
    /// `0x0A` — Rotate the treasury multisig signer set + threshold.
    /// `data` holds (new_signers ‖ new_threshold ‖ sigs).
    RotateMultisig = 0x0A,
    /// `0x0B` — Halt wave production. Multisig-signed; `data`
    /// holds sigs.
    EmergencyPause = 0x0B,
    /// `0x0C` — Resume normal processing. Multisig-signed; `data`
    /// holds sigs.
    EmergencyResume = 0x0C,
    /// `0x0D` — First-time pubkey registration for a funded-but-
    /// unregistered account. Allowed only when `balance > 0` and
    /// `auth_keys == None`; proof of pubkey ownership is the
    /// address-derivation check.
    RegisterPubkey = 0x0D,
    /// `0x0E` — Release a validator from `Jailed` back to `Active`.
    Unjail = 0x0E,
    /// `0x0F` — Rotate the FALCON-512 signing key on an already-
    /// registered validator. Signed by the OLD key; on success the
    /// handler swaps both the validator record and the account's
    /// `auth_keys` to the new key. Active validators only.
    RotateValidatorKeys = 0x0F,
    /// `0x10` — Governance dispute over a pending slash. Treasury
    /// multisig gates submission; `data` is
    /// `borsh(DisputeSlashPayload)`.
    DisputeSlash = 0x10,
}

/// Who pays the gas for a transaction.
///
/// Tag values per Ch 11 §11.6 and Ch 10 §10.7:
///
/// | Tag        | Variant                       |
/// |------------|-------------------------------|
/// | `0x00`     | [`FeePayer::Sender`]          |
/// | `0x01`     | [`FeePayer::GasTank`]         |
/// | `0x02`+addr | [`FeePayer::Paymaster`]       |
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum FeePayer {
    /// Sender's own balance pays.
    Sender = 0x00,
    /// The target contract's `gas_tank` pays.
    GasTank = 0x01,
    /// A named paymaster account pays. Subject to that paymaster's
    /// `validate_sponsorship` returning true within 100,000 gas.
    Paymaster(Address) = 0x02,
}

/// Access intent for a single `(address, slot)` entry. Drives the
/// parallel-execution scheduler (Ch 9).
#[derive(
    Clone,
    Copy,
    Debug,
    Eq,
    PartialEq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum AccessType {
    /// Slot is only read; multiple `Read` entries for the same slot
    /// can run in parallel.
    Read = 0x00,
    /// Slot may be written; conflicts with any other entry on the
    /// same slot.
    ReadWrite = 0x01,
}

/// One entry in the transaction's declared access list.
///
/// Built by `pyde_createAccessList` from a simulated execution.
/// Slots touched at execution time that aren't in the list revert
/// the tx with `ERR_ACCESS_LIST_VIOLATION`.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct AccessEntry {
    /// Account whose state is touched.
    pub address: Address,
    /// Storage slot keys touched within `address` (each is a 32-byte
    /// PIP-2 clustered key).
    pub storage_keys: Vec<[u8; 32]>,
    /// Whether these slots are read-only or read-write.
    pub access_type: AccessType,
}

/// A Pyde transaction — the wire-frozen envelope.
///
/// Field order matches Ch 11 §11.6 verbatim. **Do not reorder** —
/// Borsh serialises in declaration order and the engine's canonical
/// hash function assumes this layout.
///
/// The meaning of [`Tx::data`] depends on [`Tx::tx_type`]:
///
/// | tx_type              | `data` interpretation                                |
/// |----------------------|------------------------------------------------------|
/// | `Standard`           | calldata for the function being called               |
/// | `Deploy`             | Borsh-encoded [`DeployData`] envelope                |
/// | `StakeDeposit`       | 897-byte FALCON validator pubkey                     |
/// | `RotateValidatorKeys`| the new 897-byte FALCON pubkey                       |
/// | `Slash`              | serialised evidence per `SLASHING.md`                |
/// | `ClaimAirdrop`       | Merkle proof bytes per Ch 14                         |
/// | `MultisigTx`         | `(target ‖ amount ‖ sigs)` envelope                  |
/// | `RotateMultisig`     | `(new_signers ‖ new_threshold ‖ sigs)` envelope      |
/// | `EmergencyPause/Resume` | multisig signature bundle                         |
/// | `RegisterPubkey`     | the FALCON pubkey to register                        |
/// | `StakeWithdraw/ClaimReward/SweepAirdrop` | empty                            |
/// | `Unjail`             | empty                                                |
/// | `DisputeSlash`       | `borsh(DisputeSlashPayload)`                         |
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct Tx {
    /// Sender address.
    pub from: Address,
    /// Recipient. [`Address::ZERO`] for `Deploy` and envelope-style
    /// txs (Multisig*, Emergency*, Stake*, …).
    pub to: Address,
    /// Value attached to the call, in quanta.
    pub value: u128,
    /// Variant-specific payload — see the `data` interpretation
    /// table in the struct-level docs.
    pub data: Vec<u8>,
    /// Maximum gas the sender authorises.
    pub gas_limit: Gas,
    /// Sender's nonce. Must be `[base, base + 16)` per the 16-slot
    /// bitmap window (Ch 11 §11.4).
    pub nonce: u64,
    /// FALCON-512 signature over the tx canonical pre-image. The
    /// signature itself is NOT hashed (would be circular).
    pub signature: FalconSignature,
    /// Who pays the gas.
    pub fee_payer: FeePayer,
    /// Declared access list. Empty means "no conflicts declared" —
    /// the scheduler serialises this tx; declaring slots unlocks
    /// parallel execution.
    pub access_list: Vec<AccessEntry>,
    /// Optional inclusion deadline (wave id). After this wave, the
    /// tx is dropped from mempools and its nonce slot frees up.
    pub deadline: Option<u64>,
    /// Chain identifier — replay protection across chains. `1` =
    /// mainnet, `31337` = devnet (per spec; genesis-configured).
    pub chain_id: u64,
    /// Variant discriminant — see [`TxType`].
    pub tx_type: TxType,
}

impl Tx {
    /// Construct a transaction with an empty signature. Useful as a
    /// pre-signing scaffold — call [`crate::tx::tx_hash`] on the
    /// scaffolded tx, sign the hash, then mutate `signature` to the
    /// produced bytes.
    ///
    /// The signature is omitted from the canonical hash pre-image
    /// (otherwise signing would be circular), so signing-and-back-
    /// patching is the canonical flow.
    //
    // Many parameters by design — Tx is a wire-frozen envelope and
    // every field must be named explicitly. Prefer the
    // [`crate::tx::TxBuilder`] fluent API for the typical
    // construction path.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn unsigned(
        from: Address,
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
    ) -> Self {
        Self {
            from,
            to,
            value,
            data,
            gas_limit,
            nonce,
            signature: FalconSignature::new(Vec::new()),
            fee_payer,
            access_list,
            deadline,
            chain_id,
            tx_type,
        }
    }
}

/// Borsh-encoded payload carried in `Tx::data` when
/// `tx_type == TxType::Standard` AND `tx.to` resolves to a
/// contract account (one with a non-zero `code_hash`).
///
/// Pyde dispatches contract calls by **function name** rather than
/// by 4-byte selector — the chain parses the `pyde.abi` custom
/// section at deploy time, so `name → entry point` lookup is a free
/// O(1) ABI walk. Wallets don't need to compute selectors; they
/// just send the function name + raw argument bytes.
///
/// All fields are wire-load-bearing — adding a field is a hard
/// fork of the call envelope.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct CallPayload {
    /// Name of the contract function to invoke. Looked up in the
    /// deployed contract's `ContractAbi.functions` by exact match.
    /// Missing names revert with `ERR_INVALID_FUNCTION_NAME`;
    /// constructor-attributed functions are locked out (deploy-time
    /// only).
    pub function: String,
    /// Opaque function-argument bytes. The contract's WASM reads
    /// them via `pyde::calldata_size` + `pyde::calldata_copy`;
    /// encoding is contract-defined.
    pub calldata: Vec<u8>,
}

/// Borsh-encoded payload carried in `Tx::data` when
/// `tx_type == TxType::Deploy`.
///
/// Mirrors `engine/crates/types/src/deploy.rs::DeployData`
/// byte-for-byte. The deployed contract's address is derived from
/// `name` via [`crate::types::Address::from_contract_name`] —
/// `Poseidon2("pyde-contract:" || name)` — so the name is the
/// address-derivation input. The on-chain name registry rejects
/// duplicate names at deploy time.
///
/// All fields are wire-load-bearing — adding a field is a hard
/// fork of the deploy envelope.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct DeployData {
    /// ENS-style contract name. Registered in the on-chain name
    /// registry; doubles as the address-derivation input
    /// (`Poseidon2("pyde-contract:" || name)`).
    pub name: String,
    /// The contract's WASM binary with the `pyde.abi` custom
    /// section embedded per HOST_FN_ABI §3.7.
    pub wasm_bytes: Vec<u8>,
    /// Contract-vs-parachain discriminant. Drives the host-fn
    /// import allowlist (contracts get §7 only; parachains
    /// additionally get §8).
    pub contract_type: super::abi::ContractType,
    /// Calldata passed to the constructor. Empty if the contract
    /// has no constructor or the constructor takes no arguments.
    pub init_calldata: Vec<u8>,
}

// Internal test-only accessor; placed before the `#[cfg(test)] mod
// tests` block to satisfy clippy's items-after-test-module rule.
impl FeePayer {
    #[cfg(test)]
    #[allow(dead_code)]
    fn discriminant_tag(&self) -> u8 {
        match self {
            FeePayer::Sender => 0x00,
            FeePayer::GasTank => 0x01,
            FeePayer::Paymaster(_) => 0x02,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::FalconPubkey;

    fn minimal_tx() -> Tx {
        Tx::unsigned(
            Address::ZERO,
            Address::ZERO,
            0,
            vec![],
            21_000,
            0,
            FeePayer::Sender,
            vec![],
            None,
            1,
            TxType::Standard,
        )
    }

    #[test]
    fn tx_type_tags_match_spec() {
        assert_eq!(TxType::Standard as u8, 0x00);
        assert_eq!(TxType::Deploy as u8, 0x01);
        assert_eq!(TxType::StakeDeposit as u8, 0x03);
        assert_eq!(TxType::RegisterPubkey as u8, 0x0D);
        assert_eq!(TxType::DisputeSlash as u8, 0x10);
    }

    #[test]
    fn fee_payer_tags_match_spec() {
        assert_eq!(FeePayer::Sender.discriminant_tag(), 0x00);
        assert_eq!(FeePayer::GasTank.discriminant_tag(), 0x01);
        let addr = Address::new([0x42; 32]);
        assert_eq!(FeePayer::Paymaster(addr).discriminant_tag(), 0x02);
    }

    #[test]
    fn access_type_tags_match_spec() {
        assert_eq!(AccessType::Read as u8, 0x00);
        assert_eq!(AccessType::ReadWrite as u8, 0x01);
    }

    #[test]
    fn tx_borsh_round_trip() {
        let tx = minimal_tx();
        let bytes = borsh::to_vec(&tx).unwrap();
        let decoded: Tx = borsh::from_slice(&bytes).unwrap();
        assert_eq!(tx, decoded);
    }

    #[test]
    fn tx_borsh_handles_full_fields() {
        let tx = Tx::unsigned(
            Address::new([0x01; 32]),
            Address::new([0x02; 32]),
            10_000,
            vec![1, 2, 3, 4],
            100_000,
            7,
            FeePayer::Paymaster(Address::new([0x03; 32])),
            vec![AccessEntry {
                address: Address::new([0x04; 32]),
                storage_keys: vec![[0u8; 32], [0xFF; 32]],
                access_type: AccessType::ReadWrite,
            }],
            Some(500),
            1,
            TxType::Standard,
        );
        let bytes = borsh::to_vec(&tx).unwrap();
        let decoded: Tx = borsh::from_slice(&bytes).unwrap();
        assert_eq!(tx, decoded);
    }

    #[test]
    fn deploy_tx_uses_zero_recipient() {
        let pk = FalconPubkey::new([0x55; crate::types::FALCON_PUBKEY_LEN]);
        let from = Address::from_pubkey(&pk);
        let tx = Tx::unsigned(
            from,
            Address::ZERO,
            0,
            b"wasm_bytecode_here".to_vec(),
            5_000_000,
            0,
            FeePayer::Sender,
            vec![],
            None,
            1,
            TxType::Deploy,
        );
        assert!(tx.to.is_zero());
        assert_eq!(tx.tx_type, TxType::Deploy);
    }
}
