//! Pyde ABI handling — read and parse the `pyde.abi` WASM custom
//! section from deployed contract bytecode.
//!
//! ## T7 stub
//!
//! Real implementation lands in T10 (Phase A3). This module will own:
//!
//! - `ContractAbi` — Rust mirror of the Borsh-encoded struct in
//!   [`HOST_FN_ABI_SPEC §3.7`]:
//!   ```ignore
//!   struct ContractAbi {
//!       pyde_abi_version:  u32,
//!       contract_type:     ContractType,
//!       functions:         Vec<FunctionAbi>,
//!       state_schema_hash: [u8; 32],
//!       constructor_index: Option<u32>,
//!       fallback_index:    Option<u32>,
//!       receive_index:     Option<u32>,
//!   }
//!   ```
//! - `FunctionAbi { name, selector: [u8; 4], attributes: u32,
//!   access_list: Vec<AccessListEntry> }`.
//! - Attribute bitfield: `VIEW | PAYABLE | REENTRANT | SPONSORED |
//!   CONSTRUCTOR | FALLBACK | RECEIVE | ENTRY`.
//! - [`parse_pyde_abi(wasm: &[u8]) -> Result<ContractAbi>`] — walks
//!   the WASM binary via `wasmparser`, finds the `pyde.abi` custom
//!   section, Borsh-decodes it. Rejects: missing section, duplicate
//!   section, decode failure, version > engine's max supported.
//!
//! This is what the `pyde_abi!` macro reads when invoked with a
//! deployed contract address — fetch the `.wasm` via
//! `pyde_getContractCode(addr)`, parse here, generate typed bindings
//! from the result.
