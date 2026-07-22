//! Factory-pattern off-chain surface.
//!
//! A contract (the FACTORY) instantiates a child contract by
//! reference to a deployed TEMPLATE via the `pyde::instantiate` host
//! function. The child's address is a pure, fixed-width function of
//! `(parent, template, salt)` — so wallets, scripts, and indexers can
//! compute a child's address *before it exists* (counterfactual
//! instantiation), without touching the chain.
//!
//! This module owns the three pieces every off-chain consumer needs:
//!
//! - [`child_address`] / [`child_preimage`] — the canonical
//!   derivation, byte-identical to
//!   `engine/crates/account/src/address.rs::child_address`.
//! - [`Salt`] — helpers for building the 32-byte salt the same way
//!   contracts do (identity hashes via Borsh, unordered address
//!   pairs for AMM-style markets).
//! - [`Instantiated`] — decoder for the provenance event the engine
//!   emits on every successful `pyde::instantiate`.
//!
//! Cross-implementation conformance is pinned by shared golden
//! vectors (canonical home: `pyde-host/vectors/child_address.json`;
//! replayed here in `tests/factory_child_address.rs`).

use borsh::BorshSerialize;
use pyde_crypto::poseidon2::poseidon2_hash;

use crate::error::SdkError;
use crate::types::{Address, Event};

// ── Child-address derivation ───────────────────────────────────

/// Domain-separator prefix for factory-instantiated child addresses.
/// Exactly 11 bytes; keeps [`child_address`] disjoint
/// from every other address family (EOA raw-pubkey,
/// `"pyde-contract:"` names, raw system names). Mirrors the engine's
/// `CHILD_ADDRESS_PREFIX`.
pub const CHILD_ADDRESS_PREFIX: &[u8; 11] = b"pyde-child:";

/// Length of the [`child_preimage`], in bytes:
/// `11 (prefix) + 32 (parent) + 32 (template) + 32 (salt)`.
pub const CHILD_PREIMAGE_LEN: usize = 107;

/// Assemble the 107-byte child-address preimage:
/// `"pyde-child:" ‖ parent ‖ template ‖ salt`.
///
/// Fixed-width, no length prefixes, no separators (required for hash
/// injectivity). Exposed for tooling that wants the raw bytes — e.g.
/// to feed an on-chain `pyde::hash_poseidon2` call, or to diff
/// against a conformance vector. Most callers want [`child_address`]
/// directly.
#[must_use]
pub fn child_preimage(
    parent: &Address,
    template: &Address,
    salt: &[u8; 32],
) -> [u8; CHILD_PREIMAGE_LEN] {
    let mut preimage = [0u8; CHILD_PREIMAGE_LEN];
    preimage[..11].copy_from_slice(CHILD_ADDRESS_PREFIX);
    preimage[11..43].copy_from_slice(parent.as_bytes());
    preimage[43..75].copy_from_slice(template.as_bytes());
    preimage[75..107].copy_from_slice(salt);
    preimage
}

/// Factory child address:
/// `Poseidon2("pyde-child:" ‖ parent ‖ template ‖ salt)`.
///
/// - `parent` — the factory's own address. The engine takes it from
///   the executing frame, so a contract can only mint into its OWN
///   namespace; cross-namespace minting is cryptographically
///   impossible.
/// - `template` — the address of the referenced template contract
///   (an address, not a code hash — under v1's immutable code the
///   address commits to the code just as a code hash would).
/// - `salt` — an opaque 32-byte value the caller derives; see
///   [`Salt`] for the canonical constructions. Salt keeps addresses
///   predictable — deliberately NOT a sequence counter the chain
///   assigns.
///
/// Same `(parent, template, salt)` → same address, so anyone can
/// compute a child's address before it exists. Byte-identical to
/// `engine/crates/account/src/address.rs::child_address`; pinned by
/// the shared conformance vectors in
/// `tests/factory_child_address.rs`.
#[must_use]
pub fn child_address(parent: &Address, template: &Address, salt: &[u8; 32]) -> Address {
    let bytes: [u8; 32] = poseidon2_hash(&child_preimage(parent, template, salt)).into();
    Address::new(bytes)
}

// ── Salt helpers ───────────────────────────────────────────────

/// Namespace for the canonical salt constructions.
///
/// The salt is what makes a child address *meaningful*: derive it
/// from the identity the child represents and the address becomes a
/// deterministic function of that identity — any party can recompute
/// it offline. These helpers mirror what factory contracts do
/// on-chain, so the off-chain computation lands on the same 32
/// bytes.
pub struct Salt;

impl Salt {
    /// Identity salt: `Poseidon2(borsh(value))`.
    ///
    /// Hash the Borsh encoding of any typed value — a `u64` counter,
    /// a name string, a config tuple — into a 32-byte salt. Borsh is
    /// canonical (one value, one encoding), so equal values always
    /// produce equal salts across every SDK and the engine.
    ///
    /// # Panics
    /// Panics if `value`'s [`BorshSerialize`] impl returns an error.
    /// Unreachable for derived impls — serialising into a `Vec` is
    /// infallible — so this only fires on a hand-written impl that
    /// manufactures an error. Deriving a salt from partial bytes
    /// would silently address a *different* child, so refusing
    /// loudly is the safe behaviour.
    #[must_use]
    #[allow(clippy::expect_used)]
    pub fn of<T: BorshSerialize>(value: &T) -> [u8; 32] {
        let bytes = borsh::to_vec(value).expect("borsh encode into Vec cannot fail");
        poseidon2_hash(&bytes).into()
    }

    /// Unordered-pair salt: sort the two addresses ascending
    /// BYTEWISE (unsigned lexicographic — `0x7f…` sorts before
    /// `0x80…`), concatenate the raw 64 bytes (no framing), hash.
    ///
    /// The canonical construction for symmetric markets — an AMM
    /// pool over `(token_a, token_b)` gets the same child address no
    /// matter which order the caller lists the tokens.
    #[must_use]
    pub fn of_unordered_pair(a: &Address, b: &Address) -> [u8; 32] {
        // `[u8; 32]: Ord` is unsigned bytewise lexicographic —
        // exactly the required sort. A signed (i8) comparator would
        // swap `0x7f…`/`0x80…`; the sign-boundary conformance vector
        // exists to catch that.
        let (lo, hi) = if a.as_bytes() <= b.as_bytes() {
            (a, b)
        } else {
            (b, a)
        };
        let mut buf = [0u8; 64];
        buf[..32].copy_from_slice(lo.as_bytes());
        buf[32..].copy_from_slice(hi.as_bytes());
        poseidon2_hash(&buf).into()
    }
}

// ── Instantiated event ─────────────────────────────────────────

/// Topic-0 of the `Instantiated` provenance event:
/// `Blake3("pyde.Instantiated")`.
///
/// The engine emits one `Instantiated` event from the parent factory
/// on every successful `pyde::instantiate`, *before* any events the
/// child's constructor emitted. Filter logs on this topic to index
/// every child a factory has ever minted. Pinned by a KAT test so
/// the constant can never drift from the preimage string.
pub const INSTANTIATED_TOPIC: [u8; 32] = [
    0x62, 0x2a, 0x0a, 0x9e, 0x1e, 0x2b, 0x48, 0x72, 0x88, 0x90, 0x4a, 0x22, 0xb1, 0x81, 0x74, 0xd6,
    0xe4, 0x5b, 0x37, 0x49, 0xc7, 0x56, 0xa9, 0x42, 0x09, 0xef, 0x9a, 0x9c, 0xf7, 0x68, 0x84, 0x7a,
];

/// Byte length of an `Instantiated` event's `data` payload:
/// `parent (32) ‖ salt (32) ‖ value LE u128 (16)`.
const INSTANTIATED_DATA_LEN: usize = 80;

/// Decoded `Instantiated` provenance event.
///
/// Wire layout (frozen with the `pyde::instantiate` host fn):
///
/// - emitter (`contract_addr`) — the parent factory,
/// - `topics[0]` — [`INSTANTIATED_TOPIC`],
/// - `topics[1]` — the child's address (indexed: filter by child),
/// - `topics[2]` — the template's address (indexed: filter by
///   template),
/// - `data` — `parent (32) ‖ salt (32) ‖ value LE u128 (16)`,
///   exactly 80 bytes, no framing.
///
/// `value` is the endowment in quanta the factory forwarded to the
/// child at instantiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instantiated {
    /// The factory that instantiated the child (also the event's
    /// emitter).
    pub parent: Address,
    /// The template contract the child was instantiated from.
    pub template: Address,
    /// The 32-byte salt the factory passed to `pyde::instantiate`.
    /// Replay [`child_address`]`(parent, template, salt)` to verify
    /// it derives `child`.
    pub salt: [u8; 32],
    /// The freshly instantiated child's address.
    pub child: Address,
    /// Endowment forwarded to the child at instantiation, in quanta.
    pub value: u128,
}

impl Instantiated {
    /// Decode an [`Event`] log into a typed [`Instantiated`].
    ///
    /// # Errors
    /// [`SdkError::InvalidResponse`] if the event isn't a well-formed
    /// `Instantiated`: wrong topic count (must be exactly 3), a topic
    /// that isn't 32 bytes of hex, `topics[0]` ≠
    /// [`INSTANTIATED_TOPIC`], `data` ≠ exactly 80 bytes, or an
    /// emitter that doesn't match the parent recorded in `data`
    /// (the engine always emits the marker from the parent frame —
    /// a mismatch means the log didn't come from the canonical
    /// instantiate path).
    pub fn decode(event: &Event) -> Result<Self, SdkError> {
        if event.topics.len() != 3 {
            return Err(SdkError::InvalidResponse(format!(
                "Instantiated expects 3 topics; got {}",
                event.topics.len()
            )));
        }
        let topic0 = decode_topic(&event.topics[0])?;
        if topic0 != INSTANTIATED_TOPIC {
            return Err(SdkError::InvalidResponse(format!(
                "topic 0 is not Blake3(\"pyde.Instantiated\"): 0x{}",
                hex::encode(topic0)
            )));
        }
        let child = Address::new(decode_topic(&event.topics[1])?);
        let template = Address::new(decode_topic(&event.topics[2])?);

        let data = hex::decode(event.data.trim_start_matches("0x"))
            .map_err(|e| SdkError::InvalidResponse(format!("Instantiated data hex: {e}")))?;
        if data.len() != INSTANTIATED_DATA_LEN {
            return Err(SdkError::InvalidResponse(format!(
                "Instantiated data must be {INSTANTIATED_DATA_LEN} bytes; got {}",
                data.len()
            )));
        }
        let mut parent = [0u8; 32];
        parent.copy_from_slice(&data[..32]);
        let parent = Address::new(parent);
        let mut salt = [0u8; 32];
        salt.copy_from_slice(&data[32..64]);
        let mut value_le = [0u8; 16];
        value_le.copy_from_slice(&data[64..80]);
        let value = u128::from_le_bytes(value_le);

        let emitter = Address::from_hex(&event.contract_addr)?;
        if emitter != parent {
            return Err(SdkError::InvalidResponse(format!(
                "Instantiated emitter {emitter} does not match parent {parent}"
            )));
        }

        Ok(Self {
            parent,
            template,
            salt,
            child,
            value,
        })
    }
}

impl TryFrom<&Event> for Instantiated {
    type Error = SdkError;

    fn try_from(event: &Event) -> Result<Self, Self::Error> {
        Self::decode(event)
    }
}

/// Decode one hex topic string into a 32-byte word.
fn decode_topic(topic: &str) -> Result<[u8; 32], SdkError> {
    let bytes = hex::decode(topic.trim_start_matches("0x"))
        .map_err(|e| SdkError::InvalidResponse(format!("topic hex: {e}")))?;
    let arr: [u8; 32] = bytes.try_into().map_err(|b: Vec<u8>| {
        SdkError::InvalidResponse(format!("topic must be 32 bytes; got {}", b.len()))
    })?;
    Ok(arr)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    // ── Derivation ─────────────────────────────────────────────

    #[test]
    fn child_address_kat_pins_derivation() {
        // Anchor KAT — hardcoded independently of the golden-vector
        // fixture; mirrors the engine's pinned
        // `child_address_kat_pins_derivation`.
        let parent = Address::new([0x11; 32]);
        let template = Address::new([0x22; 32]);
        let salt = [0x33; 32];
        assert_eq!(
            child_address(&parent, &template, &salt).to_hex(),
            "0x354ab9a58e3fb76b484390a2ef277594042e12fd0b74343e5bf34dba492f3dfe"
        );
    }

    #[test]
    fn preimage_layout_is_fixed_width() {
        let parent = Address::new([0xAA; 32]);
        let template = Address::new([0xBB; 32]);
        let salt = [0xCC; 32];
        let preimage = child_preimage(&parent, &template, &salt);
        // Prefix: 11 ASCII bytes "pyde-child:".
        assert_eq!(
            &preimage[..11],
            &[0x70, 0x79, 0x64, 0x65, 0x2D, 0x63, 0x68, 0x69, 0x6C, 0x64, 0x3A]
        );
        assert_eq!(&preimage[11..43], &[0xAA; 32]);
        assert_eq!(&preimage[43..75], &[0xBB; 32]);
        assert_eq!(&preimage[75..107], &[0xCC; 32]);
    }

    #[test]
    fn child_address_hashes_the_preimage() {
        let parent = Address::new([0x01; 32]);
        let template = Address::new([0x02; 32]);
        let salt = [0x03; 32];
        let via_preimage: [u8; 32] =
            poseidon2_hash(&child_preimage(&parent, &template, &salt)).into();
        assert_eq!(child_address(&parent, &template, &salt).0, via_preimage);
    }

    #[test]
    fn child_address_distinguishes_each_input() {
        let p = Address::new([0x01; 32]);
        let t = Address::new([0x02; 32]);
        let s = [0x03; 32];
        let base = child_address(&p, &t, &s);
        assert_ne!(base, child_address(&Address::new([0x04; 32]), &t, &s));
        assert_ne!(base, child_address(&p, &Address::new([0x04; 32]), &s));
        assert_ne!(base, child_address(&p, &t, &[0x04; 32]));
        // Swapping parent/template must change the address —
        // fixed-width fields, not a commutative mix.
        assert_ne!(base, child_address(&t, &p, &s));
    }

    // ── Salt ───────────────────────────────────────────────────

    #[test]
    fn salt_of_is_deterministic_and_typed() {
        assert_eq!(Salt::of(&7u64), Salt::of(&7u64));
        assert_ne!(Salt::of(&7u64), Salt::of(&8u64));
        // Borsh is typed on width: 7u64 (8 bytes) ≠ 7u32 (4 bytes).
        assert_ne!(Salt::of(&7u64), Salt::of(&7u32));
    }

    #[test]
    fn salt_of_unordered_pair_is_order_independent() {
        let a = Address::new([0x0A; 32]);
        let b = Address::new([0x0B; 32]);
        assert_eq!(
            Salt::of_unordered_pair(&a, &b),
            Salt::of_unordered_pair(&b, &a)
        );
        assert_eq!(
            Salt::of_unordered_pair(&a, &a),
            Salt::of_unordered_pair(&a, &a)
        );
        assert_ne!(
            Salt::of_unordered_pair(&a, &b),
            Salt::of_unordered_pair(&a, &a)
        );
    }

    #[test]
    fn salt_of_unordered_pair_sorts_unsigned_at_sign_boundary() {
        // 0x7f… must sort BEFORE 0x80… (unsigned bytewise). A signed
        // comparator reverses them.
        let lo = Address::new([0x7F; 32]);
        let hi = Address::new([0x80; 32]);
        let mut sorted = [0u8; 64];
        sorted[..32].copy_from_slice(&[0x7F; 32]);
        sorted[32..].copy_from_slice(&[0x80; 32]);
        let expected: [u8; 32] = poseidon2_hash(&sorted).into();
        assert_eq!(Salt::of_unordered_pair(&hi, &lo), expected);
        assert_eq!(Salt::of_unordered_pair(&lo, &hi), expected);
    }

    // ── Instantiated topic ─────────────────────────────────────

    #[test]
    fn instantiated_topic_is_pinned() {
        // The indexer-facing event ABI: Blake3("pyde.Instantiated").
        assert_eq!(
            INSTANTIATED_TOPIC,
            *blake3::hash(b"pyde.Instantiated").as_bytes()
        );
    }

    // ── Instantiated decoder ───────────────────────────────────

    fn sample_event() -> (Event, Instantiated) {
        let parent = Address::new([0xA1; 32]);
        let template = Address::new([0xB2; 32]);
        let salt = [0xC3; 32];
        let child = child_address(&parent, &template, &salt);
        let value: u128 = 1_500_000_000; // 1.5 PYDE in quanta

        let mut data = Vec::with_capacity(80);
        data.extend_from_slice(parent.as_bytes());
        data.extend_from_slice(&salt);
        data.extend_from_slice(&value.to_le_bytes());

        let event = Event {
            wave_id: "0x2a".into(),
            tx_index: "0x0".into(),
            event_index: "0x0".into(),
            contract_addr: parent.to_hex(),
            topics: vec![
                format!("0x{}", hex::encode(INSTANTIATED_TOPIC)),
                child.to_hex(),
                template.to_hex(),
            ],
            data: format!("0x{}", hex::encode(&data)),
        };
        let expected = Instantiated {
            parent,
            template,
            salt,
            child,
            value,
        };
        (event, expected)
    }

    #[test]
    fn decode_round_trips_every_field() {
        let (event, expected) = sample_event();
        let decoded = Instantiated::decode(&event).unwrap();
        assert_eq!(decoded, expected);
        // The salt in the event re-derives the child in the topics.
        assert_eq!(
            child_address(&decoded.parent, &decoded.template, &decoded.salt),
            decoded.child
        );
        // TryFrom mirrors decode.
        let via_try: Instantiated = (&event).try_into().unwrap();
        assert_eq!(via_try, expected);
    }

    #[test]
    fn decode_rejects_wrong_topic0() {
        let (mut event, _) = sample_event();
        event.topics[0] = format!("0x{}", hex::encode([0x99u8; 32]));
        let err = Instantiated::decode(&event).unwrap_err();
        assert!(matches!(err, SdkError::InvalidResponse(_)));
        assert!(err.to_string().contains("topic 0"));
    }

    #[test]
    fn decode_rejects_wrong_topic_count() {
        let (mut event, _) = sample_event();
        event.topics.pop();
        assert!(matches!(
            Instantiated::decode(&event),
            Err(SdkError::InvalidResponse(_))
        ));

        let (mut event, _) = sample_event();
        event.topics.push(format!("0x{}", hex::encode([0u8; 32])));
        assert!(matches!(
            Instantiated::decode(&event),
            Err(SdkError::InvalidResponse(_))
        ));

        let (mut event, _) = sample_event();
        event.topics.clear();
        assert!(matches!(
            Instantiated::decode(&event),
            Err(SdkError::InvalidResponse(_))
        ));
    }

    #[test]
    fn decode_rejects_short_data() {
        let (mut event, _) = sample_event();
        // Truncate to 79 bytes — one short of parent‖salt‖value.
        let bytes = hex::decode(event.data.trim_start_matches("0x")).unwrap();
        event.data = format!("0x{}", hex::encode(&bytes[..79]));
        let err = Instantiated::decode(&event).unwrap_err();
        assert!(err.to_string().contains("80 bytes"));
    }

    #[test]
    fn decode_rejects_oversized_data() {
        let (mut event, _) = sample_event();
        // 80 bytes exactly, no framing — 81 must be rejected too.
        event.data.push_str("00");
        assert!(matches!(
            Instantiated::decode(&event),
            Err(SdkError::InvalidResponse(_))
        ));
    }

    #[test]
    fn decode_rejects_non_32_byte_topic() {
        let (mut event, _) = sample_event();
        event.topics[1] = "0xabcd".into();
        let err = Instantiated::decode(&event).unwrap_err();
        assert!(err.to_string().contains("32 bytes"));
    }

    #[test]
    fn decode_rejects_emitter_parent_mismatch() {
        let (mut event, _) = sample_event();
        event.contract_addr = Address::new([0xEE; 32]).to_hex();
        let err = Instantiated::decode(&event).unwrap_err();
        assert!(err.to_string().contains("does not match parent"));
    }
}
