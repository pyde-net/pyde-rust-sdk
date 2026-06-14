# Upstream asks

Living index of drift the SDK has surfaced in sibling repos while
building or testing against them. Each entry is a self-contained
hand-off an engine / otigen maintainer can read in isolation.

Items here are not SDK bugs — they're either spec / scaffold drift
in the upstream tooling, or stale defaults that bite as soon as a
real (non-trivial) contract ships.

**SDK-side workarounds** are baked into `examples/halt_methods.rs`
and `examples/contracts/access-guard/` so live integration works
today; once each upstream patch lands the corresponding workaround
collapses out (notes in each entry's "SDK cleanup once fixed"
line).

---

## Engine

### E1 — `pyde devnet --prefund-amount` help text off by 1000

**Repo**: `pyde-net/engine`
**Surfaced in**: T14 (devnet E2E).

The CLI help string says:

> "Default 10,000,000,000 quanta = 10,000 PYDE [default: 10000000000]"

Math is wrong. `PYDE_DECIMALS = 9`
(`engine/crates/types/src/account.rs:47` — "1 PYDE = 10⁹ micro-PYDE"),
so `10^10 quanta / 10^9 = 10 PYDE`, not 10,000.

**Fix** (one-line, either side):
- correct the help text to `"= 10 PYDE"`, OR
- bump the default to `10_000_000_000_000` quanta (= 10,000 PYDE) if
  that was the intent.

**SDK cleanup once fixed**: `examples/devnet_e2e.rs:86-90` asserts
`bal_sender == 10_000_000_000` (the actual default). If the engine
side bumps the default to 10,000 PYDE, the SDK assertion needs to
change to match. If only the text is corrected, no SDK change.

---

## Otigen

### O1 — `u64` return type emitted as `"I64"` in ABI JSON

**Repo**: `pyde-net/otigen`
**Surfaced in**: T14 (counter contract deploy + view call).

**Reproducer**:

```sh
otigen new counter --from counter
cd counter && otigen build
cat artifacts/counter.bundle/abi.json
```

The source contract declares `fn get() -> u64`, but the emitted ABI
says:

```json
{ "name": "get", "returns": "I64", ... }
```

**Expected**: `"returns": "U64"`. `ParamType::U64` is a distinct
variant in `engine/crates/types/src/abi.rs::ParamType`.

**Impact**: wire bytes are identical (8 little-endian bytes either
way), so the value still round-trips — but the type tag is wrong.
Wallets/explorers display the wrong type; range / overflow
validators use the wrong domain (treat `u64::MAX` as `-1`).

**Fix**: in the otigen signature-lowering step, map `u64 →
ParamType::U64` (not `I64`). Likely a single-arm typo; worth
grepping the whole table for `u*` / `i*` cross-talk while the
patch is open.

**SDK cleanup once fixed**: none. The SDK's typed decoder accepts
either tag for 8-byte LE input (same wire format), so behavior is
identical.

---

### O2 — `calldata_copy` stale 3-arg signature in AS / Go / C scaffolds

**Repo**: `pyde-net/otigen`
**Surfaced in**: T18 (Go access-guard contract) + T19 (4-language
survey).

**Where**:
- `otigen init --lang as` → `assembly/host_fns.ts`
- `otigen init --lang go` → `host_fns.go`
- `otigen init --lang c`  → `host_fns.h`

All three declare the 3-arg version:

```ts
// AssemblyScript
@external("pyde", "calldata_copy")
export declare function calldata_copy(offset: i32, len: i32, out_ptr: usize): i32;
```
```go
// Go
//go:wasmimport pyde calldata_copy
func calldata_copy(offset int32, length int32, outPtr int32) int32
```
```c
// C
extern int32_t calldata_copy(int32_t offset, int32_t len, uint8_t* out_ptr);
```

**Engine actually imports**
(`engine/crates/wasm-exec/src/host_fns/calldata.rs:97-105`):

```
(import "pyde" "calldata_copy" (func (param i32 i32) (result i32)))
//        params: out_ptr, out_len_ptr (the 4-byte buffer carries
//        the max-accept length in and is overwritten with the
//        actual bytes copied)
```

**Reproduce**: scaffold any of the three, add an entry that takes a
`uint64` arg, build. Validation rejects:

```
BuildRejected: import pyde.calldata_copy has wrong type —
expected (func (param i32 i32) (result i32)),
declared (func (param i32 i32 i32) (result i32))
```

**Impact**: every non-Rust contract that takes *any* input fails
to deploy. Empty counter scaffolds compile and validate fine
because they never call `calldata_copy`.

**Fix**: update the three host-fn headers to the 2-arg shape +
update the doc-comment to explain the in/out length convention.
Rust's `pyde-host` crate already has the correct signature; use
it as the reference.

**SDK cleanup once fixed**:
`examples/contracts/access-guard/host_fns.go` ships a hand-fixed
2-arg signature. Once otigen's scaffold is correct, the fixture
can stay (still smaller than the canonical 8-fn version) or be
regenerated from `otigen init --lang go` — both work.

---

### O3 — TinyGo default build flags exceed `MAX_CALLDATA`

**Repo**: `pyde-net/otigen`
**Surfaced in**: T18 + T19.

**Where**: TinyGo invocation in `otigen build` (Go language
adapter).

**Current behaviour**: invokes
`tinygo build -target=wasm-unknown -o ./build/contract.wasm .`
with no size-opt flags. The minimal counter scaffold lands at
59 KiB — **under** the 64 KiB `MAX_CALLDATA` cap by 6 KiB. Add a
handful of real entry points and you blow it.

The `access-guard` contract for T18 (8 entry points, ~250 lines)
hit 71,744 bytes. Manual workaround:

```sh
tinygo build -target=wasm-unknown -opt=z -no-debug -o ./build/contract.wasm .
otigen build --no-compile
```

Same contract drops to **5,930 bytes** with `-opt=z -no-debug`.

**Fix**: `otigen build` should pass `-opt=z -no-debug` on
`wasm-unknown` by default. If a "size vs diagnostics" choice ever
matters, expose `[contract.lang.toolchain] optimize = "size" |
"speed"` and default to `"size"`.

**SDK cleanup once fixed**:
`examples/halt_methods.rs` doc-comment `## Prereqs` currently
walks the user through the 4-line manual workaround. Once otigen
defaults are fixed, that section collapses to plain
`otigen build`. Same for
`examples/contracts/access-guard/README.md`.

---

### O4 — C scaffold's Makefile default goal is `help`

**Repo**: `pyde-net/otigen`
**Surfaced in**: T19 (4-language survey).

**Where**: `otigen init --lang c` → `Makefile`.

```make
.DEFAULT_GOAL := help
```

**Effect**: `otigen build` invokes `make` (no target), which runs
the `help` printf, exits 0, and produces no `build/contract.wasm`.
Otigen then errors with:

```
CompileOutputMissing: compiler exited 0 but did not produce
./build/contract.wasm
```

Running `make build` directly works fine — the scaffold's `build`
target itself is correct. The 858-byte WASM it emits is the
smallest of any language, so the C path is fully viable once this
Makefile typo is fixed.

**Fix** (either is sufficient):
- Change scaffold to `.DEFAULT_GOAL := build`, OR
- Change the C adapter inside otigen to invoke `make build` instead
  of plain `make`.

**Bonus typo** in the same Makefile's `help` target:

```make
@printf "  make inspect       otigen inspect $$(NAME) --network devnet\n"
```

`$$(NAME)` evaluates to `$(NAME)` and the shell tries to run `NAME`
as a command (bash errors: `NAME: command not found`). Should be a
literal-friendly construct (e.g. escape further or just spell
the project name). Not build-breaking, breaks the help output
display only.

**SDK cleanup once fixed**: none — the SDK doesn't ship any C
contract today.

---

## Language scaffold survey (T19 — snapshot 2026-06-14)

| Lang | Empty scaffold | + arg-taking fn | Counter WASM size | Blockers |
|---|---|---|---|---|
| Rust            | ✅ | ✅ | 5,073 B  | — |
| AssemblyScript  | ✅ | ❌ | 1,210 B  | O2 |
| Go (TinyGo)     | ✅ | ❌ | 59,272 B | O2 + O3 |
| C / clang       | ❌ | n/a | 858 B (via `make build`) | O4 + O2 |

**Today** Rust is the only language whose scaffold ships a working
build pipeline for non-trivial contracts. Fix O2 + O4 and AS / Go /
C all become viable; O3 makes Go shippable for real-sized
contracts.

Suggested fix order: O4 (1-line, unblocks C entirely) → O2
(unblocks every non-trivial AS / Go / C contract) → O3 (makes Go
shippable for real-sized contracts).
