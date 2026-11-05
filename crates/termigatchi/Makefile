# SPDX-License-Identifier: LicenseRef-REAL-1.0
#
# The `termigatchi` target: `godo dev termigatchi`, `godo test termigatchi`, …
# cargo remains the real build; these are shortcuts.

.PHONY: build release test dev install

build: ## Build termigatchi (debug)
	cargo build -p termigatchi

release: ## Build termigatchi (release) to target/release/termigatchi
	cargo build --release -p termigatchi

test: ## Run termigatchi's tests
	cargo test -p termigatchi

dev: ## Play termigatchi from source; pass arguments with ARGS="..."
	cargo run -q --release -p termigatchi -- $(ARGS)

install: ## Install termigatchi to ~/.cargo/bin
	cargo install --locked --path .
