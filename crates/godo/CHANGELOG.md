# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- `godo <directive> <target> [make args]` runs `make -C <target> <directive>` with `GODO_ROOT` set, in the foreground. Exit codes are make's. Unknown targets get suggestions.
- `godo list` and `godo list -n <target>` (Makefile rules with their `## descriptions`).
- Reads `godo.toml` (`[targets] name = "path"`), or the C++ godo's `.godo/config`, found by walking up from the current directory, so godo works from any subdirectory. If a directory has both, `godo.toml` wins.
- This repo is a godo project: `godo build godo`, `godo check godo`, `godo test clapsmith`.
- The full C++ godo command tree, with `-v/--verbose` as a global flag. Commands not listed above report that they aren't ported yet.

## 0.0.1

### Added

- First release, to reserve the name.
- `godo version`, `--version`, and `--help`.
