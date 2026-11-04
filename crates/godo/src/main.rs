// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! godo: make, but every run is a detachable, logged, watchable job.
//!
//! 0.0.1 only reserves the name. The commands from the C++ godo arrive in
//! later releases; see the repository README.

use std::process::ExitCode;

use clapsmith::{Args, Dispatch, Handler, Parser};

#[derive(Parser, Dispatch)]
#[command(name = "godo", version, about, long_about = None)]
enum Godo {
    /// Print the version
    Version(Version),
}

#[derive(Args)]
struct Version;

impl Handler<()> for Version {
    fn run(self, _: &mut ()) -> clapsmith::Result {
        println!("godo {}", env!("CARGO_PKG_VERSION"));
        Ok(ExitCode::SUCCESS)
    }
}

fn main() -> ExitCode {
    clapsmith::main::<Godo, _>(&mut ())
}
