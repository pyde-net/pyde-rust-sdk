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
pub mod error_code;
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
pub use error_code::{
    ErrorCode, ERR_ACCESS_LIST_VIOLATION, ERR_CIPHERTEXT_INVALID, ERR_CROSS_CALL_FAILED,
    ERR_CROSS_CALL_OUT_OF_GAS, ERR_FORBIDDEN, ERR_INSUFFICIENT_BALANCE, ERR_INTERNAL,
    ERR_INVALID_ADDRESS, ERR_INVALID_FUNCTION_NAME, ERR_INVALID_INPUT, ERR_NOT_FOUND,
    ERR_OUTPUT_BUFFER_TOO_SMALL, ERR_OUT_OF_GAS, ERR_PARACHAIN_ONLY, ERR_REENTRANCY_BLOCKED,
    ERR_SIGNATURE_INVALID, ERR_VALUE_TRANSFER_NOT_PAYABLE, ERR_XCALL_RATE_LIMITED,
};
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
    AccessEntry, AccessType, CallPayload, DeployData, FeePayer, FeeQuanta, Gas, GasUsed, Tx,
    TxType, MAX_CALLDATA, MAX_TX_SIZE, MIN_GAS_LIMIT,
};
