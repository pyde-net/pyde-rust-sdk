# 11. Examples

[← back to TOC](README.md) · prev: [Multisig](10-multisig.md) · next: [Compatibility →](12-compatibility.md)

---

Every `examples/*.rs` file is a runnable program demonstrating
one or more SDK surfaces. This chapter is a guided index —
what each example shows, how to run it, expected output, and
where to look in the source for the bits you'd lift into your
own dapp.

## Table of contents

- [11.1 Running an example](#111-running-an-example)
- [11.2 Local-only examples](#112-local-only-examples)
  - [`wallet_basics.rs`](#wallet_basicsrs)
  - [`keystore.rs`](#keystorers)
  - [`multisig_treasury.rs`](#multisig_treasuryrs)
- [11.3 Live examples (require a running devnet)](#113-live-examples-require-a-running-devnet)
  - [`transfer.rs`](#transferrs)
  - [`private_transfer.rs`](#private_transferrs)
  - [`devnet_e2e.rs`](#devnet_e2ers)
  - [`subscribe_logs.rs`](#subscribe_logsrs)
  - [`contract_dynamic.rs`](#contract_dynamicrs)
  - [`contract_typed.rs`](#contract_typedrs)
  - [`nft_marketplace.rs`](#nft_marketplacers)
  - [`halt_methods.rs`](#halt_methodsrs)
- [11.4 Running all of them](#114-running-all-of-them)

---

## 11.1 Running an example

From the SDK repo root:

```sh
cargo run --example <name>
```

Environment variables most live examples consume:

| Var | Default | What |
|---|---|---|
| `PYDE_RPC_URL` | `http://127.0.0.1:9933` | HTTP RPC endpoint. `otigen devnet` picks a random port unless you pass `--rpc-listen`, so set this to whatever the devnet advertised at launch. |
| `PYDE_WS_URL` | `ws://127.0.0.1:9933/ws` | WebSocket endpoint. Same caveat as `PYDE_RPC_URL`. |
| `PYDE_SENDER_SEED` | `devnet-0` seed | 32-byte hex seed for the sender wallet in `transfer.rs`. Defaults to the prefunded `devnet-0` account. |
| `PYDE_RECIPIENT` | (per-example) | Override the default recipient address. |
| `PYDE_CONTRACT_NAME` | (per-example) | Override the default contract name. |
| `PYDE_CONTRACT_WASM` | (none) | Optional — point a deploy-example at a custom WASM file. |

Start a devnet first (one terminal) and run examples in another.
Pin the RPC port if you want the examples to work without setting
`PYDE_RPC_URL`:

```sh
otigen devnet --rpc-listen 127.0.0.1:9933 --prefund-count 10
```

---

## 11.2 Local-only examples

These run against pure in-process code and never touch the
network — they always work.

---

### `wallet_basics.rs`

**Run:**
```sh
cargo run --example wallet_basics
```

**What it does:**

Generates a fresh `Wallet` from OS entropy, prints the
address + pubkey, signs an arbitrary 32-byte hash, and
verifies the signature. The smallest reasonable hello-world for
"did the install work?"

**Expected output:**
```
created wallet: 0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
pubkey:         0x09597966f87e94eb3e50d78a56a04918eed0faab09ddbf45e29210a8c5111c2e... (897 bytes)
signing hash:   0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
signature:      809 bytes (varies per call — FALCON is randomised)
verified:       true
```

**Surfaces shown:**
- `Wallet::generate`
- `Signer::sign_hash`
- Verifying with `pyde_crypto::falcon::falcon_verify`

---

### `keystore.rs`

**Run:**
```sh
cargo run --example keystore
```

**What it does:**

Round-trips a wallet through the encrypted keystore: generate →
`to_keystore("pw")` → write JSON to `/tmp/pyde-keystore-example.json`
→ re-read → `from_keystore` → assert address matches. Then
demonstrates the wrong-password failure path.

**Expected output:**
```
created wallet:  0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
wrote keystore:  /tmp/pyde-keystore-example.json
restored wallet: 0x2d0fcd97e1773e3a99cddf0a11108ee5e199926bcf41639ddfa978f826d89cd2
wrong-password error: invalid argument: decrypt failed — bad password or corrupt keystore
```

**Surfaces shown:**
- `Wallet::to_keystore` (Argon2id + AES-256-GCM)
- `Wallet::from_keystore`
- The `Keystore` JSON envelope (`version`, `address`, `pubkey`, `kdf`, `cipher`)

---

### `multisig_treasury.rs`

**Run:**
```sh
cargo run --example multisig_treasury
```

**What it does:**

Generates three FALCON signers, has two of them sign the
canonical treasury-spend message, assembles the bundle, builds
the envelope-style `MultisigTx`, asserts the wire round-trip
preserves every field. Submission to a live devnet is commented
out — the chain's treasury account is initialised at genesis
with a specific signer set, so an arbitrary 3-key set generated
here wouldn't verify.

**Expected output:**
```
== Treasury signer set (3 FALCON-512 keys) ==
  signer[0]: 0x91d71aa34b9bb74da9e141f4ff43881ec89782536c6072aaa5899d685b30a865
  signer[1]: 0xaa3608e5000fa99389bef9d6fb596ff741387fe2822eb9df98d930e134c4907d
  signer[2]: 0x90e1c43419c5ae6c82553394c3a71cfdd1c86807329e68acad0c70dd226dd23f
  bundle entry 0: 809 sig bytes
  bundle entry 2: 809 sig bytes

== Bundle: 2-of-3 ==

== Built MultisigTx ==
  data: 1686 bytes (borsh MultisigTxPayload)
  wire bytes: 1805 total
  ✓ borsh round-trip preserves target + amount + bundle

✔ Multisig treasury spend constructed end-to-end
```

**Surfaces shown:**
- `multisig::canonical_msg`
- `multisig::sign_action`
- `multisig::MultisigTxPayload` + `SigBundle` wire shape
- `TxBuilder::multisig_treasury_spend`
- `tx::encode` / `tx::decode` wire round-trip

---

## 11.3 Live examples (require a running devnet)

These all take `PYDE_RPC_URL` (default
`http://127.0.0.1:9933`) and assume `otigen devnet` is running.

---

### `transfer.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example transfer
```

**What it does:**

Generates a fresh wallet, queries chain-id + nonce + balance,
builds a 1.5-PYDE transfer to a recipient (`PYDE_RECIPIENT` env
override), signs, submits, waits for the receipt.

The canonical "submit my first tx" demo. NB: the fresh wallet
has zero balance, so the chain will reject the submission with
`insufficient balance` — this is intentional, showing the error
path. To see a successful submit, replace `Wallet::generate()`
with `Wallet::from_seed(devnet_seed(0))` (see
[Quickstart §2.3](02-quickstart.md#23-the-full-program)).

**Expected output (transient-failure path):**
```
chain_id:  31337
sender:    0xa1b2c3...
balance:   0 quanta
nonce:     0
submitting: 0xdeadbeef... (signed, 1986 bytes)
error: insufficient balance
```

**Surfaces shown:**
- `HttpTransport::new` + `RootProvider::new`
- `Provider::chain_id`, `get_balance`, `get_nonce`
- `TxBuilder::transfer`
- `Signer::sign_tx`
- `PendingTx::wait_for_receipt`

---

### `private_transfer.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example private_transfer
```

**What it does:**

Round-trips a transfer through Pyde's MEV-protected private
mempool via the one-call commit-reveal flow. Builds + signs a
plaintext inner Tx, then hands it to
`provider.send_private(&wallet, inner_tx)`, which computes the
commitment, submits a `Commit` (bonded by `required_bond`), waits
out the reveal window, submits the matching `Reveal`, and resolves
on the *inner* tx receipt. The commitment fixes the inner tx's
ordering position before its contents are visible, with no
decryption key anywhere — so content-targeted front-running is
prevented. It is not a total ordering lock: the reveal necessarily
exposes the contents before the inner tx executes, and an unrelated
tx arriving in the reveal-to-execute window can still be ordered
around it.

**Expected output:**
```
signed inner tx: 928 bytes, inner_hash=0x7644b2d6...
commitment: 0x3f9c1a...  value_ceiling=1500000000  bond=15000000 quanta
submitted commit: 0xb353a662...
waiting out reveal window (120 waves) ...
submitted reveal: 0xa1774c0e...
waiting for inner receipt under 0x7644b2d6...
✓ committed: status=Success wave=134 gas_used=100000
```

**Surfaces shown:**
- `RootProvider::send_private(&wallet, inner_tx)` — the one-call flow
- `PrivateSendHandle::commit_hash()` / `reveal_hash()` / `inner_hash()`
- `PrivateSendHandle::await_receipt()` — resolves on the inner tx receipt
- `tx::commitment_hash` + `tx::required_bond`
- The `COMMIT_REVEAL_WINDOW_WAVES` / `MIN_COMMIT_BOND` / `COMMIT_BOND_BPS` constants

**Note:** For relays or split commit/reveal phases, drop to the
low-level `TxBuilder::commit(commitment, value_ceiling)` and
`TxBuilder::reveal(commitment, nonce, inner_tx_bytes)` builders,
which emit `TxType::Commit` (`0x11`) and `TxType::Reveal` (`0x12`)
carrying `CommitPayload` / `RevealPayload` in `tx.data`.

---

### `devnet_e2e.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example devnet_e2e
```

**What it does:**

Reproduces the devnet's pre-funded wallets via the Blake3 seed
scheme, asserts the 10-PYDE pre-fund landed, runs a self-pay
transfer + verifies balances move correctly, decodes the account
record, and runs an error-envelope test on the chain's revert
path. The smoke test the SDK CI mirror runs.

**Expected output:**
```
== devnet-0 reproduced ==
address: 0xf07856fdf4796baa6d477ddfe926774d367b25c20e8c7d9d337b63034c9e0cfa
pubkey:  0x0943494b728c5e84…
balance: 10.0 PYDE  (10000000000 quanta)
devnet-1 balance: 10.0 PYDE

== account record ==
  type: eoa
  nonce: 0
  code_hash: 0x0000000000000000000000000000000000000000000000000000000000000000

== transferring 1.234 PYDE from devnet-0 → 0xdd182ff6... ==
signed tx: 809 sig bytes
submitted: 0xb8494f86ad764a5734c5e0bf2d4a4d8e4f6d35a9414d7c8b7f36accaa854ddef
committed: wave 57 / tx_index 0 / status Success / gas 100000
recipient balance: 0 → 1234000000 (0+1234000000)
sender   balance: 10000000000 → 8765900000 (spent 1234100000 quanta = 100000 gas + value)

== error-envelope test ==
(expected error) invalid argument: decode CallPayload from `data`: ...

✔ devnet E2E smoke test passed
```

**Surfaces shown:**
- `Wallet::from_seed` + the deterministic devnet seed scheme
- `Provider::get_account`, `get_balance`, `get_nonce`
- Self-pay transfer round-trip + balance accounting
- Error decoding from a deliberately-malformed `pyde_call`

---

### `subscribe_logs.rs`

**Run:**
```sh
PYDE_WS_URL=ws://127.0.0.1:9933/ws cargo run --example subscribe_logs
```

**What it does:**

Opens a WebSocket, subscribes to logs via `subscribe_logs`,
streams events to stdout until you `Ctrl-C`. Filter is
configurable via `PYDE_CONTRACT_ADDR` / `PYDE_TOPIC0` env vars.

**Expected output:**
```
WS connected: ws://127.0.0.1:9933/ws
subscription id: 0x42
(waiting for events; emit some via another tx, or Ctrl-C to exit)
event: wave=1234, address=0xdead..., topics=["0x..."]
event: wave=1235, address=0xdead..., topics=["0x...", "0x..."]
^C
```

(Idle if nothing's emitting events; trigger some via the
`contract_*` or `nft_marketplace` examples to see frames flow.)

**Surfaces shown:**
- `WsProvider::connect_ws`
- `Subscription<Event>::recv()` loop
- `LogFilter` env-driven configuration

---

### `contract_dynamic.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 PYDE_CONTRACT_NAME=counter cargo run --example contract_dynamic
```

**Requires:** a `counter` contract already deployed on chain
(see `otigen init --lang rust counter && otigen build && otigen deploy`).

**What it does:**

Loads a contract by name via `Contract::load`, makes a view
call, decodes the result via `Value`.

**Expected output:**
```
loaded contract: counter (at 0x123456...)
abi: 2 functions
calling get_count() ...
return: U64(42)
```

**Surfaces shown:**
- `Contract::load(name, provider)` — name resolution + WASM fetch + ABI parse
- `Contract::call(name, args)` for view functions
- The dynamic `Value` enum

---

### `contract_typed.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example contract_typed
```

**Requires:** the same `counter` contract deployed, plus
`abi/counter.json` bundled at compile time.

**What it does:**

Same as `contract_dynamic.rs` but uses the `pyde_abi!` proc-macro
to generate compile-time typed wrappers. Compare the two files
side-by-side to see the ergonomic delta — the typed version has
no string method names, no `Value` boxing, full Rust type
inference.

**Expected output:**
```
typed counter at 0x123456...
get_count() → 42
add(5) → submitted 0xab...
wait for receipt ... wave 1234 / status Success
get_count() → 47
```

**Surfaces shown:**
- `pyde_abi!(Counter, "abi/counter.json")` — macro-generated wrapper
- Generated `async fn get_count(&self) -> Result<u64>` (view)
- Generated `async fn add(&self, signer, n, gas_limit, value) -> Result<PendingTx>` (state-mutating)

---

### `nft_marketplace.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example nft_marketplace
```

**What it does:**

Multi-account, multi-contract orchestration. Deploys three
contracts (a PTS-F token + a PTS-N NFT + a custom marketplace), funds three
accounts (seller, buyer, treasury), runs an atomic NFT swap +
royalty payout end-to-end, asserts every balance moves correctly.

457 lines of real dapp orchestration code — closest thing in
the SDK to "what a production integration test looks like."

**Expected output (abbreviated):**
```
deploying fungible-token at 0x...
deploying nft-mkt at 0x...
deploying marketplace at 0x...
funding seller: 100 PYDE
funding buyer:  100 PYDE
minting NFT #1 to seller
seller approves marketplace for NFT #1
seller lists NFT #1 for 50 PYDE
buyer purchases NFT #1
verifying balances:
  seller: 95.0 PYDE + 0 NFT
  buyer:  50.0 PYDE + 1 NFT (#1)
  treasury (royalty): 5.0 PYDE
✔ atomic swap verified
```

**Surfaces shown:**
- Multi-account `LocalSigner` setup
- Contract deployment with `init_calldata`
- Cross-contract calls (`approve` → `purchase`)
- Event-driven receipt verification
- The typical structure of a real dapp test harness

---

### `halt_methods.rs`

**Run:**
```sh
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example halt_methods
```

**Requires:** a Go-authored `access-guard` contract (source in
`examples/contracts/access-guard/`). Build with:

```sh
cd examples/contracts/access-guard
tinygo build -target=wasm-unknown -opt=z -no-debug -o build/contract.wasm .
otigen build --no-compile
```

**What it does:**

Deploys the contract and walks through every halt mechanism
Pyde supports:

| # | Halt mode | What the contract does |
|---|---|---|
| 1 | **Authorization revert** | Unauthorised caller hits the admin-guarded entry; chain returns a plain-text revert reason. |
| 2 | **Plain message revert** | Entry calls `pyde.revert` with a UTF-8 string. |
| 3 | **`ERR_*` named-token revert** | Entry includes `"ERR_FORBIDDEN"` in the revert string; SDK's `error_code()` extracts the named token. |
| 4 | **Negative integer code revert** | Entry includes `"-5"` in the revert string; SDK's `error_code()` parses it. |
| 5 | **WASM trap** | Entry triggers an out-of-bounds index; chain catches the trap and surfaces `ERR_CROSS_CALL_FAILED`. |

Includes a `precise_user_message()` helper showing how to map
every halt mode to dapp-grade error UX. See [Errors §9.6](09-errors.md#96-dapp-ux-pattern--map-codes-to-messages).

**Expected output (abbreviated):**
```
deployed access-guard at 0x...

[1/5] authorization revert
   raw revert reason: "unauthorized: caller is not admin"
   user-facing: You're not authorised to do that.

[2/5] plain message revert
   raw revert reason: "custom message: contract deliberately reverted"
   user-facing: Reverted: custom message: contract deliberately reverted

[3/5] ERR_FORBIDDEN named-token revert
   raw revert reason: "ERR_FORBIDDEN: this entry never permits execution"
   extracted code: ERR_FORBIDDEN (-5)
   user-facing: You're not authorised to do that.

[4/5] negative integer code revert
   raw revert reason: "aborted with code -5"
   extracted code: ERR_FORBIDDEN (-5)
   user-facing: You're not authorised to do that.

[5/5] WASM trap → ERR_CROSS_CALL_FAILED
   ...
   extracted code: ERR_CROSS_CALL_FAILED (-10)
   user-facing: A sub-call failed — try again or raise gas.

✔ all halt modes parsed correctly
```

The SDK also surfaces this last case as `RevertCategory::Vm` via
the structured `revert_reason` field on the receipt — see
[Errors §9.9](09-errors.md#99-structured-revert-categories) for
branching on the failure *layer* without parsing messages.

**Surfaces shown:**
- `TxBuilder::deploy` with a Go-authored WASM
- `SdkError::Reverted` + `revert_reason()` decoding
- `SdkError::error_code()` with both named-token and integer
  extraction paths
- `SdkError::from_receipt(&receipt)` for converting a non-success
  `Receipt` to the matching variant
- `revert_category()` / `is_engine_validation_revert()` /
  `is_contract_revert()` / `is_vm_trap()` for branching on the
  *layer* that rejected the tx without parsing messages
- Every revert encoding the chain supports

---

## 11.4 Running all of them

```sh
# 1. Local-only — always work.
cargo run --example wallet_basics
cargo run --example keystore
cargo run --example multisig_treasury

# 2. Start a devnet in the background.
otigen devnet --rpc-listen 127.0.0.1:9933 &
DEVNET_PID=$!
sleep 1

# 3. Run live examples.
for ex in transfer devnet_e2e; do
    PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example $ex
done

# 4. Deploy the counter contract for the contract examples.
( cd /tmp && \
  otigen init --lang rust counter && cd counter && otigen build && \
  PYDE_RPC_URL=http://127.0.0.1:9933 otigen deploy )

PYDE_RPC_URL=http://127.0.0.1:9933 PYDE_CONTRACT_NAME=counter \
    cargo run --example contract_dynamic
PYDE_RPC_URL=http://127.0.0.1:9933 \
    cargo run --example contract_typed

# 5. The larger demos build their own contracts inline.
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example nft_marketplace
PYDE_RPC_URL=http://127.0.0.1:9933 cargo run --example halt_methods

# 6. Clean up.
kill $DEVNET_PID
```

The contract examples (`contract_*`, `nft_marketplace`,
`halt_methods`) need their contract bundles built — see each
example's source comments for the exact `otigen` invocation.
