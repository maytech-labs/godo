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
godo build web -d               # start it in the daemon and return (--json for {"id": N})
godo wait -i 3 --timeout 10m    # exits with job 3's code; 124 if it's still running
godo logs -i 3 --tail 50        # plain text; --from N for what's new, --grep RE
godo kill -i 3
godo mcp                        # serve godo to coding agents over MCP
```

In a terminal, runs go to a background daemon in a pseudo-terminal. You can type into the job, and resizing your terminal resizes it; Ctrl+C detaches and leaves it running, Ctrl+C twice kills it and everything it started. In CI and pipes they run in the foreground with make's exit code. `GODO_DAEMON=1` or `0` overrides that.

Jobs run in a pseudo-terminal, so tools draw colours and progress bars as they would for you. On servers, where nothing reads the output in a terminal, `--no-pty` or `GODO_PTY=0` runs them on plain pipes, with stderr logged apart from stdout.

## Agents

Coding agents can start builds, wait for them, page through their logs, and read exit codes without holding a terminal. Their jobs run in the same daemon as yours, so `godo watch` shows them live, marked `agent`.

**Over MCP.** `godo mcp` is an MCP server on stdin and stdout, for the project in the directory it starts in (or `-C <dir>`). With Claude Code:

```sh
claude mcp add godo -- godo mcp
```

| Tool | What it does |
|---|---|
| `list_targets` | targets and their directives, with `##` descriptions |
| `run` | starts `make <directive>` in a target; waits up to `wait_seconds` (30) and returns the exit code and the end of the output, or the job still running. Returns the same job if it's already running, unless `new` |
| `wait` | waits up to `timeout_seconds` (60) for a job to end |
| `logs` | a job's output as plain text, numbered: `from_line` for what's new since `next_line`, `tail`, `grep` |
| `list_jobs` | this project's jobs, newest first |
| `kill` | stops a job the agent started |

Logs come back as a terminal would draw them, without colours, and with progress bars collapsed to their last state.

**From a shell.** Agents that only have a shell can use the same operations: `godo <directive> <target> -d --json`, `godo wait`, `godo logs`, and `godo kill`, plus `--json` on `ps`, `history`, and `list`.

**Limits.** By default agents can run any directive but can't pass arguments to make (`VAR=value` and flags can make it run anything), and they can only kill jobs that agents started. `godo.toml` can change this. The rules apply to `godo mcp`; an agent with a shell can do whatever the shell lets it.

```toml
[agents]
allow = ["build *", "test *"]   # "directive target"; * matches anything. Default: everything
deny = ["deploy *"]             # checked after allow
make_args = false
kill = "agent"                  # agent, all, or none
```
