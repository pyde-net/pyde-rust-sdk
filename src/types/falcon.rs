//! FALCON-512 public keys and signatures — wire newtypes.
//!
//! FALCON-512 is Pyde's NIST-Level-1 post-quantum signature scheme.
//! These types are the **wire shape** — fixed-size pubkey, variable
//! signature — so they Borsh-encode byte-for-byte identical to the
//! engine's `crates/types/src/signature.rs`.
//!
//! The actual cryptographic operations (keygen / sign / verify) live
//! in the sibling [`pyde_crypto::falcon`] crate; that crate uses its
//! own `Vec`-backed types internally. Conversions in both directions
//! are provided so signer code can move bytes between the two
//! representations cheaply.

use borsh::{BorshDeserialize, BorshSerialize};
use core::fmt;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::SdkError;

/// Length of a FALCON-512 public key, in bytes. Fixed.
pub const FALCON_PUBKEY_LEN: usize = 897;

/// Length of a FALCON-512 secret key, in bytes. Fixed.
pub const FALCON_SECRET_LEN: usize = 1281;

/// Upper bound on a FALCON-512 signature, in bytes. The real
/// signature is variable (~666 B average) — the constant is the
/// generous over-allocation cap. Encoded sigs include their length,
/// so the over-allocation never bloats wire bytes.
pub const FALCON_SIG_MAX_LEN: usize = 690;

/// A FALCON-512 public key — 897 fixed bytes.
///
/// Wire shape: Borsh-encodes as a `[u8; 897]` (no length prefix),
/// matching the engine's `crates/types/src/signature.rs::FalconPubkey`.
#[derive(Clone, Copy, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct FalconPubkey(#[serde(with = "serde_arr_897")] pub [u8; FALCON_PUBKEY_LEN]);

impl FalconPubkey {
    /// Construct from a fixed-size 897-byte array.
    #[must_use]
    pub const fn new(bytes: [u8; FALCON_PUBKEY_LEN]) -> Self {
        Self(bytes)
    }

    /// View the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; FALCON_PUBKEY_LEN] {
        &self.0
    }

    /// Parse from a slice. Returns `None` if the slice is not
    /// exactly [`FALCON_PUBKEY_LEN`] bytes.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != FALCON_PUBKEY_LEN {
            return None;
        }
        let mut out = [0u8; FALCON_PUBKEY_LEN];
        out.copy_from_slice(bytes);
        Some(Self(out))
    }

    /// Lower-case hex representation, with the `0x` prefix.
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Parse a `0x`-prefixed (or bare) hex string into a pubkey.
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidArgument`] on length or hex mismatch.
    pub fn from_hex(s: &str) -> Result<Self, SdkError> {
        let h = s.trim_start_matches("0x");
        let bytes = hex::decode(h)
            .map_err(|e| SdkError::InvalidArgument(format!("invalid pubkey hex: {e}")))?;
        Self::from_slice(&bytes).ok_or_else(|| {
            SdkError::InvalidArgument(format!(
                "expected {FALCON_PUBKEY_LEN}-byte pubkey, got {}",
                bytes.len()
            ))
        })
    }
}

impl fmt::Debug for FalconPubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Pubkeys are 897 B — never print the full thing in logs.
        write!(f, "FalconPubkey(0x{}…)", hex::encode(&self.0[..8]))
    }
}

impl PartialEq for FalconPubkey {
    fn eq(&self, other: &Self) -> bool {
        self.0[..] == other.0[..]
    }
}

impl Eq for FalconPubkey {}

impl core::hash::Hash for FalconPubkey {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl From<[u8; FALCON_PUBKEY_LEN]> for FalconPubkey {
    fn from(bytes: [u8; FALCON_PUBKEY_LEN]) -> Self {
        Self(bytes)
    }
}

impl From<&pyde_crypto::falcon::FalconPublicKey> for FalconPubkey {
    fn from(pk: &pyde_crypto::falcon::FalconPublicKey) -> Self {
        // `pyde-crypto` enforces the 897-byte invariant in its
        // constructors, so `as_bytes()` always returns the right
        // length here. Fall back to zero on the impossible-shouldn't-
        // happen path rather than panicking.
        Self::from_slice(pk.as_bytes()).unwrap_or(Self([0u8; FALCON_PUBKEY_LEN]))
    }
}

impl From<&FalconPubkey> for pyde_crypto::falcon::FalconPublicKey {
    fn from(pk: &FalconPubkey) -> Self {
        // Same invariant: 897 bytes round-trips losslessly. The
        // `from_bytes` returns `Option` because pyde-crypto accepts
        // arbitrary input slices; we know ours is well-formed.
        pyde_crypto::falcon::FalconPublicKey::from_bytes(&pk.0).unwrap_or_else(|| {
            // Impossible — `pk.0` is statically 897 bytes. We
            // surface a zeroed key rather than panicking.
            pyde_crypto::falcon::FalconPublicKey::from_bytes(&[0u8; FALCON_PUBKEY_LEN])
                .unwrap_or_else(|| unreachable!("897-byte zero buffer is valid"))
        })
    }
}

/// A FALCON-512 signature — variable length (≈666 B average).
///
/// Wire shape: Borsh-encodes as a `Vec<u8>` (4-byte length prefix +
/// payload), matching the engine's
/// `crates/types/src/signature.rs::FalconSignature`.
#[derive(Clone, Eq, PartialEq, Hash, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct FalconSignature(pub Vec<u8>);

impl FalconSignature {
    /// Construct from raw bytes. Length is not validated here —
    /// signature shape is checked by FALCON's verifier at the chain
    /// edge; the wire newtype just carries the bytes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// View the signature bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the signature is empty (length zero).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Lower-case hex representation, with the `0x` prefix.
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

impl fmt::Debug for FalconSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.len() >= 8 {
            write!(
                f,
                "FalconSignature({} B; 0x{}…)",
                self.0.len(),
                hex::encode(&self.0[..8])
            )
        } else {
            write!(f, "FalconSignature(0x{})", hex::encode(&self.0))
        }
    }
}

impl From<Vec<u8>> for FalconSignature {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

impl From<&pyde_crypto::falcon::FalconSignature> for FalconSignature {
    fn from(sig: &pyde_crypto::falcon::FalconSignature) -> Self {
        Self(sig.to_vec())
    }
}

/// A FALCON-512 secret key — 1281 fixed bytes.
///
/// **Sensitive material.** Zeroizes on drop so the secret bytes
/// don't linger in deallocated heap pages where a later allocation,
/// swap-to-disk page, or core dump could read them.
///
/// Not Borsh/Serde — secret keys never go on the wire. The encrypted
/// keystore in [`crate::wallet`] is the only persistence path.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct FalconSecret(pub [u8; FALCON_SECRET_LEN]);

impl FalconSecret {
    /// Construct from a fixed-size 1281-byte array.
    #[must_use]
    pub const fn new(bytes: [u8; FALCON_SECRET_LEN]) -> Self {
        Self(bytes)
    }

    /// View the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; FALCON_SECRET_LEN] {
        &self.0
    }

    /// Parse from a slice. Returns `None` if the slice is not
    /// exactly [`FALCON_SECRET_LEN`] bytes.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != FALCON_SECRET_LEN {
            return None;
        }
        let mut out = [0u8; FALCON_SECRET_LEN];
        out.copy_from_slice(bytes);
        Some(Self(out))
    }
}

impl fmt::Debug for FalconSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // NEVER print secret bytes.
        f.write_str("FalconSecret(<redacted>)")
    }
}

impl From<&pyde_crypto::falcon::FalconSecretKey> for FalconSecret {
    fn from(sk: &pyde_crypto::falcon::FalconSecretKey) -> Self {
        Self::from_slice(sk.as_bytes()).unwrap_or(Self([0u8; FALCON_SECRET_LEN]))
    }
}

impl From<&FalconSecret> for pyde_crypto::falcon::FalconSecretKey {
    fn from(sk: &FalconSecret) -> Self {
        pyde_crypto::falcon::FalconSecretKey::from_bytes(&sk.0).unwrap_or_else(|| {
            pyde_crypto::falcon::FalconSecretKey::from_bytes(&[0u8; FALCON_SECRET_LEN])
                .unwrap_or_else(|| unreachable!("1281-byte zero buffer is valid"))
        })
    }
}

/// Serde adapter for serializing a 897-byte array as a byte string
/// rather than a 897-element sequence (which is the default `serde`
/// handling for `[u8; N]` with `N > 32`).
mod serde_arr_897 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(
        bytes: &[u8; super::FALCON_PUBKEY_LEN],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        // For human-readable formats (JSON), emit hex. For binary
        // formats, emit the raw byte slice. `serde_bytes`'s convention
        // is what most byte-array fields in alloy/ethers follow.
        if s.is_human_readable() {
            let hex = format!("0x{}", hex::encode(bytes));
            hex.serialize(s)
        } else {
            bytes.as_slice().serialize(s)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<[u8; super::FALCON_PUBKEY_LEN], D::Error> {
        use serde::de::Error;
        if d.is_human_readable() {
            let s = String::deserialize(d)?;
            let hex = s.trim_start_matches("0x");
            let bytes = hex::decode(hex).map_err(D::Error::custom)?;
            if bytes.len() != super::FALCON_PUBKEY_LEN {
                return Err(D::Error::custom(format!(
                    "expected {} bytes, got {}",
                    super::FALCON_PUBKEY_LEN,
                    bytes.len()
                )));
            }
            let mut out = [0u8; super::FALCON_PUBKEY_LEN];
            out.copy_from_slice(&bytes);
            Ok(out)
        } else {
            let v = Vec::<u8>::deserialize(d)?;
            if v.len() != super::FALCON_PUBKEY_LEN {
                return Err(D::Error::custom(format!(
                    "expected {} bytes, got {}",
                    super::FALCON_PUBKEY_LEN,
                    v.len()
                )));
            }
            let mut out = [0u8; super::FALCON_PUBKEY_LEN];
            out.copy_from_slice(&v);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn pubkey_borsh_round_trip() {
        let pk = FalconPubkey::new([0x42; FALCON_PUBKEY_LEN]);
        let bytes = borsh::to_vec(&pk).unwrap();
        // Fixed-size array → no length prefix in Borsh.
        assert_eq!(bytes.len(), FALCON_PUBKEY_LEN);
        let decoded: FalconPubkey = borsh::from_slice(&bytes).unwrap();
        assert_eq!(pk, decoded);
    }

    #[test]
    fn pubkey_hex_round_trip() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let parsed = FalconPubkey::from_hex(&pk.to_hex()).unwrap();
        assert_eq!(pk, parsed);
    }

    #[test]
    fn pubkey_debug_redacts() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        let dbg = format!("{pk:?}");
        // Debug should NOT print all 897 bytes.
        assert!(dbg.len() < 100);
        assert!(dbg.contains('…'));
    }

    #[test]
    fn signature_borsh_round_trip() {
        let sig = FalconSignature::new(vec![0xCD; 666]);
        let bytes = borsh::to_vec(&sig).unwrap();
        // Vec<u8> → 4-byte LE length prefix + payload.
        assert_eq!(bytes.len(), 4 + 666);
        let decoded: FalconSignature = borsh::from_slice(&bytes).unwrap();
        assert_eq!(sig, decoded);
    }

    #[test]
    fn secret_debug_never_leaks_bytes() {
        let sk = FalconSecret::new([0xAB; FALCON_SECRET_LEN]);
        let dbg = format!("{sk:?}");
        assert!(!dbg.contains("ab"));
        assert!(dbg.contains("redacted"));
    }

    #[test]
    fn pubkey_conversion_round_trips_via_pyde_crypto() {
        let pk = FalconPubkey::new([0x7E; FALCON_PUBKEY_LEN]);
        let crypto_pk: pyde_crypto::falcon::FalconPublicKey = (&pk).into();
        let back: FalconPubkey = (&crypto_pk).into();
        assert_eq!(pk, back);
    }
}
