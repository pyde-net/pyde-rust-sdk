<!--
Thanks for the PR!

Before submitting, please read:
  https://github.com/pyde-net/.github/blob/main/CONTRIBUTING.md

Protocol-affecting changes (consensus rules, tx wire format, gas
costs, host fn ABI, etc.) need a PIP first — see
https://github.com/pyde-net/pips
-->

## Summary

<!-- 1-3 bullets on what changed and why. Link the issue / PIP / doc section that motivated it. -->

-
-

## Type of change

<!-- Check the one that fits. -->

- [ ] Bug fix (non-breaking change that fixes documented behaviour)
- [ ] New feature (non-breaking, additive)
- [ ] Breaking change (public API removed / renamed / signature changed)
- [ ] Documentation only
- [ ] Internal refactor (no public-API behaviour change)
- [ ] Test / CI infrastructure
- [ ] Performance / hardening

## Test plan

<!--
What did you do to verify? Concrete commands + expected output
where possible. "I ran cargo test" is fine for trivial PRs; bigger
PRs deserve more.
-->

- [ ] `cargo fmt --check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace --no-fail-fast` passes
- [ ] `cargo test --doc` passes
- [ ] (if touching examples) `cargo build --workspace --examples` clean
- [ ] (if touching live-RPC code) verified end-to-end against `otigen devnet`
- [ ] (if a CHANGELOG-worthy change) added an entry under `## [Unreleased]`

## Wire-format impact

<!--
Skip if your change doesn't touch any borsh-encoded type, RPC
method, or wire constant. Otherwise:
  - Which type changed (e.g. `MultisigTxPayload`, `Tx`)?
  - Does it match the engine's shape exactly?
  - Is there a byte-pin test in this PR?
-->

None — pure SDK-internal change.
<!--
  OR:
  Changed `XYZ` to match engine commit `<sha>`. Byte-pin test:
  `tests/wire_compat.rs::test_xyz_pinned`.
-->

## Breaking changes

<!-- Skip if "Type of change" wasn't "Breaking". Otherwise list every removed / renamed / re-signatured public API. -->

None.

## Additional notes

<!-- Anything reviewers should know — design tradeoffs, alternatives considered, follow-ups intentionally deferred. -->
