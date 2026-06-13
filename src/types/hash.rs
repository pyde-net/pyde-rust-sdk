//! 32-byte hash newtypes — [`TxHash`], [`Blake3Hash`], [`Poseidon2Hash`].
//!
//! Pyde uses a dual-hash strategy throughout: **Blake3** for the
//! high-volume native paths (gossip, mempool, RPC integrity), and
//! **Poseidon2** for the ZK-friendly paths (tx hashes, state roots,
//! address derivation). All three newtypes are 32 bytes wide; the
//! type system enforces "right hash, right place."

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use serde::{Deserialize, Serialize};

/// Length of every hash newtype in this module, in bytes.
pub const HASH_LEN: usize = 32;

macro_rules! define_hash {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(
            Clone,
            Copy,
            Default,
            Eq,
            PartialEq,
            Ord,
            PartialOrd,
            Hash,
            BorshSerialize,
            BorshDeserialize,
            Serialize,
            Deserialize,
        )]
        pub struct $name(pub [u8; HASH_LEN]);

        impl $name {
            /// All-zero hash — sentinel for "no parent" / "empty tree" / "no tx."
            #[must_use]
            pub const fn zero() -> Self {
                Self([0u8; HASH_LEN])
            }

            /// Construct from a 32-byte array.
            #[must_use]
            pub const fn new(bytes: [u8; HASH_LEN]) -> Self {
                Self(bytes)
            }

            /// View the underlying bytes.
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; HASH_LEN] {
                &self.0
            }

            /// Lower-case hex representation, with the `0x` prefix.
            #[must_use]
            pub fn to_hex(&self) -> String {
                format!("0x{}", hex::encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.to_hex())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.to_hex())
            }
        }

        impl From<[u8; HASH_LEN]> for $name {
            fn from(bytes: [u8; HASH_LEN]) -> Self {
                Self(bytes)
            }
        }

        impl From<$name> for [u8; HASH_LEN] {
            fn from(h: $name) -> Self {
                h.0
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

define_hash!(
    TxHash,
    "A 32-byte transaction hash — Poseidon2 over the tx canonical \
     pre-image per Ch 11 §11.6. See [`crate::tx::tx_hash`]."
);
define_hash!(
    Blake3Hash,
    "A 32-byte Blake3 digest. Used for the high-volume native paths — \
     gossip, mempool, contract selectors per HOST_FN_ABI §3.7."
);
define_hash!(
    Poseidon2Hash,
    "A 32-byte Poseidon2 digest. Used everywhere Pyde needs a \
     ZK-friendly hash — light-client proofs, state roots, address derivation."
);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn zero_helpers_match() {
        assert_eq!(TxHash::zero().0, [0u8; 32]);
        assert_eq!(Blake3Hash::zero().0, [0u8; 32]);
        assert_eq!(Poseidon2Hash::zero().0, [0u8; 32]);
    }

    #[test]
    fn display_uses_0x_prefix() {
        let h = TxHash::new([0xCC; 32]);
        let s = format!("{h}");
        assert!(s.starts_with("0x"));
        assert_eq!(s.len(), 66);
    }

    #[test]
    fn borsh_round_trip() {
        let h = TxHash::new([0x42; 32]);
        let bytes = borsh::to_vec(&h).unwrap();
        assert_eq!(bytes.len(), HASH_LEN);
        let decoded: TxHash = borsh::from_slice(&bytes).unwrap();
        assert_eq!(h, decoded);
    }
}
