# SPDX-License-Identifier: LicenseRef-REAL-1.0
#
# The `clapsmith` target: `godo test clapsmith`, `godo doc clapsmith`.

.PHONY: build test doc

build: ## Build clapsmith and its derive
	cargo build -p clapsmith

test: ## Run clapsmith's tests, including the compile-fail tests
	cargo test -p clapsmith

doc: ## Build clapsmith's docs and open them
	cargo doc -p clapsmith --no-deps --open
