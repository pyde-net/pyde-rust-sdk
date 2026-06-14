# `access-guard` — Go contract demonstrating every Pyde halt method

Source for the contract that `examples/halt_methods.rs` deploys and
walks through. Written in Go (TinyGo) to prove the SDK's
language-agnostic deployment + error-parsing surface.

## Reproduce

```sh
# 1. Scaffold the layout otigen expects.
otigen init --lang go access-guard

# 2. Replace the four scaffolded files with the ones here.
cp examples/contracts/access-guard/main.go        ./access-guard/
cp examples/contracts/access-guard/host_fns.go    ./access-guard/
cp examples/contracts/access-guard/otigen.toml    ./access-guard/
cp examples/contracts/access-guard/go.mod         ./access-guard/

# 3. Build small (TinyGo's otigen default produces a 71 KiB binary
#    that exceeds the chain's 64 KiB calldata cap — pass `-opt=z
#    -no-debug` to get a ~6 KiB binary, then re-package without
#    recompiling).
cd access-guard
tinygo build -target=wasm-unknown -opt=z -no-debug -o ./build/contract.wasm .
otigen build --no-compile

# Bundle lands at:
# ./artifacts/access-guard.bundle/contract.wasm
```

## Surface

| Entry                                  | Behaviour |
|---|---|
| `init`                                 | Constructor — stores `caller` as `admin`. |
| `get_admin() -> address`               | View. |
| `get_count() -> uint64`                | View. |
| `admin_bump(n: uint64) -> uint64`      | State-mutating. Reverts with `"unauthorized: caller is not admin"` unless `caller == admin`. |
| `cause_revert_with_message`            | Reverts with a plain UTF-8 string. |
| `cause_revert_with_err_forbidden`      | Reverts with payload containing `"ERR_FORBIDDEN"` — exercises the SDK's named-token `error_code()` extractor. |
| `cause_revert_with_negative_code`      | Reverts with payload containing `"-5"` — exercises the SDK's integer-parse `error_code()` fallback. |
| `cause_panic`                          | Out-of-bounds slice access — engine traps the WASM instance, no revert payload. |

## Notes

- TinyGo wasm-unknown target requires an empty `main()` even for
  library builds. Pyde never invokes it; the chain dispatcher calls
  the `//go:wasmexport`-marked functions directly.
- `pyde_return` and `revert` are semantically `noreturn`. Go can't
  express that in an import signature, so each call site is followed
  by `for {}` to convince the wasm-ld linker not to emit
  stack-cleanup code after the host fn.
- The trimmed `host_fns.go` here imports only the 8 host fns this
  contract actually calls. The full canonical template at
  `otigen init --lang go` declares every §7 fn; safe to use but
  inflates the binary slightly (TinyGo's dead-code elimination
  prunes unused imports).
