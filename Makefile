# pyde-rust-sdk — local CI mirror.
#
# `make ci` runs the same job matrix the GitHub Actions workflow
# runs, locally. Keep this in lockstep with `.github/workflows/ci.yml`
# so green local = green CI (modulo sibling-repo access).
#
# Run `make help` for the target list.

CARGO        ?= cargo
RUSTUP       ?= rustup

EXAMPLES     := wallet_basics keystore transfer contract_dynamic contract_typed subscribe_logs

.PHONY: help ci fmt fmt-fix clippy build test test-no-fail-fast doc examples \
        run-wallet run-keystore clean toolchain

help: ## Show available targets
	@awk 'BEGIN {FS = ":.*?## "} \
	     /^[a-zA-Z0-9_-]+:.*?##/ { \
	       printf "  \033[36m%-22s\033[0m %s\n", $$1, $$2 \
	     }' $(MAKEFILE_LIST)

ci: fmt clippy build test doc examples ## Full local CI mirror (fmt + clippy + build + test + doc + examples)
	@echo "✔ ci passed"

fmt: ## Check formatting
	$(CARGO) fmt --all -- --check

fmt-fix: ## Apply rustfmt
	$(CARGO) fmt --all

clippy: ## Strict clippy across all targets
	$(CARGO) clippy --workspace --all-targets -- -D warnings

build: ## Build the workspace
	$(CARGO) build --workspace --all-targets

test: ## Full workspace test run
	$(CARGO) test --workspace

test-no-fail-fast: ## Run every test even if some fail (CI-equivalent)
	$(CARGO) test --workspace --no-fail-fast

doc: ## Build docs with warnings denied
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps

examples: ## Build every example binary
	$(CARGO) build --workspace --examples

run-wallet: ## Run the wallet_basics example (no RPC needed)
	$(CARGO) run --example wallet_basics

run-keystore: ## Run the keystore example (no RPC needed)
	$(CARGO) run --example keystore

toolchain: ## Print the active toolchain components
	$(RUSTUP) show

clean: ## Remove target/
	$(CARGO) clean
