# Contributing

## Which crate?

| You want to work on | Directory | Commands | Issue label |
|---|---|---|---|
| clapsmith | `crates/clapsmith`, `crates/clapsmith-derive` | `cargo test -p clapsmith` | `crate:clapsmith` |
| godo | `crates/godo` | `cargo run -p godo-cli -- --help` | `crate:godo` |

clapsmith never depends on godo, so it builds and tests on its own.

## godo builds itself

This repo is a godo project (`godo.toml`). With godo installed: `godo list -n godo`, `godo build godo`, `godo check godo`, `godo test clapsmith`. Plain cargo works the same.

## Before opening a PR

```sh
cargo xtask check    # what CI runs: fmt, clippy, tests, docs, notices, real-check
```

`real-check` checks the licence setup: `brew install maytech-labs/tap/real-check`. If dependencies change, run `cargo xtask notices` and commit `THIRD-PARTY-NOTICES.md`.

Every source file starts with `// SPDX-License-Identifier: LicenseRef-REAL-1.0`.

Commit messages: scope by crate, e.g. `feat(clapsmith): ...` or `fix(godo): ...`.

## Licence and sign-off

Contributions are under the [REAL 1.0](LICENSE.md) licence, the same as the project. Sign off each commit (`git commit -s`) to certify the [Developer Certificate of Origin](https://developercertificate.org/).

## Releasing godo

1. Make sure `crates/godo/CHANGELOG.md` has an `Unreleased` section with entries.
2. Optional dry run: Actions → **Release godo** → Run workflow. It checks, builds, and packages every platform at the current version and uploads the archives as artifacts, without releasing.
3. Tag main's latest commit and push the tag:

   ```sh
   git tag godo-v0.1.0 && git push origin godo-v0.1.0
   ```

The workflow sets `crates/godo`'s version, dates the changelog, and commits that to main. It then builds macOS and Linux (arm and intel) binaries, publishes a GitHub release, and bumps `Formula/godo.rb` in [maytech-labs/homebrew-tap](https://github.com/maytech-labs/homebrew-tap). The last step needs the `HOMEBREW_TAP_TOKEN` secret: a fine-grained token with contents write on the tap only.

clapsmith releases separately (to crates.io) with `clapsmith-v*` tags. That isn't automated yet.
