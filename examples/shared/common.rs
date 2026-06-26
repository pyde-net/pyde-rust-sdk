//! Shared helpers used across the examples.
//!
//! Examples include this via:
//!
//! ```ignore
//! #[path = "shared/common.rs"]
//! mod common;
//! ```
//!
//! Subdirectories under `examples/` are not compiled as standalone
//! binaries by Cargo, so `examples/shared/` is a safe home for code
//! shared between examples.

#![allow(dead_code, clippy::print_stderr)]

use pyde_rust_sdk::Wallet;

const DEFAULT_RPC_URL: &str = "http://127.0.0.1:9933";
const DEFAULT_WS_URL: &str = "ws://127.0.0.1:9933/ws";

/// JSON-RPC endpoint for the active devnet.
///
/// Reads `PYDE_RPC_URL`. If unset, falls back to
/// `http://127.0.0.1:9933` and prints a one-line warning, because
/// `otigen devnet` chooses a random RPC port by default and the
/// fallback is unlikely to match.
pub fn rpc_url() -> String {
    std::env::var("PYDE_RPC_URL").unwrap_or_else(|_| {
        eprintln!(
            "warning: PYDE_RPC_URL not set, using {DEFAULT_RPC_URL}. \
             otigen devnet picks a random port; set PYDE_RPC_URL to its \
             advertised RPC URL if the connection fails."
        );
        DEFAULT_RPC_URL.to_string()
    })
}

/// WebSocket endpoint for the active devnet.
///
/// Reads `PYDE_WS_URL`. If unset, falls back to
/// `ws://127.0.0.1:9933/ws` and prints a one-line warning.
pub fn ws_url() -> String {
    std::env::var("PYDE_WS_URL").unwrap_or_else(|_| {
        eprintln!(
            "warning: PYDE_WS_URL not set, using {DEFAULT_WS_URL}. \
             otigen devnet picks a random port; set PYDE_WS_URL to its \
             advertised WS URL if the connection fails."
        );
        DEFAULT_WS_URL.to_string()
    })
}

/// Reproduce the devnet's deterministic prefund seed for the i-th
/// account. Mirrors `engine/crates/node/src/devnet/runner.rs::devnet_secret`
/// so an example always lands on the same address otigen pre-funds.
pub fn devnet_secret(i: u64) -> [u8; 32] {
    let mut input = Vec::with_capacity(b"pyde-devnet-v1/".len() + 8);
    input.extend_from_slice(b"pyde-devnet-v1/");
    input.extend_from_slice(&i.to_le_bytes());
    *blake3::hash(&input).as_bytes()
}

/// Build a wallet from `PYDE_SENDER_SEED` (32-byte hex) when set,
/// otherwise from `devnet_secret(0)` so the example sends from a
/// pre-funded devnet account by default.
pub fn seeded_wallet() -> anyhow::Result<Wallet> {
    match std::env::var("PYDE_SENDER_SEED") {
        Ok(hex_seed) => {
            let bytes = hex::decode(hex_seed.trim_start_matches("0x"))?;
            let seed: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("PYDE_SENDER_SEED must decode to exactly 32 bytes"))?;
            Ok(Wallet::from_seed(&seed)?)
        }
        Err(_) => Ok(Wallet::from_seed(&devnet_secret(0))?),
    }
}
