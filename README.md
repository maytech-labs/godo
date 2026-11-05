# godo

[![License: REAL 1.0](https://img.shields.io/badge/License-REAL_1.0-blue.svg)](LICENSE.md)

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](LICENSE.md).

This repository holds two projects that are developed together:

| Crate | What it is | Docs |
|---|---|---|
| [`godo-cli`](crates/godo) | **godo**, the tool: make, but every run is a detachable, logged, watchable job. Installs the `godo` binary. | [README](crates/godo/README.md) |
| [`clapsmith`](crates/clapsmith) | A companion to [clap]: typed dispatch to command handlers, interactive mode, styled output. Usable without godo. | [README](crates/clapsmith/README.md), [docs.rs](https://docs.rs/clapsmith) |
| [`clapsmith-derive`](crates/clapsmith-derive) | clapsmith's derive macros. Use through `clapsmith`. | |

**Status:** godo 0.2.0 runs Makefile targets from anywhere in a project as detachable, logged jobs, with a live dashboard (`godo watch`), with prebuilt binaries on the [releases page](https://github.com/maytech-labs/godo/releases) and in the Homebrew tap. Agent access comes next; the plan is in the [godo README](crates/godo/README.md).

Only interested in clapsmith? Work in `crates/clapsmith`, run `cargo test -p clapsmith`, and filter issues by the `crate:clapsmith` label. See [CONTRIBUTING.md](CONTRIBUTING.md).

[clap]: https://docs.rs/clap
