//! Contract ABI types — the schema embedded in the `pyde.abi` WASM
//! custom section.
//!
//! Mirrors `engine/crates/types/src/abi.rs` byte-for-byte. The
//! `otigen` toolchain builds these structs at build time and
//! Borsh-encodes them into a WASM custom section; the chain
//! decodes the section at deploy time to validate the contract.
//!
//! See [HOST_FN_ABI_SPEC §3.7] for the canonical schema.
//!
//! [HOST_FN_ABI_SPEC §3.7]: https://book.pyde.network/companion/HOST_FN_ABI_SPEC#37-the-pydeabi-custom-section

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::state_schema::StateSchema;

// ── Function attributes ────────────────────────────────────────

/// Function attribute bitflags.
///
/// Per [HOST_FN_ABI_SPEC §3.5] the attribute field is `u32`. Each
/// bit gates one runtime check; bit positions are wire-load-bearing
/// — never re-assign.
///
/// [HOST_FN_ABI_SPEC §3.5]: https://book.pyde.network/companion/HOST_FN_ABI_SPEC#35-function-attributes
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct FunctionAttrs {
    /// Raw bitflags. See associated constants for individual bits.
    pub bits: u32,
}

impl FunctionAttrs {
    /// Read-only function. Mutating host fns trap with
    /// `ERR_FORBIDDEN`. View-call execution is FREE — the caller
    /// pays only the dispatch base.
    pub const VIEW: u32 = 1 << 0;
    /// Accepts attached PYDE value (`tx.value > 0`). Non-payable
    /// functions reject value transfers.
    pub const PAYABLE: u32 = 1 << 1;
    /// Opts in to being called while already on the call stack.
    /// Default is non-reentrant.
    pub const REENTRANT: u32 = 1 << 2;
    /// Gas charged to the contract's `gas_tank` instead of the
    /// caller.
    pub const SPONSORED: u32 = 1 << 3;
    /// Callable only at deploy time. Later calls reject with
    /// `ERR_CONSTRUCTOR_REENTRANT`.
    pub const CONSTRUCTOR: u32 = 1 << 4;
    /// Catch-all when no declared function name matches. At most
    /// one per contract.
    pub const FALLBACK: u32 = 1 << 5;
    /// Handler for bare value transfers. At most one per contract;
    /// must also be `PAYABLE`.
    pub const RECEIVE: u32 = 1 << 6;
    /// Function is callable from outside the contract. Required
    /// for every function not marked with another dispatch
    /// attribute.
    pub const ENTRY: u32 = 1 << 7;

    /// Construct from raw bits.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    /// True if any of the given bits are set.
    #[must_use]
    pub const fn has(&self, bits: u32) -> bool {
        self.bits & bits != 0
    }

    /// True if ALL the given bits are set.
    #[must_use]
    pub const fn has_all(&self, bits: u32) -> bool {
        self.bits & bits == bits
    }

    /// `true` iff this function is read-only ([`Self::VIEW`]).
    #[must_use]
    pub const fn is_view(&self) -> bool {
        self.has(Self::VIEW)
    }

    /// `true` iff this function accepts attached value
    /// ([`Self::PAYABLE`]).
    #[must_use]
    pub const fn is_payable(&self) -> bool {
        self.has(Self::PAYABLE)
    }

    /// `true` iff this function is the contract constructor.
    #[must_use]
    pub const fn is_constructor(&self) -> bool {
        self.has(Self::CONSTRUCTOR)
    }
}

// ── ParamType ──────────────────────────────────────────────────

/// ABI-level parameter type.
///
/// Maps `otigen.toml`-declared types to canonical Borsh encoding.
/// See HOST_FN_ABI §3.7.3 for the full table.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum ParamType {
    /// `u8`.
    U8,
    /// `u16`.
    U16,
    /// `u32`.
    U32,
    /// `u64`.
    U64,
    /// `u128`.
    U128,
    /// `i8`.
    I8,
    /// `i16`.
    I16,
    /// `i32`.
    I32,
    /// `i64`.
    I64,
    /// `i128`.
    I128,
    /// Boolean.
    Bool,
    /// 32-byte Pyde address.
    Address,
    /// Variable-length byte array.
    Bytes,
    /// UTF-8 string.
    String,
    /// Fixed-size byte array (for hash digests, salts, etc.).
    FixedBytes(u32),
    /// Homogeneous list of inner type.
    Vec(Box<ParamType>),
    /// Key-value map.
    Map {
        /// Key type.
        key: Box<ParamType>,
        /// Value type.
        value: Box<ParamType>,
    },
    /// Optional value.
    Option(Box<ParamType>),
    /// Reference to a contract-defined custom type by name. Looked
    /// up against [`ContractAbi::types`].
    Custom(String),
}

// ── ParamAbi / FunctionAbi / EventAbi ──────────────────────────

/// A single named ABI parameter.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ParamAbi {
    /// Human-readable name (for tooling; ignored by the chain).
    pub name: String,
    /// Canonical type.
    pub ty: ParamType,
}

/// ABI entry for one contract function.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct FunctionAbi {
    /// 4-byte function selector (typically `Blake3(name)[..4]`).
    pub selector: [u8; 4],
    /// Human-readable function name.
    pub name: String,
    /// Attributes.
    pub attrs: FunctionAttrs,
    /// Ordered parameter list.
    pub params: Vec<ParamAbi>,
    /// Return type, or `None` for `()`.
    pub returns: Option<ParamType>,
}

/// ABI entry for one event.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct EventAbi {
    /// Human-readable event name.
    pub name: String,
    /// Parameters in declaration order.
    pub params: Vec<ParamAbi>,
    /// Bitmask: bit `i` set means parameter `i` is emitted as a
    /// topic (indexed); otherwise it lives in the event data. At
    /// most 4 bits may be set (the topic cap per HOST_FN_ABI §15.3).
    pub indexed_mask: u8,
}

impl EventAbi {
    /// Number of indexed parameters (topics) on this event.
    #[must_use]
    pub fn topic_count(&self) -> u8 {
        self.indexed_mask.count_ones() as u8
    }

    /// `true` iff parameter at `index` is indexed (emitted as a
    /// topic). Returns `false` for out-of-range indices.
    #[must_use]
    pub fn is_indexed(&self, index: u8) -> bool {
        if index >= 8 {
            return false;
        }
        (self.indexed_mask >> index) & 1 == 1
    }
}

// ── Custom types ───────────────────────────────────────────────

/// A contract-author-declared composite type. Referenced from
/// [`ParamType::Custom`] in function / event / state schema
/// declarations.
///
/// The chain treats Custom-typed values as opaque Borsh bytes —
/// this struct exists purely so wallets / indexers / explorers
/// can self-decode return values + event payloads without an
/// out-of-band sidecar.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct TypeAbi {
    /// User-facing type name. Must be unique within the contract.
    pub name: String,
    /// Shape — struct or enum.
    pub kind: TypeKind,
}

/// The two kinds of composite types contracts can declare.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum TypeKind {
    /// Named-field record. Borsh wire = field-by-field
    /// concatenation in declaration order.
    Struct {
        /// Fields in declaration order.
        fields: Vec<ParamAbi>,
    },
    /// Tagged union. Borsh wire = `u8 variant_tag + payload`.
    /// Variant tag = declaration-order index.
    Enum {
        /// Variants in declaration order.
        variants: Vec<EnumVariant>,
    },
}

/// One variant of a [`TypeKind::Enum`]. v1 supports unit variants
/// only (no payload — just the tag byte).
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct EnumVariant {
    /// Variant name (Borsh tags by index, not name; the name is for
    /// off-chain tooling).
    pub name: String,
}

// ── Contract type ──────────────────────────────────────────────

/// Whether a deployed module is a smart contract or a parachain.
/// Decides the eligible host-fn import set at deploy time.
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
pub enum ContractType {
    /// Ordinary smart contract. Restricted to the §7 host-fn set.
    Contract = 0x00,
    /// Parachain. Additionally allowed the §8 functions.
    Parachain = 0x01,
}

// ── ContractAbi ────────────────────────────────────────────────

/// The full ABI of a contract — Borsh-encoded and embedded in the
/// WASM `pyde.abi` custom section.
///
/// The chain stores only the `.wasm` bytes; this struct travels
/// with the code. Wallets and indexers parse the custom section
/// via [`crate::abi::extract_abi`].
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ContractAbi {
    /// Semver-packed ABI version (high 16 = major, low 16 = minor).
    pub pyde_abi_version: u32,
    /// Smart contract vs parachain.
    pub contract_type: ContractType,
    /// Exported callable functions, in declaration order.
    pub functions: Vec<FunctionAbi>,
    /// Blake3 hash of the canonical state-schema encoding.
    pub state_schema_hash: [u8; 32],
    /// Index into `functions` of the constructor, if any.
    pub constructor_index: Option<u32>,
    /// Index into `functions` of the fallback handler, if any.
    pub fallback_index: Option<u32>,
    /// Index into `functions` of the receive handler, if any.
    pub receive_index: Option<u32>,
    /// Contract name (extension; from `otigen.toml`).
    pub name: String,
    /// Contract semver (extension; from `otigen.toml`).
    pub version: String,
    /// Declared events.
    pub events: Vec<EventAbi>,
    /// Parachain capabilities this contract imports. Empty for
    /// `contract_type = Contract`.
    pub parachain_imports: Vec<String>,
    /// Declared persistent-storage schema.
    pub state_schema: StateSchema,
    /// Author-declared custom types referenced via
    /// [`ParamType::Custom`].
    pub types: Vec<TypeAbi>,
}

impl ContractAbi {
    /// Original ABI version (`0x0001_0000` — v1.0).
    pub const V1_0: u32 = 0x0001_0000;
    /// v1.1 added [`Self::state_schema`].
    pub const V1_1: u32 = 0x0001_0001;
    /// Current ABI version. v1.2 added [`Self::types`].
    pub const V1_2: u32 = 0x0001_0002;
    /// Alias for older callers.
    pub const SCHEMA_V1: u32 = Self::V1_2;
    /// Max version the SDK Borsh-decodes today.
    pub const MAX_SUPPORTED: u32 = Self::V1_2;

    /// Extract the major version from a packed version word.
    #[must_use]
    pub const fn major(version: u32) -> u16 {
        (version >> 16) as u16
    }

    /// Extract the minor version from a packed version word.
    #[must_use]
    pub const fn minor(version: u32) -> u16 {
        (version & 0xFFFF) as u16
    }

    /// Find a function by selector.
    #[must_use]
    pub fn function_by_selector(&self, selector: &[u8; 4]) -> Option<&FunctionAbi> {
        self.functions.iter().find(|f| &f.selector == selector)
    }

    /// Find a function by name.
    #[must_use]
    pub fn function_by_name(&self, name: &str) -> Option<&FunctionAbi> {
        self.functions.iter().find(|f| f.name == name)
    }

    /// Find an event by name.
    #[must_use]
    pub fn event_by_name(&self, name: &str) -> Option<&EventAbi> {
        self.events.iter().find(|e| e.name == name)
    }

    /// Find a custom type by name.
    #[must_use]
    pub fn type_by_name(&self, name: &str) -> Option<&TypeAbi> {
        self.types.iter().find(|t| t.name == name)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn function_attrs_bit_layout_is_stable() {
        // Spec-load-bearing — changing these is a hard fork.
        assert_eq!(FunctionAttrs::VIEW, 1 << 0);
        assert_eq!(FunctionAttrs::PAYABLE, 1 << 1);
        assert_eq!(FunctionAttrs::REENTRANT, 1 << 2);
        assert_eq!(FunctionAttrs::SPONSORED, 1 << 3);
        assert_eq!(FunctionAttrs::CONSTRUCTOR, 1 << 4);
        assert_eq!(FunctionAttrs::FALLBACK, 1 << 5);
        assert_eq!(FunctionAttrs::RECEIVE, 1 << 6);
        assert_eq!(FunctionAttrs::ENTRY, 1 << 7);
    }

    #[test]
    fn function_attrs_helpers() {
        let a = FunctionAttrs::from_bits(FunctionAttrs::VIEW | FunctionAttrs::PAYABLE);
        assert!(a.is_view());
        assert!(a.is_payable());
        assert!(!a.is_constructor());
        assert!(a.has_all(FunctionAttrs::VIEW | FunctionAttrs::PAYABLE));
    }

    #[test]
    fn contract_type_tags_stable() {
        assert_eq!(borsh::to_vec(&ContractType::Contract).unwrap(), vec![0x00]);
        assert_eq!(borsh::to_vec(&ContractType::Parachain).unwrap(), vec![0x01]);
    }

    #[test]
    fn version_packing() {
        assert_eq!(ContractAbi::V1_0, 0x0001_0000);
        assert_eq!(ContractAbi::V1_2, 0x0001_0002);
        assert_eq!(ContractAbi::major(0x0002_0007), 2);
        assert_eq!(ContractAbi::minor(0x0002_0007), 7);
    }

    #[test]
    fn event_topic_count_and_indexed() {
        let e = EventAbi {
            name: "Transfer".into(),
            params: vec![],
            indexed_mask: 0b0000_0101,
        };
        assert_eq!(e.topic_count(), 2);
        assert!(e.is_indexed(0));
        assert!(!e.is_indexed(1));
        assert!(e.is_indexed(2));
        assert!(!e.is_indexed(8));
    }

    fn sample_abi() -> ContractAbi {
        ContractAbi {
            pyde_abi_version: ContractAbi::V1_2,
            contract_type: ContractType::Contract,
            functions: vec![FunctionAbi {
                selector: [0x12, 0x34, 0x56, 0x78],
                name: "transfer".into(),
                attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY | FunctionAttrs::PAYABLE),
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
                returns: Some(ParamType::Bool),
            }],
            state_schema_hash: [0xAB; 32],
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
    fn contract_abi_borsh_round_trip() {
        let abi = sample_abi();
        let bytes = borsh::to_vec(&abi).unwrap();
        let decoded: ContractAbi = borsh::from_slice(&bytes).unwrap();
        assert_eq!(abi, decoded);
    }

    #[test]
    fn lookups_work() {
        let abi = sample_abi();
        assert!(abi.function_by_name("transfer").is_some());
        assert!(abi.function_by_name("nope").is_none());
        assert!(abi
            .function_by_selector(&[0x12, 0x34, 0x56, 0x78])
            .is_some());
        assert!(abi.event_by_name("Transferred").is_some());
    }
}
