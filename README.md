# godo

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](LICENSE.md).

This repository holds two projects that are developed together:

| Crate | What it is | Docs |
|---|---|---|
| [`godo-cli`](crates/godo) | **godo**, the tool: make, but every run is a detachable, logged, watchable job. Installs the `godo` binary. | [README](crates/godo/README.md) |
| [`clapsmith`](crates/clapsmith) | A companion to [clap]: typed dispatch to command handlers, interactive mode, styled output. Usable without godo. | [README](crates/clapsmith/README.md), [docs.rs](https://docs.rs/clapsmith) |
| [`clapsmith-derive`](crates/clapsmith-derive) | clapsmith's derive macros. Use through `clapsmith`. | |

**Status:** 0.0.1, early. The plan is in the godo README.

Only interested in clapsmith? Work in `crates/clapsmith`, run `cargo test -p clapsmith`, and filter issues by the `crate:clapsmith` label. See [CONTRIBUTING.md](CONTRIBUTING.md).

[clap]: https://docs.rs/clap
