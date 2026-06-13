//! 32-byte Pyde addresses.
//!
//! Every Pyde address is a 32-byte Poseidon2 digest. Addresses are
//! never truncated to 20 bytes (the EVM convention) — Pyde's address
//! space is the full Poseidon2 output. See
//! [Chapter 11 §11.2](https://book.pyde.network/chapters/11-account-model#112-address-derivation).
//!
//! Four derivation paths, each disjoint by input length and/or domain
//! separator:
//!
//! | Path        | Input                                                 |
//! |-------------|-------------------------------------------------------|
//! | EOA         | `falcon_pubkey_bytes` (897 B)                         |
//! | CREATE      | `deployer (32 B) ‖ nonce_le (8 B)`                    |
//! | CREATE2     | `0xFF ‖ deployer ‖ salt (32 B) ‖ code_hash (32 B)`    |
//! | Contract    | `"pyde-contract:" ‖ name_bytes`                       |
//! | System      | `name_bytes` (genesis only)                           |
//!
//! The SDK exposes the derivations every dapp / wallet needs to know
//! its address *before* talking to the chain — i.e. EOA, CREATE,
//! CREATE2, and contract-name. `system_address` is genesis-only and
//! left to the engine.

use crate::error::SdkError;
use borsh::{BorshDeserialize, BorshSerialize};
use pyde_crypto::poseidon2::poseidon2_hash;
use serde::{Deserialize, Serialize};
use std::fmt;

use super::hash::Poseidon2Hash;
use super::FalconPubkey;

/// Length of a Pyde address, in bytes.
pub const ADDRESS_LEN: usize = 32;

/// EIP-1014-equivalent prefix for CREATE2 — keeps CREATE2 inputs
/// (`0xFF ‖ …`, 97 B) structurally disjoint from CREATE inputs
/// (40 B, no prefix byte).
pub const CREATE2_PREFIX: u8 = 0xFF;

/// Domain-separator prefix for name-based contract addresses. Pairs
/// with [`Address::from_contract_name`] to keep contract addresses
/// disjoint from raw system addresses even when the name string
/// happens to match.
pub const CONTRACT_ADDRESS_PREFIX: &[u8] = b"pyde-contract:";

/// A 32-byte Pyde address.
///
/// Wire layout is a fixed-size Borsh array — no length prefix, no
/// padding. Cheap to copy and pass around (it's a `[u8; 32]` under
/// the hood). The `Display` and `Debug` impls render the canonical
/// `0x`-prefixed lower-case hex form used by RPC payloads and
/// block explorers.
#[derive(
    Clone,
    Copy,
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
pub struct Address(pub [u8; ADDRESS_LEN]);

impl Address {
    /// The all-zero address — sentinel for "no recipient" (a `Deploy`
    /// tx, or an envelope-style tx that targets the protocol rather
    /// than an account). Never a valid user account.
    pub const ZERO: Self = Self([0u8; ADDRESS_LEN]);

    /// Construct from raw bytes. Callers are responsible for ensuring
    /// the bytes are a legitimate Poseidon2 digest — for safe
    /// derivation use [`Address::from_pubkey`], [`Address::create`],
    /// [`Address::create2`], or [`Address::from_contract_name`].
    #[must_use]
    pub const fn new(bytes: [u8; ADDRESS_LEN]) -> Self {
        Self(bytes)
    }

    /// View the underlying bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; ADDRESS_LEN] {
        &self.0
    }

    /// Whether this is [`Address::ZERO`].
    #[must_use]
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; ADDRESS_LEN]
    }

    /// Lower-case hex representation, with the `0x` prefix.
    ///
    /// Canonical text form used everywhere on the wire — JSON-RPC
    /// payloads, block explorers, log filters. Mirrors `Display`.
    #[must_use]
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.0))
    }

    /// Parse a `0x`-prefixed (or bare) 64-character hex string into
    /// an address.
    ///
    /// # Errors
    /// Returns [`SdkError::InvalidAddress`] if the input isn't exactly
    /// 64 hex characters after stripping an optional `0x` prefix, or
    /// if any character isn't a valid hex digit.
    pub fn from_hex(s: &str) -> Result<Self, SdkError> {
        let hex_str = s.trim_start_matches("0x");
        if hex_str.len() != 64 {
            return Err(SdkError::InvalidAddress(format!(
                "expected 64 hex chars, got {}",
                hex_str.len()
            )));
        }
        let bytes =
            hex::decode(hex_str).map_err(|e| SdkError::InvalidAddress(format!("bad hex: {e}")))?;
        let mut out = [0u8; ADDRESS_LEN];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    /// EOA derivation: `Poseidon2(falcon_pubkey_bytes)`.
    ///
    /// The wallet runs this exactly once at keygen to learn its
    /// on-chain address. Output is the full 32-byte digest — no
    /// truncation per [`address-naming-collision`][collision] design.
    ///
    /// [collision]: https://book.pyde.network/companion/design-memos
    #[must_use]
    pub fn from_pubkey(pubkey: &FalconPubkey) -> Self {
        let bytes: [u8; 32] = poseidon2_hash(pubkey.as_bytes()).into();
        Self(bytes)
    }

    /// CREATE derivation: `Poseidon2(deployer ‖ nonce_le)`.
    ///
    /// `nonce` is encoded as 8 little-endian bytes. Matches the
    /// Ethereum-style `(deployer, nonce)` scheme — useful for wallets
    /// that want to predict a deploy address before broadcasting the
    /// deploy tx.
    #[must_use]
    pub fn create(deployer: &Address, nonce: u64) -> Self {
        let mut input = [0u8; 40];
        input[..32].copy_from_slice(deployer.as_bytes());
        input[32..40].copy_from_slice(&nonce.to_le_bytes());
        let bytes: [u8; 32] = poseidon2_hash(&input).into();
        Self(bytes)
    }

    /// CREATE2 derivation:
    /// `Poseidon2(0xFF ‖ deployer ‖ salt ‖ code_hash)`.
    ///
    /// `code_hash` is the Poseidon2 hash of the contract's init
    /// bytecode. Same `(deployer, salt, code_hash)` always derives
    /// the same address — useful for counterfactual deployments.
    #[must_use]
    pub fn create2(deployer: &Address, salt: &[u8; 32], code_hash: &Poseidon2Hash) -> Self {
        let mut input = [0u8; 97];
        input[0] = CREATE2_PREFIX;
        input[1..33].copy_from_slice(deployer.as_bytes());
        input[33..65].copy_from_slice(salt);
        input[65..97].copy_from_slice(code_hash.as_bytes());
        let bytes: [u8; 32] = poseidon2_hash(&input).into();
        Self(bytes)
    }

    /// Name-based contract address:
    /// `Poseidon2("pyde-contract:" ‖ name_bytes)`.
    ///
    /// Pyde's primary deploy path registers contracts under an
    /// ENS-style name; the address is a deterministic function of
    /// the name string. Wallets and indexers can resolve
    /// `name → address` without a chain query. Uniqueness is enforced
    /// chain-side at deploy time.
    #[must_use]
    pub fn from_contract_name(name: &str) -> Self {
        let mut input = Vec::with_capacity(CONTRACT_ADDRESS_PREFIX.len() + name.len());
        input.extend_from_slice(CONTRACT_ADDRESS_PREFIX);
        input.extend_from_slice(name.as_bytes());
        let bytes: [u8; 32] = poseidon2_hash(&input).into();
        Self(bytes)
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.to_hex())
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl From<[u8; ADDRESS_LEN]> for Address {
    fn from(bytes: [u8; ADDRESS_LEN]) -> Self {
        Self(bytes)
    }
}

impl From<Address> for [u8; ADDRESS_LEN] {
    fn from(addr: Address) -> Self {
        addr.0
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl std::str::FromStr for Address {
    type Err = SdkError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::FALCON_PUBKEY_LEN;

    #[test]
    fn zero_address_is_all_zeros() {
        assert!(Address::ZERO.is_zero());
        assert_eq!(Address::ZERO.0, [0u8; ADDRESS_LEN]);
    }

    #[test]
    fn display_uses_0x_prefix_lowercase() {
        let addr = Address::new([0xAB; ADDRESS_LEN]);
        let rendered = format!("{addr}");
        assert!(rendered.starts_with("0x"));
        assert_eq!(rendered.len(), 2 + 64);
        assert_eq!(rendered, format!("0x{}", "ab".repeat(ADDRESS_LEN)));
    }

    #[test]
    fn hex_round_trip() {
        let addr = Address::new([0x5A; ADDRESS_LEN]);
        let parsed = Address::from_hex(&addr.to_hex()).unwrap();
        assert_eq!(addr, parsed);
    }

    #[test]
    fn hex_accepts_bare_and_prefixed() {
        let bare = "a".repeat(64);
        let prefixed = format!("0x{bare}");
        let a = Address::from_hex(&bare).unwrap();
        let b = Address::from_hex(&prefixed).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hex_rejects_wrong_length() {
        assert!(Address::from_hex("0xabcd").is_err());
    }

    #[test]
    fn hex_rejects_non_hex() {
        let bogus = "z".repeat(64);
        assert!(Address::from_hex(&bogus).is_err());
    }

    #[test]
    fn from_pubkey_is_deterministic() {
        let pk = FalconPubkey::new([0xAB; FALCON_PUBKEY_LEN]);
        assert_eq!(Address::from_pubkey(&pk), Address::from_pubkey(&pk));
    }

    #[test]
    fn from_pubkey_distinguishes_distinct_pubkeys() {
        let a = FalconPubkey::new([0x01; FALCON_PUBKEY_LEN]);
        let b = FalconPubkey::new([0x02; FALCON_PUBKEY_LEN]);
        assert_ne!(Address::from_pubkey(&a), Address::from_pubkey(&b));
    }

    #[test]
    fn create_address_changes_with_nonce() {
        let deployer = Address::new([0x42; 32]);
        assert_ne!(Address::create(&deployer, 0), Address::create(&deployer, 1));
    }

    #[test]
    fn create2_disjoint_from_create() {
        let deployer = Address::new([0x42; 32]);
        let create_addr = Address::create(&deployer, 0);
        let create2_addr = Address::create2(&deployer, &[0u8; 32], &Poseidon2Hash::zero());
        assert_ne!(create_addr, create2_addr);
    }

    #[test]
    fn contract_name_address_disjoint_from_create() {
        let deployer = Address::new([0x42; 32]);
        assert_ne!(
            Address::from_contract_name("counter"),
            Address::create(&deployer, 0)
        );
    }

    #[test]
    fn borsh_round_trip() {
        let addr = Address::new([0x77; ADDRESS_LEN]);
        let bytes = borsh::to_vec(&addr).unwrap();
        // Fixed-size array → no length prefix.
        assert_eq!(bytes.len(), ADDRESS_LEN);
        let decoded: Address = borsh::from_slice(&bytes).unwrap();
        assert_eq!(addr, decoded);
    }

    #[test]
    fn from_str_works() {
        use std::str::FromStr;
        let s = "0xaabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";
        let addr = Address::from_str(s).unwrap();
        assert_eq!(addr.to_hex(), s);
    }
}
