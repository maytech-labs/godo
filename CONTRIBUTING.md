# Contributing

## Which crate?

| You want to work on | Directory | Commands | Issue label |
|---|---|---|---|
| clapsmith | `crates/clapsmith`, `crates/clapsmith-derive` | `cargo test -p clapsmith` | `crate:clapsmith` |
| godo | `crates/godo` | `cargo run -p godo-cli -- --help` | `crate:godo` |

clapsmith never depends on godo, so it builds and tests on its own.

## Before opening a PR

```sh
cargo xtask check    # what CI runs: fmt, clippy, tests, docs, notices, real-check
```

`real-check` checks the licence setup: `brew install maytech-labs/tap/real-check`. If dependencies change, run `cargo xtask notices` and commit `THIRD-PARTY-NOTICES.md`.

Every source file starts with `// SPDX-License-Identifier: LicenseRef-REAL-1.0`.

Commit messages: scope by crate, e.g. `feat(clapsmith): ...` or `fix(godo): ...`.

## Licence and sign-off

Contributions are under the [REAL 1.0](LICENSE.md) licence, the same as the project. Sign off each commit (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/).
