# pyde-rust-sdk documentation

Comprehensive reference for the Rust SDK against the Pyde
blockchain. Every chapter is self-contained — read end-to-end the
first time, then dip in by topic.

## Table of contents

| # | Chapter | What |
|---|---|---|
| 1 | [Install](01-install.md) | Cargo dep, MSRV, system tooling |
| 2 | [Quickstart](02-quickstart.md) | 5-minute end-to-end against a local devnet |
| 3 | [Concepts](03-concepts.md) | FALCON-512, Poseidon2/Blake3, addresses, nonce window, wave vs block, PYDE vs quanta |
| 4 | [Wallets](04-wallets.md) | `Wallet`, `LocalSigner`, `Keystore`, custom signers |
| 5 | [Transactions](05-transactions.md) | `TxBuilder`, `tx_hash`, signing, encoding, gas + fees |
| 6 | [Providers](06-providers.md) | `HttpProvider`, `WsProvider`, every RPC method, `PendingTx` |
| 7 | [Contracts](07-contracts.md) | Deploy, `pyde_abi!` macro, dynamic `Contract`, `Value`, events |
| 8 | [Events](08-events.md) | `LogFilter`, `EventFilter`, cursor pagination, WS subscriptions |
| 9 | [Errors](09-errors.md) | `SdkError`, `ErrorCode`, revert-reason decoding, dapp UX |
| 10 | [Multisig](10-multisig.md) | Treasury bundle, `canonical_msg`, `sign_action`, 2-of-3 walkthrough |
| 11 | [Examples](11-examples.md) | Walkthrough of every `examples/*.rs` file |
| 12 | [Compatibility](12-compatibility.md) | Wire format guarantees, ABI version range, MSRV, semver intent, TS SDK delta |

## Conventions

- **Code examples** are real and compile against the current
  workspace — file paths in samples match the repo layout.
- **Wire types** (anything that crosses the JSON-RPC boundary or
  gets borsh-encoded) are pinned byte-for-byte to the chain
  engine. Where this matters, the chapter calls it out and points
  at the engine source.
- **"Wave"** is what Pyde calls a committed batch of transactions
  — analogous to an Ethereum block. Both terms appear in docs;
  the SDK ships `BlockHeader = WaveHeader` as a porting alias but
  new code should use `WaveHeader`.
- **Quanta** is the smallest PYDE unit. `1 PYDE = 10^9 quanta`
  (`PYDE_DECIMALS = 9`). Internal balances are always `u128`
  quanta; never floats.
- **Async-first** — every network call returns `Future`. We use
  `tokio` throughout the examples; any runtime that satisfies the
  futures contract works.

## What's not in v1

Honest gaps the docs don't gloss over:

- **Cross-SDK keystores** — `pyde-ts-sdk` uses a different cipher
  + envelope shape; keystores don't import across SDKs today.
  Wire-format types (Tx, AuthKeys, FALCON pubkey/sig encoding)
  *are* shared.
- **Session keys + programmable accounts** — reserved tags exist
  in `AuthKeys`; the helper surface ships in v2.
- **Hardware wallet trait** — `Signer` is async + trait-objected
  to make this easy when it lands, but no concrete HW backend
  ships in v1.
- **Subscription kinds beyond `subscribe_logs`** — `subscribe_pending_txs`
  and `subscribe_new_waves` are queued behind engine work.

## Where to file issues

Bugs and SDK-side feature requests:
[github.com/pyde-net/pyde-rust-sdk/issues](https://github.com/pyde-net/pyde-rust-sdk/issues).

Chain-side bugs (wire format, RPC behavior, consensus): file
against [`pyde-net/engine`](https://github.com/pyde-net/engine).

Contract tooling / `otigen` issues: file against
[`pyde-net/otigen`](https://github.com/pyde-net/otigen).
