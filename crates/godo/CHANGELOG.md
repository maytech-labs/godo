# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

## 0.1.1 - 2026-11-04

### Added

- `godo init [-p path]` creates an empty `godo.toml`.
- `godo register -n name [-p path]` adds or updates a target, and `godo unregister -n name` removes one. Paths are relative to where you run the command and are stored relative to the project root. Edits keep `godo.toml`'s comments and layout.
- `godo scaff -n name [-p path]` creates a directory with a starter Makefile and registers it. It never overwrites an existing Makefile, unlike the C++ godo.
- `godo migrate` writes `godo.toml` from the C++ godo's `.godo/config` and keeps the old file. Projects still on `.godo/config` are asked to migrate before `register`, `unregister`, or `scaff` change anything.

## 0.1.0 - 2026-11-04

### Added

- `godo <directive> <target> [make args]` runs `make -C <target> <directive>` with `GODO_ROOT` set, in the foreground. Exit codes are make's; make runs with `--no-print-directory`. Unknown targets get suggestions.
- `godo list` and `godo list -n <target>` (Makefile rules with their `## descriptions`).
- Reads `godo.toml` (`[targets] name = "path"`), or the C++ godo's `.godo/config`, found by walking up from the current directory, so godo works from any subdirectory. If a directory has both, `godo.toml` wins.
- This repo is a godo project: `godo build godo`, `godo check godo`, `godo test clapsmith`.
- The full C++ godo command tree, with `-v/--verbose` as a global flag. Commands not listed above report that they aren't ported yet.

## 0.0.1

### Added

- First release, to reserve the name.
- `godo version`, `--version`, and `--help`.
