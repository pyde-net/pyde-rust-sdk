//! Contract persistent-storage schema, as declared in the
//! deployed `pyde.abi` custom section.
//!
//! Mirrors `engine/crates/types/src/state_schema.rs` byte-for-byte.
//! The schema is part of the [`crate::types::ContractAbi`] wire
//! shape; reordering or renaming variants is a wire break.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// One scalar type the chain knows how to validate.
///
/// Variants are listed in declaration order — new variants append
/// at the end so existing Borsh discriminants don't shift.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum ScalarType {
    /// `u8` — 1 byte LE.
    U8,
    /// `u16` — 2 bytes LE.
    U16,
    /// `u32` — 4 bytes LE.
    U32,
    /// `u64` — 8 bytes LE.
    U64,
    /// `u128` — 16 bytes LE.
    U128,
    /// `i8` — 1 byte two's-complement.
    I8,
    /// `i16` — 2 bytes.
    I16,
    /// `i32` — 4 bytes.
    I32,
    /// `i64` — 8 bytes.
    I64,
    /// `i128` — 16 bytes.
    I128,
    /// Boolean — 1 byte (`0x00`/`0x01`).
    Bool,
    /// 32-byte Pyde address.
    Address,
    /// 32-byte hash digest (Blake3 / Poseidon2).
    Hash32,
    /// Variable-length opaque bytes. `u32_le` length prefix.
    Bytes,
    /// Variable-length UTF-8 string. `u32_le` length prefix.
    String,
    /// Homogeneous array. `u32_le` length prefix + elements.
    Vec(Box<ScalarType>),
}

impl ScalarType {
    /// Fixed byte width, or `None` for variable-width types.
    #[must_use]
    pub const fn fixed_byte_width(&self) -> Option<usize> {
        match self {
            Self::U8 | Self::I8 | Self::Bool => Some(1),
            Self::U16 | Self::I16 => Some(2),
            Self::U32 | Self::I32 => Some(4),
            Self::U64 | Self::I64 => Some(8),
            Self::U128 | Self::I128 => Some(16),
            Self::Address | Self::Hash32 => Some(32),
            Self::Bytes | Self::String | Self::Vec(_) => None,
        }
    }
}

/// Shape of a state field — single value or a keyed map.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldKind {
    /// Single-value field.
    Scalar {
        /// Value type at the slot.
        value: ScalarType,
    },
    /// Keyed map (1-3 keys).
    Map {
        /// Key types in declaration order.
        keys: Vec<ScalarType>,
        /// Value type at the derived slot.
        value: ScalarType,
    },
}

/// One declared persistent-storage field.
#[derive(
    Clone, Debug, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct StateField {
    /// Field name (matches the `[state.<name>]` block).
    pub name: String,
    /// Single-value or keyed-map shape.
    pub kind: FieldKind,
}

/// Full state schema embedded in the contract's [`crate::types::ContractAbi`].
#[derive(
    Clone,
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
pub struct StateSchema {
    /// Declared fields in declaration order. Order is wire-load-
    /// bearing — the schema's Blake3 hash (held in
    /// [`crate::types::ContractAbi::state_schema_hash`]) covers it.
    pub fields: Vec<StateField>,
}

impl StateSchema {
    /// Construct an empty schema.
    #[must_use]
    pub const fn empty() -> Self {
        Self { fields: Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn scalar_widths_are_correct() {
        assert_eq!(ScalarType::U8.fixed_byte_width(), Some(1));
        assert_eq!(ScalarType::U128.fixed_byte_width(), Some(16));
        assert_eq!(ScalarType::Address.fixed_byte_width(), Some(32));
        assert_eq!(ScalarType::Bytes.fixed_byte_width(), None);
        assert_eq!(
            ScalarType::Vec(Box::new(ScalarType::U64)).fixed_byte_width(),
            None
        );
    }

    #[test]
    fn schema_borsh_round_trip() {
        let schema = StateSchema {
            fields: vec![
                StateField {
                    name: "total_supply".into(),
                    kind: FieldKind::Scalar {
                        value: ScalarType::U128,
                    },
                },
                StateField {
                    name: "balances".into(),
                    kind: FieldKind::Map {
                        keys: vec![ScalarType::Address],
                        value: ScalarType::U128,
                    },
                },
            ],
        };
        let bytes = borsh::to_vec(&schema).unwrap();
        let decoded: StateSchema = borsh::from_slice(&bytes).unwrap();
        assert_eq!(schema, decoded);
    }
}
