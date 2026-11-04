# godo

make, but every run is a detachable, logged, watchable job.

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](https://github.com/maytech-labs/godo/blob/main/LICENSE.md).

```sh
brew install maytech-labs/tap/godo
brew install --HEAD maytech-labs/tap/godo    # builds main
cargo install --git https://github.com/maytech-labs/godo godo-cli
```

**Status:** early. Running targets and managing the registry work; jobs run in the foreground for now.

```sh
godo init                       # start a project: writes godo.toml
godo scaff -n web               # web/Makefile, registered as `web`
godo register -n api -p apps/api
godo list                       # registered targets
godo list -n web                # web's Makefile targets and their ## descriptions
godo build web                  # make -C web build, from anywhere in the project
godo migrate                    # convert the C++ godo's .godo/config to godo.toml
```

godo is being ported to Rust from a C++ tool. Planned, in order:

1. **Jobs.** Every run goes to a per-user daemon in a PTY. Ctrl+C detaches, `godo attach` reattaches, and logs and history are kept.
2. **`godo watch`.** A live dashboard of every running job.
3. **Agents.** `godo mcp` lets coding agents start builds, follow logs, and read exit codes without holding a terminal.
