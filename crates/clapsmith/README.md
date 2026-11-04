# clapsmith

A companion to [clap]. clap turns arguments into a typed value; clapsmith runs it.

Source-available under the [Reciprocal Ethical AI License (REAL) 1.0](https://github.com/maytech-labs/godo/blob/main/LICENSE.md). Read it before depending on this crate: it applies to programs built with it.

```rust
use clapsmith::{Args, Dispatch, Handler, Parser};
use std::process::ExitCode;

#[derive(Parser, Dispatch)]
enum Cli {
    /// Build a target
    Build(Build),
}

#[derive(Args)]
struct Build {
    target: String,
}

impl Handler<()> for Build {
    fn run(self, _: &mut ()) -> clapsmith::Result {
        println!("building {}", self.target);
        Ok(ExitCode::SUCCESS)
    }
}

fn main() -> ExitCode {
    clapsmith::main::<Cli, _>(&mut ())
}
```

- **Typed dispatch.** `#[derive(Dispatch)]` sends each subcommand to its `Handler`. A subcommand without a handler is a compile error, not a forgotten `match` arm.
- **A context, not globals.** Handlers get `&mut Ctx`: config, clients, an output buffer in tests.
- **Colourful help out of the box.** clapsmith draws each help screen itself: blue title, cyan headings, magenta flags, green commands, dim placeholders, and `[required]` / `[default: x]` notes. clap's usage errors use the same palette. Colours are dropped when output isn't a terminal. Pick your own with `Theme`, or `Theme::plain()`.
- **Outcomes, not `exit()`.** `dispatch_from` returns `Outcome::{Ran, Help, Usage, Failed}` without printing, so tests can assert on it. `main` prints it and returns the exit code.

## Planned

- **Interactive mode.** A menu when no subcommand is given. Typed prompts for missing values (select for enums, confirm for bools, masked for secrets). Off when not on a terminal. Prints the equivalent command afterwards so you learn the CLI.
- **Styled output beyond help.** The same `Theme` for prompts and your program's own output.
- **More from the command tree.** Completions, man pages, MCP tool definitions.

## Working on clapsmith

```sh
cargo test -p clapsmith
cargo doc -p clapsmith --open
```

[clap]: https://docs.rs/clap
