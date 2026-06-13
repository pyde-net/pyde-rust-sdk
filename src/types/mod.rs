//! Public types — addresses, hashes, FALCON keys, transactions,
//! accounts, RPC responses.
//!
//! These are the SDK's canonical wire types. Every type whose
//! Borsh encoding crosses the engine boundary is byte-for-byte
//! identical to its counterpart in
//! `engine/crates/types/src/`, so a transaction built and signed
//! by the SDK is decoded as-is by a validator.
//!
//! Submodules:
//!
//! - [`address`] — `Address` newtype + EOA/CREATE/CREATE2/contract-name
//!   derivation
//! - [`hash`] — `TxHash`, `Blake3Hash`, `Poseidon2Hash` newtypes
//! - [`falcon`] — `FalconPubkey`, `FalconSignature`, `FalconSecret`
//!   wire newtypes + bridges to [`pyde_crypto::falcon`]
//! - [`account`] — `AccountType`, `AuthKeys`, nonce-window constants
//! - [`tx_types`] — `Tx`, `TxType`, `FeePayer`, `AccessEntry`,
//!   `AccessType`, the wire-frozen envelope
//! - [`rpc`] — JSON-RPC response shapes (`Receipt`, `Log`, `LogFilter`,
//!   `BlockHeader`, `CallOverrides`, `FeeData`)

pub mod abi;
pub mod account;
pub mod address;
pub mod falcon;
pub mod hash;
pub mod rpc;
pub mod state_schema;
pub mod tx_types;

// ── Flat re-exports — the surface most consumers reach for. ───
pub use abi::{
    ContractAbi, ContractType, EnumVariant, EventAbi, FunctionAbi, FunctionAttrs, ParamAbi,
    ParamType, TypeAbi, TypeKind,
};
pub use account::{
    AccountType, AuthKeys, Balance, InvalidAuthKeys, Nonce, MAX_MULTISIG_SIGNERS, NONCE_WINDOW_SIZE,
};
pub use address::{Address, ADDRESS_LEN, CONTRACT_ADDRESS_PREFIX, CREATE2_PREFIX};
pub use falcon::{
    FalconPubkey, FalconSecret, FalconSignature, FALCON_PUBKEY_LEN, FALCON_SECRET_LEN,
    FALCON_SIG_MAX_LEN,
};
pub use hash::{Blake3Hash, Poseidon2Hash, TxHash, HASH_LEN};
pub use rpc::{
    AccountInfo, BlockHeader, CallOverrides, CallRequest, Event, EventFilter, FeeData, Log,
    LogCursor, LogFilter, LogPage, NodeInfo, Receipt, ReceiptStatus, SimulationAccessList,
    SimulationRead, SimulationReadVersion, SimulationReceipt, SimulationResult, WaveHeader,
};
pub use state_schema::{FieldKind, ScalarType, StateField, StateSchema};
pub use tx_types::{
    AccessEntry, AccessType, CallPayload, FeePayer, FeeQuanta, Gas, GasUsed, Tx, TxType,
    MAX_CALLDATA, MAX_TX_SIZE, MIN_GAS_LIMIT,
};
