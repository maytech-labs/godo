# godo

make, but every run is a detachable, logged, watchable job.

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](https://github.com/maytech-labs/godo/blob/main/LICENSE.md).

```sh
brew install maytech-labs/tap/godo
brew install --HEAD maytech-labs/tap/godo    # builds main
cargo install --git https://github.com/maytech-labs/godo godo-cli
```

**Status:** early, and at parity with the C++ godo's targets, jobs, and dashboard.

```sh
godo init                       # start a project: writes godo.toml
godo scaff -n web               # web/Makefile, registered as `web`
godo register -n api -p apps/api
godo list                       # registered targets
godo list -n web                # web's Makefile targets and their ## descriptions
godo build web                  # make -C web build, from anywhere in the project
godo migrate                    # convert the C++ godo's .godo/config to godo.toml
godo watch                      # live dashboard (last day; --since 2h, --all)
godo ps                         # running jobs
godo attach -i 3                # follow job 3; Ctrl+C detaches, twice kills
godo history                    # every job, newest first; enter opens its logs
godo build web -v -- -j8        # godo flags anywhere, make flags after --
```

In a terminal, runs go to a background daemon in a pseudo-terminal. You can type into the job, and resizing your terminal resizes it; Ctrl+C detaches and leaves it running, Ctrl+C twice kills it and everything it started. In CI and pipes they run in the foreground with make's exit code. `GODO_DAEMON=1` or `0` overrides that.

godo was ported to Rust from a C++ tool. Planned next:

1. **Agents.** `godo mcp` lets coding agents start builds, follow logs, and read exit codes without holding a terminal.
