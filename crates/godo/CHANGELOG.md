# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/).

## Unreleased

### Added

- **`godo mcp`**: an MCP server so coding agents can use godo without holding a terminal. Its tools are `list_targets`, `run` (waits up to `wait_seconds` and returns the exit code and the end of the output, or the job still running; returns an identical running job instead of starting another), `wait`, `logs` (plain text, numbered lines, `from_line`/`tail`/`grep`), `list_jobs`, and `kill`. `-C <dir>` picks the project.
- **`[agents]` in `godo.toml`** limits what agents may do through `godo mcp`: `allow` and `deny` patterns (`"directive target"`, `*` matches anything), `make_args` (default false), and `kill` (`agent`, the default, `all`, or `none`).
- **`--detach` / `-d`** starts a job in the daemon and returns its ID, in a terminal or not (`--json` prints `{"id": N}`). `godo run -c` takes `--detach` too.
- **`godo wait -i <id> [--timeout 10m] [--tail N]`** exits with the job's exit code, or 124 if it's still running.
- **`godo logs -i <id>`** prints a job's output as plain text, drawn as a terminal would draw it, so progress bars are collapsed to their last state. `--from N` gives what's new since a previous call, plus `--tail N` and `--grep REGEX`.
- **`godo kill -i <id>`.**
- **`--json`** on `ps`, `history`, `list`, `wait`, and `logs`.
- **`--no-pty`**, or `GODO_PTY=0` to make it the default, runs jobs on plain pipes instead of a pseudo-terminal, for servers. stderr is logged apart from stdout.
- Jobs record who started them (a terminal, a shell, or an agent, with the MCP client's name) and their project. `godo watch` marks targets an agent ran last.

## 0.2.0 - 2026-11-04

### Added

- **`godo watch`**: the live dashboard from the C++ godo, ported to ratatui with the same layout and colours. A TARGETS pane lists each `directive target` whose newest job is running or started in the last day (`--since 2h`, `--all`), plus the daemon, with a status dot (green running, red failed, yellow succeeded, yellow ○ no jobs left); it scrolls with the selection and shows `▲/▼ N more`. The LOGS pane follows the selected target and draws output as a terminal would, so progress bars update in place. ↑↓/jk choose a target; PgUp/PgDn, Ctrl+U/Ctrl+D, and the wheel scroll; g/G jump to the top or bottom; q, Esc, or Ctrl+C quits. A footer lists the keys.
- **`godo history`** in a terminal: the C++ godo's full-screen history, newest first. Enter opens a job's logs (followed live if it's running); q goes back. Outside a terminal it prints a table.
- **The daemon.** In a terminal, `godo <directive> <target>` and `godo run -c <cmd> -t <target> [-d <directive>]` run in a per-user background daemon, in a pseudo-terminal. Ctrl+C detaches; Ctrl+C twice quickly kills. Logs and history are kept. Outside a terminal (CI, pipes) runs stay in the foreground; `GODO_DAEMON=1` or `0` overrides.
- `godo ps`, `godo attach -i <id>`, `godo stop`, `godo restart`, `godo history [--clear]`. `ps`, `history`, and `attach` to finished jobs work without the daemon, from its saved history.
- While you follow a job in a terminal, your keystrokes go to it, so prompts and interactive tools work; Ctrl+C stays godo's (once detaches, twice kills). Resizing your terminal resizes the job's.
- godo's flags (`-v`) work anywhere in `godo <directive> <target> ...`. `VAR=value` and plain words still go to make; make's flags go after `--`: `godo build web -- -j8`. An unknown flag before `--` is an error that says so.
- Job output is kept as written, `\r` included, so terminals and `watch` draw progress bars properly.

### Fixed (compared with the C++ godo)

- Jobs run with the working directory, environment, and terminal size of the command that started them, not those the daemon started with.
- Killing a job stops its whole process group, so make's children don't outlive it.
- The daemon lives in a private directory (`$GODO_HOME`, else `$XDG_STATE_HOME/godo`, else `~/.local/state/godo`), not a shared `/tmp/godo.sock` that any local user could connect to. It doesn't clash with a running C++ godo.
- A client and daemon of different versions say so instead of misreading each other.
- Writing to a closed stdout (`godo build web | head`) no longer crashes godo.
- `godo attach` replays a running job's whole output from its log, not only the last megabyte, and the daemon sends each chunk to the log and clients together, so nothing is missed or doubled.
- `godo stop` and `godo restart` refuse while jobs are running and list them; `--force` stops them too.
- A newer godo replaces an idle daemon of another version by itself. One with running jobs is left alone, with a message saying `godo restart --force`.
- The daemon keeps the newest 500 finished jobs and their logs (`GODO_HISTORY_LIMIT` to change), and trims its own log.
- Builds that aren't a release say so: `godo --version` gives `<version>+<commits>.g<sha>`.
- Target names with wide characters no longer misalign `watch`'s selection.

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
