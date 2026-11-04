# godo

make, but every run is a detachable, logged, watchable job.

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](https://github.com/maytech-labs/godo/blob/main/LICENSE.md).

```sh
brew install maytech-labs/tap/godo           # after the first release
brew install --HEAD maytech-labs/tap/godo    # until then: builds main
cargo install --git https://github.com/maytech-labs/godo godo-cli
```

**Status:** early. `godo <directive> <target>` (e.g. `godo build web`) and `godo list` work, in the foreground, reading the C++ godo's `.godo/config`. godo is being ported to Rust from a C++ tool. Planned, in order:

1. **Targets.** Register named Makefile directories and run `godo <directive> <target>` from anywhere in the repo. `godo list` shows each Makefile's `## documented` targets.
2. **Jobs.** Every run goes to a per-user daemon in a PTY. Ctrl+C detaches, `godo attach` reattaches, and logs and history are kept.
3. **`godo watch`.** A live dashboard of every running job.
4. **Agents.** `godo mcp` lets coding agents start builds, follow logs, and read exit codes without holding a terminal.
