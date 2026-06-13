//! ABI parsing — extract the `pyde.abi` custom section from a
//! WASM binary and Borsh-decode it into a typed [`ContractAbi`].
//!
//! The chain stores only the `.wasm` bytes; the ABI travels with
//! the code in a custom section named `"pyde.abi"`. This module
//! gives wallets, indexers, and dapps the same parsing primitive
//! the chain uses at deploy time.
//!
//! ```ignore
//! use pyde_rust_sdk::abi::extract_abi;
//!
//! let wasm_bytes = std::fs::read("./build/my_contract.wasm")?;
//! let abi = extract_abi(&wasm_bytes)?;
//! println!("contract: {} v{}", abi.name, abi.version);
//! for f in &abi.functions {
//!     println!("  fn {}({} params)", f.name, f.params.len());
//! }
//! ```

use wasmparser::{Parser, Payload};

use crate::error::SdkError;
use crate::types::ContractAbi;

/// Custom-section name carrying the Borsh-encoded [`ContractAbi`].
/// Locked at v1 mainnet — renaming requires an ABI version bump.
pub const PYDE_ABI_SECTION_NAME: &str = "pyde.abi";

/// Maximum [`ContractAbi::pyde_abi_version`] the SDK can decode
/// today. Matches `engine/crates/wasm-exec/src/deploy.rs`'s
/// `MAX_SUPPORTED_ABI_VERSION` — bump in lockstep as the engine
/// adds new ABI versions.
pub const MAX_SUPPORTED_ABI_VERSION: u32 = ContractAbi::MAX_SUPPORTED;

/// Extract the `pyde.abi` custom section from `wasm` and Borsh-
/// decode it.
///
/// # Errors
/// - [`SdkError::InvalidArgument`] if no `pyde.abi` section is
///   present (contract was not built with `otigen`).
/// - [`SdkError::InvalidArgument`] if more than one `pyde.abi`
///   section is present (binary is malformed).
/// - [`SdkError::InvalidArgument`] if the binary's section
///   structure is malformed (wasmparser rejected the payload
///   stream).
/// - [`SdkError::InvalidArgument`] if the Borsh decode fails (the
///   bytes are corrupted or the binary was built against a schema
///   version this SDK doesn't understand).
/// - [`SdkError::InvalidArgument`] if `pyde_abi_version` exceeds
///   [`MAX_SUPPORTED_ABI_VERSION`].
pub fn extract_abi(wasm: &[u8]) -> Result<ContractAbi, SdkError> {
    let bytes = extract_abi_section(wasm)?;
    let abi: ContractAbi = borsh::from_slice(bytes)
        .map_err(|e| SdkError::InvalidArgument(format!("pyde.abi Borsh decode: {e}")))?;
    if abi.pyde_abi_version > MAX_SUPPORTED_ABI_VERSION {
        return Err(SdkError::InvalidArgument(format!(
            "unsupported ABI version 0x{:08X}; SDK max is 0x{:08X}",
            abi.pyde_abi_version, MAX_SUPPORTED_ABI_VERSION
        )));
    }
    Ok(abi)
}

/// Locate the `pyde.abi` custom section bytes (without decoding).
///
/// Useful for advanced callers that want to Borsh-decode the bytes
/// themselves.
///
/// # Errors
/// Same as [`extract_abi`] minus the Borsh-decode + version check.
pub fn extract_abi_section(wasm: &[u8]) -> Result<&[u8], SdkError> {
    let mut found: Option<&[u8]> = None;
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|e| SdkError::InvalidArgument(format!("wasm parse: {e}")))?;
        if let Payload::CustomSection(section) = payload {
            if section.name() == PYDE_ABI_SECTION_NAME {
                if found.is_some() {
                    return Err(SdkError::InvalidArgument(
                        "duplicate pyde.abi custom sections".into(),
                    ));
                }
                found = Some(section.data());
            }
        }
    }
    found.ok_or_else(|| {
        SdkError::InvalidArgument(
            "missing pyde.abi custom section — contract was not built with otigen".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::types::{
        ContractType, FunctionAbi, FunctionAttrs, ParamAbi, ParamType, StateSchema,
    };

    /// Build a tiny WASM module that embeds a `pyde.abi` custom
    /// section with the provided payload bytes. Hand-assembled so
    /// the test doesn't depend on a WASM toolchain.
    fn wasm_with_abi(abi_payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\0asm");
        out.extend_from_slice(&1u32.to_le_bytes());
        let name = PYDE_ABI_SECTION_NAME.as_bytes();
        let mut payload = Vec::new();
        encode_uleb128(&mut payload, name.len() as u64);
        payload.extend_from_slice(name);
        payload.extend_from_slice(abi_payload);
        out.push(0x00);
        encode_uleb128(&mut out, payload.len() as u64);
        out.extend_from_slice(&payload);
        out
    }

    fn encode_uleb128(out: &mut Vec<u8>, mut value: u64) {
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn sample_abi() -> ContractAbi {
        ContractAbi {
            pyde_abi_version: ContractAbi::V1_2,
            contract_type: ContractType::Contract,
            functions: vec![FunctionAbi {
                selector: [0x11, 0x22, 0x33, 0x44],
                name: "hello".into(),
                attrs: FunctionAttrs::from_bits(FunctionAttrs::ENTRY),
                params: vec![ParamAbi {
                    name: "n".into(),
                    ty: ParamType::U64,
                }],
                returns: Some(ParamType::U64),
            }],
            state_schema_hash: [0u8; 32],
            constructor_index: None,
            fallback_index: None,
            receive_index: None,
            name: "Sample".into(),
            version: "0.1.0".into(),
            events: vec![],
            parachain_imports: vec![],
            state_schema: StateSchema::empty(),
            types: vec![],
        }
    }

    #[test]
    fn extract_abi_round_trip() {
        let abi = sample_abi();
        let bytes = borsh::to_vec(&abi).unwrap();
        let wasm = wasm_with_abi(&bytes);
        let decoded = extract_abi(&wasm).unwrap();
        assert_eq!(decoded, abi);
    }

    #[test]
    fn extract_abi_rejects_missing_section() {
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm");
        wasm.extend_from_slice(&1u32.to_le_bytes());
        let err = extract_abi(&wasm).unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(msg) if msg.contains("missing")));
    }

    #[test]
    fn extract_abi_rejects_duplicate_section() {
        let abi = sample_abi();
        let bytes = borsh::to_vec(&abi).unwrap();
        let mut wasm = Vec::new();
        wasm.extend_from_slice(b"\0asm");
        wasm.extend_from_slice(&1u32.to_le_bytes());
        for _ in 0..2 {
            let name = PYDE_ABI_SECTION_NAME.as_bytes();
            let mut payload = Vec::new();
            encode_uleb128(&mut payload, name.len() as u64);
            payload.extend_from_slice(name);
            payload.extend_from_slice(&bytes);
            wasm.push(0x00);
            encode_uleb128(&mut wasm, payload.len() as u64);
            wasm.extend_from_slice(&payload);
        }
        let err = extract_abi(&wasm).unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(msg) if msg.contains("duplicate")));
    }

    #[test]
    fn extract_abi_rejects_garbage_payload() {
        let wasm = wasm_with_abi(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let err = extract_abi(&wasm).unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(_)));
    }

    #[test]
    fn extract_abi_rejects_future_version() {
        let mut abi = sample_abi();
        abi.pyde_abi_version = 0xFFFF_FFFF;
        let bytes = borsh::to_vec(&abi).unwrap();
        let wasm = wasm_with_abi(&bytes);
        let err = extract_abi(&wasm).unwrap_err();
        assert!(matches!(err, SdkError::InvalidArgument(msg) if msg.contains("unsupported")));
    }

    #[test]
    fn extract_abi_section_returns_raw_bytes() {
        let abi = sample_abi();
        let bytes = borsh::to_vec(&abi).unwrap();
        let wasm = wasm_with_abi(&bytes);
        let extracted = extract_abi_section(&wasm).unwrap();
        assert_eq!(extracted, &bytes[..]);
    }
}
