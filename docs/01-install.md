# 1. Install

[← back to TOC](README.md) · next: [Quickstart →](02-quickstart.md)

---

## Cargo

```toml
[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
```

The SDK is pre-1.0; pin the git rev for production:

```toml
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk", rev = "<sha>" }
```

Once the crate is published to crates.io:

```toml
pyde-rust-sdk = "0.1"
```

## Minimum supported Rust version

**Rust 1.75+** (`async fn` in traits is stable as of 1.75; the
[`Provider`](06-providers.md) trait depends on it).

Check yours:

```sh
rustc --version
# rustc 1.75.0 or newer
```

Upgrade via [rustup](https://rustup.rs):

```sh
rustup update stable
```

## Runtime

Any executor that implements the standard futures contract works.
Examples use [`tokio`](https://crates.io/crates/tokio) with the
`full` feature set. Minimal feature set:

```toml
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
```

`rt-multi-thread` is needed because the HTTP and WebSocket
transports park on multiple worker threads. If you're embedding
in a single-threaded executor, swap to `rt` + ensure your top-level
task drives I/O explicitly.

## Optional: `pyde` devnet binary

For local development you'll want the chain binary. It ships in
the [`pyde-net/engine`](https://github.com/pyde-net/engine) repo
and provides `pyde devnet` — a single-validator, instant-wave
chain at `127.0.0.1:8545`.

```sh
git clone https://github.com/pyde-net/engine
cd engine
cargo build --release --bin pyde
./target/release/pyde devnet --rpc-listen 127.0.0.1:8545
```

Banner-printed pre-funded accounts (deterministic from
`Blake3("pyde-devnet-v1/" || i)`) give you 10 PYDE each to play
with. Quickstart picks up from here.

## Optional: `otigen` for contracts

To deploy WASM contracts you'll want
[`otigen`](https://github.com/pyde-net/otigen) — Pyde's contract
toolchain:

```sh
git clone https://github.com/pyde-net/otigen
cd otigen
cargo build --release --bin otigen
```

Scaffold a contract:

```sh
otigen init --lang rust counter
cd counter
otigen build
# → produces ./artifacts/counter.bundle/contract.wasm
```

Supported languages: `rust` (recommended), `as` (AssemblyScript),
`go` (TinyGo), `c` (clang wasm32).

## Verify the install

A 10-line smoke test:

```rust,no_run
use pyde_rust_sdk::Wallet;

fn main() -> anyhow::Result<()> {
    let w = Wallet::generate()?;
    println!("address: {}", w.address());
    Ok(())
}
```

```sh
cargo run
# address: 0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
```

If that prints an address, you're set.

## Troubleshooting

**`pyde-crypto` fails to fetch on CI** — the `pyde-crypto` repo
is private during pre-v1. CI runs in environments without
`pyde-net` access will fail at dependency resolution. Workaround
until the crate is published: run CI in an environment with
access, or pin to a vendored snapshot.

**`reqwest` link errors on Linux** — the SDK builds against
`rustls` rather than OpenSSL, so the `pkg-config` / `libssl-dev`
prereqs you might expect don't apply. If you see openssl link
errors, you're pulling a different version of reqwest from a
parent crate — add `default-features = false, features = ["json",
"rustls-tls"]` to your reqwest dep.

**TLS handshakes fail against `localhost`** — the devnet HTTP
RPC server is HTTP-only, not HTTPS. Use
`HttpTransport::new("http://127.0.0.1:8545")` (no `s`).
