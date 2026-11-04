// SPDX-License-Identifier: LicenseRef-REAL-1.0

use std::process::ExitCode;

use clapsmith::{Args, Dispatch, Handler, Parser};

#[derive(Parser, Dispatch)]
enum Cli {
    Build(Build),
    Clean(Clean),
}

#[derive(Args)]
struct Build;

#[derive(Args)]
struct Clean;

impl Handler<()> for Build {
    fn run(self, _: &mut ()) -> clapsmith::Result {
        Ok(ExitCode::SUCCESS)
    }
}

// No Handler for Clean, so Cli has no Handler either.
fn main() -> ExitCode {
    clapsmith::main::<Cli, _>(&mut ())
}
