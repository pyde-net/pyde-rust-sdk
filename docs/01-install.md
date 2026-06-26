# 1. Install

[← back to TOC](README.md) · next: [Quickstart →](02-quickstart.md)

---

## Table of contents

- [1.1 Adding the SDK to your project](#11-adding-the-sdk-to-your-project)
- [1.2 Minimum Rust version (MSRV)](#12-minimum-rust-version-msrv)
- [1.3 Async runtime](#13-async-runtime)
- [1.4 Installing `otigen` (devnet + contracts)](#14-installing-otigen-devnet--contracts)
- [1.5 Verifying the install](#15-verifying-the-install)
- [1.6 Optional dev tooling](#16-optional-dev-tooling)
- [1.7 Troubleshooting](#17-troubleshooting)

---

## 1.1 Adding the SDK to your project

The SDK is pre-1.0 and currently distributed as a git dependency.
Add to your `Cargo.toml`:

```toml
[dependencies]
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk" }
tokio = { version = "1", features = ["full"] }
```

**For production**, pin to a specific git revision so a force-push
or branch rename can't break your build:

```toml
pyde-rust-sdk = { git = "https://github.com/pyde-net/pyde-rust-sdk", rev = "<sha>" }
```

You can find the latest reviewed revision under [Releases](https://github.com/pyde-net/pyde-rust-sdk/releases).

**For local development** (e.g., you're hacking on the SDK
alongside your dapp), point at a local path:

```toml
pyde-rust-sdk = { path = "../pyde-rust-sdk" }
```

Once the crate ships to crates.io (post-1.0), you'll be able to:

```toml
pyde-rust-sdk = "0.1"
```

---

## 1.2 Minimum Rust version (MSRV)

**MSRV: Rust 1.75.** The [`Provider`](06-providers.md) trait uses
`async fn` directly in trait definitions, stabilised in 1.75.

Check yours:

```sh
rustc --version
```

**Expected output:**
```
rustc 1.75.0 (or newer)
```

Upgrade via [rustup](https://rustup.rs):

```sh
rustup update stable
```

The repo ships a `rust-toolchain.toml` pinning the `stable`
channel so contributors get a known-good toolchain
automatically; the MSRV itself lives in `Cargo.toml`
(`rust-version = "1.75"`). Anything stable above 1.75 is
supported.

---

## 1.3 Async runtime

Every network call returns a `Future`. The examples use
[`tokio`](https://crates.io/crates/tokio); any runtime that
satisfies the standard futures contract works.

### Recommended feature set

```toml
tokio = { version = "1", features = ["full"] }
```

`full` is convenient for examples. For production builds the
minimal feature set that exercises every SDK path is:

```toml
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
```

| Feature | Why the SDK needs it |
|---|---|
| `macros` | `#[tokio::main]` + `#[tokio::test]` |
| `rt-multi-thread` | The HTTP + WebSocket transports park on multiple workers |
| `time` | `PendingTx::wait_for_receipt` uses `tokio::time::sleep` |

### Single-threaded executors

If you must use `tokio = { features = ["rt"] }` (current_thread
runtime), the SDK works but `PendingTx::wait_for_receipt` will
block the only worker during its sleep ticks. Either keep the
poll interval short or drive `PendingTx::wait_for_receipt` from
a dedicated current-thread that doesn't share work with your
event loop.

---

## 1.4 Installing `otigen` (devnet + contracts)

Pyde's developer toolchain is one binary: **`otigen`**. It
provides:

- `otigen devnet` — single-validator instant-wave local chain.
- `otigen init` / `otigen build` — scaffold and compile WASM
  contracts.
- `otigen deploy` / `otigen call` / `otigen console` — push
  contracts onto a chain and interact with them.

The chain runtime is built into `otigen` — there's no separate
`pyde` binary to install.

### Build from source

```sh
git clone https://github.com/pyde-net/otigen
cd otigen
cargo build --release --bin otigen
```

Add to your `PATH`:

```sh
export PATH="$PWD/target/release:$PATH"
```

**Verify:**

```sh
otigen --version
```

**Expected output:**
```
otigen 0.1.0
```

### Run the devnet

```sh
otigen devnet --rpc-listen 127.0.0.1:9933
```

`otigen devnet` picks a random RPC port when `--rpc-listen` is
omitted; pin it to `9933` (as above) if you want the examples and
docs to work out of the box. Otherwise, watch the startup banner
for the URL the devnet advertised and pass it to the SDK via
`PYDE_RPC_URL`.

**Expected output (truncated):**

```
═══════════════════════════════════════════════════════════════
  Pyde devnet — single-validator, instant-wave
═══════════════════════════════════════════════════════════════

Chain id:         31337
Prefund count:    10
Prefund balance:  10000000000 quanta per account
Tick interval:    1000 ms

Prefunded accounts (deterministic from canonical seed):

  (0) address: 0xf07856fdf4796baa6d477ddfe926774d367b25c20e8c7d9d337b63034c9e0cfa
      secret:  0x7720ecbbdff51d016964b92cd9d6c8082078adb55730802b796fb8e199e77991
      ...
```

The banner enumerates 10 pre-funded accounts deterministically
derived via `Blake3("pyde-devnet-v1/" || i.to_le_bytes())`. Each
account holds 10 PYDE (10,000,000,000 quanta) — see
[Concepts §3.6](03-concepts.md#36-units) for the quanta/PYDE
relationship.

Once running, the chain exposes:

| Endpoint | What |
|---|---|
| `http://127.0.0.1:9933` | JSON-RPC over HTTP — `pyde_chainId`, `pyde_sendRawTransaction`, etc. |
| `ws://127.0.0.1:9933/ws` | JSON-RPC + push subscriptions over WebSocket |

`Ctrl-C` for graceful shutdown.

### Compile a contract

```sh
otigen init --lang rust counter
cd counter
otigen build
```

**Expected output (truncated):**
```
✓ Compiled → ./build/contract.wasm
✓ Built "counter" → ./artifacts/counter.bundle
  wasm: 5,073 bytes (blake3 ...)
  abi:  141 bytes (blake3 ...)
```

Supported languages:

| `--lang` | Toolchain | Typical empty-contract size |
|---|---|---|
| `rust` (default, recommended) | `cargo` + `wasm32-unknown-unknown` | ~5 KB |
| `as` (AssemblyScript) | `asc` via `npm run build` | ~1 KB |
| `go` (TinyGo) | `tinygo build -target=wasm-unknown` (size-optimised) | ~1.8 KB |
| `c` (clang) | `clang --target=wasm32` (direct, no Make) | ~860 B |

---

## 1.5 Verifying the install

10-line smoke test:

```rust,no_run
use pyde_rust_sdk::Wallet;

fn main() -> anyhow::Result<()> {
    let w = Wallet::generate()?;
    println!("address: {}", w.address());
    println!("pubkey:  {}", w.pubkey().to_hex());
    Ok(())
}
```

```sh
cargo run
```

**Expected output (your address will differ — FALCON keygen is
randomised):**

```
address: 0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
pubkey:  0x09597966f87e94eb3e50d78a56a04918eed0faab09ddbf45e29210a8c5111c2e4a7e999725f5e582b5635731febab4d38767b36a5b5824c01f838d0ab79bddfc0b5e2f834a2489134ca80931d1891d8844187b45f4023ebb30614eef6f9e784...
```

If both lines print, you have a working SDK.

---

## 1.6 Optional dev tooling

| Tool | Why |
|---|---|
| [`cargo-watch`](https://crates.io/crates/cargo-watch) | `cargo watch -x test` for re-run-on-save |
| [`cargo-fuzz`](https://crates.io/crates/cargo-fuzz) | Run the SDK's 6 fuzz targets locally |
| [`cargo-edit`](https://crates.io/crates/cargo-edit) | `cargo upgrade` for dep bumps |
| [`cargo-nextest`](https://crates.io/crates/cargo-nextest) | Faster + nicer test runner |

Install with `cargo install <name>`. None are required to use
the SDK; all make day-to-day work nicer.

---

## 1.7 Troubleshooting

### `error: failed to load source for dependency 'pyde-crypto'`

The `pyde-crypto` crate is currently private — `cargo` can't
fetch it from a public CI environment. Workaround until it's
published:

- Run CI inside a build environment with credentials for the
  private repo.
- Or vendor `pyde-crypto` into your monorepo and patch the
  `pyde-rust-sdk` dep path.

### `reqwest` link errors on Linux about OpenSSL

The SDK builds reqwest with `rustls`, not OpenSSL — `libssl-dev`
isn't required. Link errors mean a parent crate in your tree is
pulling reqwest with its `default-features` (which pulls
`native-tls`). Fix by disabling defaults on the conflicting
crate:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

### TLS handshake fails against `localhost`

The devnet's HTTP RPC server is plain HTTP, not HTTPS. Use:

```rust,no_run
# use pyde_rust_sdk::provider::HttpTransport;
# fn run() -> pyde_rust_sdk::Result<()> {
let t = HttpTransport::new("http://127.0.0.1:9933")?;
# Ok(()) }
```

Note the `http://` (no `s`). The same applies to the WS endpoint
— `ws://127.0.0.1:9933/ws`, not `wss://`.

### `otigen build` fails with "calldata size N exceeds cap 65536"

A non-Rust contract grew past the 64 KB chain limit. For TinyGo
this is automatic now (the size flags `-opt=z -no-debug` are
applied by default in current `otigen` releases). If you see it
on Rust, you're probably linking too much; use `cargo build
--release --target wasm32-unknown-unknown -Z build-std=...` or
trim your dependency tree.

### `Connection refused` from the SDK against a running devnet

Check:

1. The devnet was started with `--rpc-listen 127.0.0.1:9933`
   (without that flag, `otigen devnet` picks a random RPC port —
   read the startup banner for the URL it actually bound to).
2. You're connecting to the same address — `http://127.0.0.1:9933`
   matches `127.0.0.1:9933`; `localhost:9933` may resolve to
   `[::1]:9933` (IPv6) and fail if the listener is IPv4-only.

The SDK now retries transient connection failures by default
(3 attempts with exponential backoff + jitter — see
[Providers §6.6](06-providers.md#66-retry-policy)). If retries
exhaust, the original error type is surfaced.
