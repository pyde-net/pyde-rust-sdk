//! Factory (PIP-0006) child-address conformance — full replay of the
//! shared golden vectors.
//!
//! The golden fixture (`tests/fixtures/child_address.golden.json`) is
//! a verbatim copy of the canonical vector file, whose home is
//! `pyde-host/vectors/child_address.json` — regenerate there
//! (`cargo test -p pyde-host regenerate_golden_vectors -- --ignored`)
//! and re-copy; never edit the copy by hand. Every implementation
//! (engine, 4 language bindings, rust/ts SDKs, CLI) must reproduce
//! every vector byte-for-byte.
//!
//! This SDK carries the exact engine Poseidon2 (via `pyde-crypto`),
//! so the replay here is FULL: preimage assembly, the hash itself,
//! the identity-salt rule (`Salt::of` = Poseidon2 over borsh bytes,
//! including the empty-input case), and the unordered-pair sort
//! (unsigned bytewise — the sign-boundary vector catches a signed
//! comparator).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pyde_crypto::poseidon2::poseidon2_hash;
use pyde_rust_sdk::factory::{child_address, child_preimage, Salt, CHILD_PREIMAGE_LEN};
use pyde_rust_sdk::types::Address;
use serde::Deserialize;

#[derive(Deserialize)]
struct GoldenFile {
    vectors: Vec<Vector>,
}

#[derive(Deserialize)]
struct Vector {
    name: String,
    parent: String,
    template: String,
    salt: String,
    preimage: String,
    child_address: String,
    #[serde(default)]
    salt_source_borsh: Option<String>,
    #[serde(default)]
    salt_source_pair_args: Option<[String; 2]>,
}

fn golden_vectors() -> Vec<Vector> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/child_address.golden.json");
    let json = std::fs::read_to_string(path).expect("golden fixture present");
    let file: GoldenFile = serde_json::from_str(&json).expect("golden fixture parses");
    file.vectors
}

fn addr(hex_str: &str) -> Address {
    Address::from_hex(hex_str).expect("vector address is 32 bytes of hex")
}

fn bytes32(hex_str: &str) -> [u8; 32] {
    let bytes = hex::decode(hex_str).unwrap();
    bytes.try_into().expect("vector field is 32 bytes")
}

#[test]
fn golden_vectors_are_all_present() {
    assert_eq!(golden_vectors().len(), 13, "vector count drifted");
}

/// Full replay: for every vector, the preimage assembly must equal
/// the pinned `preimage` field AND hashing that preimage must equal
/// the pinned `child_address` — two independent assertions so a
/// failure localises to assembly vs hash.
#[test]
fn golden_vectors_replay_preimage_and_child_address() {
    for v in golden_vectors() {
        let parent = addr(&v.parent);
        let template = addr(&v.template);
        let salt = bytes32(&v.salt);

        // Preimage assembly, byte-for-byte.
        let preimage = child_preimage(&parent, &template, &salt);
        assert_eq!(
            hex::encode(preimage),
            v.preimage,
            "[{}] preimage assembly drifted",
            v.name
        );
        assert_eq!(preimage.len(), CHILD_PREIMAGE_LEN);

        // Hash of the *pinned* preimage bytes — proves this SDK's
        // Poseidon2 is the engine's.
        let pinned_preimage = hex::decode(&v.preimage).unwrap();
        let hashed: [u8; 32] = poseidon2_hash(&pinned_preimage).into();
        assert_eq!(
            hex::encode(hashed),
            v.child_address,
            "[{}] Poseidon2 over the pinned preimage drifted",
            v.name
        );

        // And the one-call derivation lands on the same address.
        assert_eq!(
            child_address(&parent, &template, &salt).to_hex(),
            format!("0x{}", v.child_address),
            "[{}] child_address drifted",
            v.name
        );
    }
}

/// Identity-salt vectors: `salt = Poseidon2(salt_source_borsh)`, and
/// where the typed source is reconstructible, `Salt::of` over the
/// typed value must produce both the pinned borsh bytes and the
/// pinned salt.
#[test]
fn golden_vectors_replay_identity_salts() {
    let mut replayed = 0;
    for v in golden_vectors() {
        let Some(source_hex) = &v.salt_source_borsh else {
            continue;
        };
        // Pair vectors pin their borsh field too (the sorted 64-byte
        // concat) but are replayed through Salt::of_unordered_pair in
        // the dedicated test below.
        if v.salt_source_pair_args.is_some() {
            continue;
        }
        replayed += 1;

        let source = hex::decode(source_hex).unwrap();
        let hashed: [u8; 32] = poseidon2_hash(&source).into();
        assert_eq!(
            hex::encode(hashed),
            v.salt,
            "[{}] Poseidon2(salt_source_borsh) drifted",
            v.name
        );

        // Typed replay through Salt::of where the vector names a
        // reconstructible value.
        let typed: Option<[u8; 32]> = match v.name.as_str() {
            "salt-of-unit-empty-borsh" => Some(Salt::of(&())),
            "salt-of-counter-0" => Some(Salt::of(&0u64)),
            "salt-of-counter-1" => Some(Salt::of(&1u64)),
            "salt-of-i128-neg-one" => Some(Salt::of(&-1i128)),
            "salt-of-i128-min" => Some(Salt::of(&i128::MIN)),
            "salt-of-string" => Some(Salt::of(&"amm-pool-v1".to_string())),
            "salt-of-mixed-tuple" => Some(Salt::of(&([0xCCu8; 32], 42u64, true))),
            other => panic!("new identity-salt vector {other:?} — add its typed replay"),
        };
        if let Some(typed_salt) = typed {
            assert_eq!(
                hex::encode(typed_salt),
                v.salt,
                "[{}] Salt::of over the typed value drifted",
                v.name
            );
            // The typed value's borsh bytes must be the pinned ones.
            let typed_borsh: Vec<u8> = match v.name.as_str() {
                "salt-of-unit-empty-borsh" => borsh::to_vec(&()).unwrap(),
                "salt-of-counter-0" => borsh::to_vec(&0u64).unwrap(),
                "salt-of-counter-1" => borsh::to_vec(&1u64).unwrap(),
                "salt-of-i128-neg-one" => borsh::to_vec(&-1i128).unwrap(),
                "salt-of-i128-min" => borsh::to_vec(&i128::MIN).unwrap(),
                "salt-of-string" => borsh::to_vec(&"amm-pool-v1".to_string()).unwrap(),
                "salt-of-mixed-tuple" => borsh::to_vec(&([0xCCu8; 32], 42u64, true)).unwrap(),
                _ => unreachable!(),
            };
            assert_eq!(
                hex::encode(typed_borsh),
                *source_hex,
                "[{}] borsh bytes",
                v.name
            );
        }
    }
    assert_eq!(replayed, 7, "identity-salt vector count drifted");
}

/// Pair vectors: `Salt::of_unordered_pair` from the deliberately
/// UNSORTED `salt_source_pair_args` must land on the pinned salt,
/// argument order must not matter, and a naive unsorted concat must
/// diverge — including at the 0x7f/0x80 sign boundary, where a
/// signed (i8) comparator sorts the wrong way.
#[test]
fn golden_vectors_replay_unordered_pairs() {
    let mut replayed = 0;
    for v in golden_vectors() {
        let Some([arg_a, arg_b]) = &v.salt_source_pair_args else {
            continue;
        };
        replayed += 1;

        let a = addr(arg_a);
        let b = addr(arg_b);

        // From the unsorted args as recorded.
        assert_eq!(
            hex::encode(Salt::of_unordered_pair(&a, &b)),
            v.salt,
            "[{}] of_unordered_pair(a, b) drifted",
            v.name
        );
        // Reversed args — identical by construction.
        assert_eq!(
            Salt::of_unordered_pair(&b, &a),
            Salt::of_unordered_pair(&a, &b),
            "[{}] of_unordered_pair must be order-independent",
            v.name
        );

        // A naive implementation that skips the sort hashes the args
        // in call order. Both pair vectors record their args unsorted
        // on purpose, so the naive result must diverge.
        let mut naive = Vec::with_capacity(64);
        naive.extend_from_slice(a.as_bytes());
        naive.extend_from_slice(b.as_bytes());
        let naive_salt: [u8; 32] = poseidon2_hash(&naive).into();
        assert_ne!(
            hex::encode(naive_salt),
            v.salt,
            "[{}] unsorted concat must NOT reproduce the salt",
            v.name
        );

        // The pinned borsh field is the SORTED 64-byte concat.
        let sorted = hex::decode(v.salt_source_borsh.as_ref().unwrap()).unwrap();
        let sorted_salt: [u8; 32] = poseidon2_hash(&sorted).into();
        assert_eq!(
            hex::encode(sorted_salt),
            v.salt,
            "[{}] sorted concat",
            v.name
        );
    }
    assert_eq!(
        replayed, 2,
        "expected exactly the pair + sign-boundary pair vectors"
    );
}

/// Anchor KAT, hardcoded independently of the fixture — mirrors the
/// engine's `child_address_kat_pins_derivation`. If the fixture file
/// is ever regenerated wrongly, this still fails.
#[test]
fn engine_kat_anchor_hardcoded() {
    let parent = Address::new([0x11; 32]);
    let template = Address::new([0x22; 32]);
    let salt = [0x33; 32];
    assert_eq!(
        child_address(&parent, &template, &salt).to_hex(),
        "0x354ab9a58e3fb76b484390a2ef277594042e12fd0b74343e5bf34dba492f3dfe"
    );
}
