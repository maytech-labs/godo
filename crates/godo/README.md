# godo

make, but every run is a detachable, logged, watchable job.

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](https://github.com/maytech-labs/godo/blob/main/LICENSE.md).

```sh
cargo install godo-cli    # installs the `godo` binary
```

**Status:** 0.0.1 holds the name and prints its version. godo is being ported to Rust from a C++ tool. Planned, in order:

1. **Targets.** Register named Makefile directories and run `godo <directive> <target>` from anywhere in the repo. `godo list` shows each Makefile's `## documented` targets.
2. **Jobs.** Every run goes to a per-user daemon in a PTY. Ctrl+C detaches, `godo attach` reattaches, and logs and history are kept.
3. **`godo watch`.** A live dashboard of every running job.
4. **Agents.** `godo mcp` lets coding agents start builds, follow logs, and read exit codes without holding a terminal.
