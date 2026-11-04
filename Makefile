# SPDX-License-Identifier: LicenseRef-REAL-1.0
#
# The `godo` target: `godo build godo`, `godo check godo`, and so on.
# cargo remains the real build; these are shortcuts.

.PHONY: build release test check dev install clean

build: ## Build the godo binary (debug)
	cargo build -p godo-cli

release: ## Build the godo binary (release) to target/release/godo
	cargo build --release -p godo-cli

test: ## Run every test in the workspace
	cargo test --workspace

check: ## Everything CI runs: fmt, clippy, tests, docs, notices, real-check
	cargo xtask check

dev: ## Run godo from source; pass arguments with ARGS="..."
	cargo run -q -p godo-cli -- $(ARGS)

install: ## Install godo to ~/.cargo/bin
	cargo install --locked --path crates/godo

clean: ## Remove build output
	cargo clean
