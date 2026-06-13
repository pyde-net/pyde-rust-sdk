//! Account types — `AccountType`, `AuthKeys`, nonce window.
//!
//! Mirrors `engine/crates/types/src/account.rs` byte-for-byte. The
//! wire format is frozen at v1; reordering variants or changing tag
//! values is a hard fork.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::FalconPubkey;

/// Number of slots in an account's sliding nonce window.
///
/// Pyde uses a 16-slot window (vs Ethereum's strict sequential
/// nonces) so a wallet can have multiple in-flight transactions
/// concurrently. The mempool accepts any `nonce ∈ [base, base + 16)`
/// that hasn't already been committed; the window slides forward
/// as the low-end bits fill in.
pub const NONCE_WINDOW_SIZE: usize = 16;

/// Maximum signer count in [`AuthKeys::MultiSig`].
///
/// Per Ch 11 §11.5, native multisig is capped at 16 to keep the
/// per-account auth-bytes footprint compact. Weighted / N-of-many
/// schemes live at the contract layer.
pub const MAX_MULTISIG_SIGNERS: usize = 16;

/// Account kind — EOA, contract, or system-reserved.
///
/// Wire tag values from Ch 11 §11.3:
///
/// | Tag  | Variant     |
/// |------|-------------|
/// | 0x00 | [`AccountType::Eoa`]      |
/// | 0x01 | [`AccountType::Contract`] |
/// | 0x02 | [`AccountType::System`]   |
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
pub enum AccountType {
    /// Externally-owned account — held by a keypair, authorized by
    /// an [`AuthKeys`] variant.
    Eoa = 0x00,
    /// Smart contract — holds a balance, runs WASM, has storage.
    Contract = 0x01,
    /// Protocol-reserved (treasury, burn sink, parachain registry,
    /// …). Mutated only by specific handlers.
    System = 0x02,
}

/// Account-native balance in quanta. `1 PYDE = 10^9 quanta`.
///
/// `u128` is wider than the total supply needs but accommodates
/// arithmetic during fee distribution and staking yield without
/// overflow.
pub type Balance = u128;

/// Per-account transaction counter.
///
/// Pyde nonces aren't strictly sequential — they live inside a
/// 16-slot window. See [`NONCE_WINDOW_SIZE`].
pub type Nonce = u64;

/// Authorization keys for an account.
///
/// Tag values from Ch 11 §11.5 — wire-stable, never reassign:
///
/// | Tag  | Variant                              |
/// |------|--------------------------------------|
/// | 0x00 | [`AuthKeys::None`]                    |
/// | 0x01 | [`AuthKeys::Single`]                  |
/// | 0x02 | [`AuthKeys::MultiSig`]                |
/// | 0x03 | [`AuthKeys::Programmable`] (v2 reserved) |
///
/// `Programmable` is encoded but rejected by v1 engines —
/// reserving the discriminant lets v2 ship programmable accounts
/// without a wire-breaking change.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[borsh(use_discriminant = true)]
#[repr(u8)]
pub enum AuthKeys {
    /// No authorization. Valid for system accounts and contracts
    /// without an admin key.
    None = 0x00,
    /// Single FALCON-512 pubkey. The default for EOAs.
    Single(FalconPubkey) = 0x01,
    /// k-of-n native multisig.
    ///
    /// `threshold` must satisfy `1 <= threshold <= signers.len()` and
    /// `signers.len() <= MAX_MULTISIG_SIGNERS`. The
    /// [`AuthKeys::validate`] helper checks both.
    MultiSig {
        /// Required signatures to authorize a tx.
        threshold: u8,
        /// Ordered authorized pubkey set.
        signers: Vec<FalconPubkey>,
    } = 0x02,
    /// **RESERVED for v2.** Programmable / smart-contract auth.
    /// v1 nodes reject any tx whose sender uses this variant.
    Programmable {
        /// Opaque policy bytes; interpreted under v2.
        policy: Vec<u8>,
    } = 0x03,
}

impl AuthKeys {
    /// Whether this auth shape is accepted by a v1 engine.
    ///
    /// `Programmable` is reserved at v1 and rejected at validation.
    #[must_use]
    pub fn is_v1_supported(&self) -> bool {
        match self {
            Self::None | Self::Single(_) | Self::MultiSig { .. } => true,
            Self::Programmable { .. } => false,
        }
    }

    /// Validate the structural invariants of the variant.
    ///
    /// `MultiSig` is checked for `threshold` and signer-count bounds;
    /// other variants always validate.
    ///
    /// # Errors
    /// [`InvalidAuthKeys`] when a `MultiSig` set has zero signers,
    /// more than [`MAX_MULTISIG_SIGNERS`], or a threshold outside
    /// `[1, signers.len()]`.
    pub fn validate(&self) -> Result<(), InvalidAuthKeys> {
        if let Self::MultiSig { threshold, signers } = self {
            if signers.is_empty() {
                return Err(InvalidAuthKeys::NoSigners);
            }
            if signers.len() > MAX_MULTISIG_SIGNERS {
                return Err(InvalidAuthKeys::TooManySigners {
                    signers: signers.len(),
                    max: MAX_MULTISIG_SIGNERS,
                });
            }
            if *threshold == 0 || (*threshold as usize) > signers.len() {
                return Err(InvalidAuthKeys::InvalidThreshold {
                    threshold: *threshold,
                    signers: signers.len(),
                });
            }
        }
        Ok(())
    }
}

/// Errors from constructing an [`AuthKeys`] that violates a
/// structural invariant. Wire decoding does NOT invoke this check —
/// callers run it explicitly when building auth from user input.
#[derive(Debug, Error)]
pub enum InvalidAuthKeys {
    /// `MultiSig.threshold` was zero or exceeded `signers.len()`.
    #[error("multisig threshold {threshold} invalid for {signers} signers")]
    InvalidThreshold {
        /// The declared threshold.
        threshold: u8,
        /// The declared signer count.
        signers: usize,
    },
    /// `MultiSig.signers.len()` exceeded [`MAX_MULTISIG_SIGNERS`].
    #[error("multisig signer count {signers} exceeds maximum {max}")]
    TooManySigners {
        /// Declared signer count.
        signers: usize,
        /// Configured cap.
        max: usize,
    },
    /// `MultiSig.signers` was empty.
    #[error("multisig requires at least one signer")]
    NoSigners,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::FALCON_PUBKEY_LEN;

    fn pk(byte: u8) -> FalconPubkey {
        FalconPubkey::new([byte; FALCON_PUBKEY_LEN])
    }

    #[test]
    fn account_type_tag_values_match_spec() {
        assert_eq!(AccountType::Eoa as u8, 0x00);
        assert_eq!(AccountType::Contract as u8, 0x01);
        assert_eq!(AccountType::System as u8, 0x02);
    }

    #[test]
    fn auth_keys_v1_support_flags() {
        assert!(AuthKeys::None.is_v1_supported());
        assert!(AuthKeys::Single(pk(1)).is_v1_supported());
        assert!(AuthKeys::MultiSig {
            threshold: 2,
            signers: vec![pk(1), pk(2), pk(3)]
        }
        .is_v1_supported());
        assert!(!AuthKeys::Programmable {
            policy: vec![0u8; 8]
        }
        .is_v1_supported());
    }

    #[test]
    fn multisig_validates_signer_bounds() {
        // Threshold 0 → reject.
        let bad = AuthKeys::MultiSig {
            threshold: 0,
            signers: vec![pk(1)],
        };
        assert!(bad.validate().is_err());

        // Threshold > signers → reject.
        let bad = AuthKeys::MultiSig {
            threshold: 5,
            signers: vec![pk(1)],
        };
        assert!(bad.validate().is_err());

        // Empty signer set → reject.
        let bad = AuthKeys::MultiSig {
            threshold: 1,
            signers: vec![],
        };
        assert!(bad.validate().is_err());

        // Too many signers → reject.
        let bad = AuthKeys::MultiSig {
            threshold: 1,
            signers: (0..(MAX_MULTISIG_SIGNERS + 1) as u8).map(pk).collect(),
        };
        assert!(bad.validate().is_err());

        // Sane shape → accept.
        let ok = AuthKeys::MultiSig {
            threshold: 2,
            signers: vec![pk(1), pk(2), pk(3)],
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn auth_keys_borsh_round_trip() {
        for variant in [
            AuthKeys::None,
            AuthKeys::Single(pk(0x42)),
            AuthKeys::MultiSig {
                threshold: 2,
                signers: vec![pk(0x01), pk(0x02), pk(0x03)],
            },
            AuthKeys::Programmable {
                policy: vec![0xDE, 0xAD, 0xBE, 0xEF],
            },
        ] {
            let bytes = borsh::to_vec(&variant).unwrap();
            let decoded: AuthKeys = borsh::from_slice(&bytes).unwrap();
            assert_eq!(variant, decoded);
        }
    }
}
