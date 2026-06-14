# 11. Examples

[← back to TOC](README.md) · prev: [Multisig](10-multisig.md) · next: [Compatibility →](12-compatibility.md)

---

Every `examples/*.rs` file is a runnable program demonstrating
one or more SDK surfaces. This chapter is a guided index — what
each example shows, how to run it, and where to look in the code
for the bits you'd lift into your own project.

## Local-only (no node required)

These run against pure in-process code and never touch the
network.

### `wallet_basics.rs`

```sh
cargo run --example wallet_basics
```

Generates a fresh `Wallet`, prints the address + pubkey, signs
an arbitrary 32-byte hash, and verifies the signature. The
smallest reasonable hello-world for "did the install work."

Skills shown: `Wallet::generate`, `Signer::sign_hash`,
verifying with `pyde_crypto::falcon::falcon_verify`.

### `keystore.rs`

```sh
cargo run --example keystore
```

Round-trips a wallet through the encrypted keystore: generate
→ `to_keystore("pw")` → write JSON to `/tmp/` → re-read →
`from_keystore` → assert address matches. Also demonstrates the
wrong-password failure path.

Skills shown: `Wallet::to_keystore`, `Wallet::from_keystore`,
`Keystore` JSON envelope, password-derived AES key.

## Live (require a running node)

These all take `PYDE_RPC_URL` (default `http://127.0.0.1:8545`)
and assume `pyde devnet` is running.

### `transfer.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example transfer
```

Generates a fresh wallet, queries chain-id + nonce + balance,
builds a 1.5-PYDE transfer to a recipient (`PYDE_RECIPIENT`
env override), signs, submits, waits for the receipt. The
canonical "submit my first tx" demo.

Skills shown: `HttpTransport`, `Provider::chain_id`,
`Provider::get_balance`, `TxBuilder::transfer`,
`Signer::sign_tx`, `PendingTx::wait_for_receipt`.

### `devnet_e2e.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example devnet_e2e
```

Reproduces the devnet's pre-funded wallets via the Blake3 seed
scheme, asserts the 10-PYDE pre-fund landed, runs a self-pay
transfer + verifies balances move correctly, decodes the
account record, and runs an error-envelope test on the chain's
revert path. The smoke test the SDK CI mirror runs.

Skills shown: `Wallet::from_seed`, the devnet seed scheme,
`Provider::get_account`, error decoding from a deliberately-
malformed `pyde_call`.

### `subscribe_logs.rs`

```sh
PYDE_WS_URL=ws://127.0.0.1:8546 cargo run --example subscribe_logs
```

Opens a WebSocket, subscribes to logs via `subscribe_logs`,
streams events to stdout until you Ctrl-C. Filter is
configurable via `PYDE_CONTRACT_ADDR` / `PYDE_TOPIC0`.

Skills shown: `WsProvider::connect_ws`, `Subscription<Event>`,
`Subscription::into_stream`.

### `contract_dynamic.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 PYDE_CONTRACT_NAME=counter cargo run --example contract_dynamic
```

Loads a contract by name via `Contract::load`, makes a view
call, decodes the result via `Value`. Requires a `counter`
contract already deployed on chain.

Skills shown: `Contract::load`, `Contract::call`, the dynamic
`Value` enum, ABI extraction from a deployed contract's
`pyde.abi` custom section.

### `contract_typed.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example contract_typed
```

Same as `contract_dynamic.rs` but uses the `pyde_abi!`
proc-macro to generate compile-time typed wrappers. Compare the
two files side-by-side to see the ergonomic delta.

Skills shown: `pyde_abi!` macro, generated `async fn get_count(&self)
-> Result<u64>` shape, generated `async fn add(&self, signer,
value, gas_limit, value_quanta) -> Result<PendingTx>` shape.

## Larger demos

### `nft_marketplace.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example nft_marketplace
```

Multi-account, multi-contract orchestration. Deploys three
contracts (ERC20 + ERC721 + a custom marketplace), funds three
accounts (seller, buyer, treasury), runs an atomic NFT swap +
royalty payout end-to-end, asserts every balance moves
correctly.

Skills shown: multi-account `LocalSigner` setup, contract
deployment with `init_calldata`, cross-contract calls,
event-driven receipt verification, the typical structure of a
real dapp test harness.

### `halt_methods.rs`

```sh
PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example halt_methods
```

Deploys a Go-authored `access-guard` contract (source in
`examples/contracts/access-guard/`) and walks through every halt
mechanism Pyde supports:

1. **Authorization revert** — unauthorised caller hits the
   admin-guarded entry; chain returns a plain-text revert reason.
2. **Plain message revert** — entry calls `pyde.revert` with a
   UTF-8 string.
3. **`ERR_*` named-token revert** — entry includes `"ERR_FORBIDDEN"`
   in the revert string; SDK's `error_code()` extracts the named
   token.
4. **Negative integer code revert** — entry includes `"-5"` in
   the revert string; SDK's `error_code()` parses it.
5. **WASM trap** — entry triggers an out-of-bounds index;
   chain catches the trap and surfaces `ERR_CROSS_CALL_FAILED`.

The example includes a `precise_user_message()` helper showing
how to map every halt mode to dapp-grade error UX. See
[Errors](09-errors.md) for the full pattern.

Skills shown: `TxBuilder::deploy`, Go contract toolchain
(`otigen` + TinyGo), structured error parsing, every revert
encoding the chain supports.

### `multisig_treasury.rs`

```sh
cargo run --example multisig_treasury
```

(Local-only — see file header for why submission is commented
out.) Generates three FALCON signers, has two of them sign the
canonical treasury-spend message, assembles the bundle, builds
the envelope-style `MultisigTx`, asserts the wire round-trip
preserves every field.

Skills shown: `multisig::canonical_msg`, `multisig::sign_action`,
`MultisigTxPayload`, `TxBuilder::multisig_treasury_spend`,
borsh wire round-trip verification.

## Running them all

```sh
# Local-only (always work).
cargo run --example wallet_basics
cargo run --example keystore
cargo run --example multisig_treasury

# Live (require devnet on 8545).
pyde devnet --rpc-listen 127.0.0.1:8545 &
sleep 1
for ex in transfer devnet_e2e contract_dynamic contract_typed nft_marketplace halt_methods; do
    PYDE_RPC_URL=http://127.0.0.1:8545 cargo run --example $ex
done
```

The contract examples (`contract_*`, `nft_marketplace`,
`halt_methods`) also need their contract bundles built — see
each example's source comments for the `otigen` invocation.
