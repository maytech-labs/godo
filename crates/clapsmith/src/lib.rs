// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! A companion to [clap]. clap turns arguments into a typed value; clapsmith
//! runs it. Each command implements [`Handler`], an enum of commands derives
//! [`Dispatch`], and [`main`] does the rest: parse, print help or errors, run,
//! and return the exit code.
//!
//! ```
//! use clapsmith::{Dispatch, Handler, Parser};
//! use std::process::ExitCode;
//!
//! #[derive(Parser, Dispatch)]
//! enum Cli {
//!     /// Say hello
//!     Hello(Hello),
//! }
//!
//! #[derive(clapsmith::Args)]
//! struct Hello {
//!     #[arg(long, default_value = "world")]
//!     name: String,
//! }
//!
//! impl Handler<Vec<String>> for Hello {
//!     fn run(self, out: &mut Vec<String>) -> clapsmith::Result {
//!         out.push(format!("hello, {}", self.name));
//!         Ok(ExitCode::SUCCESS)
//!     }
//! }
//!
//! let mut out = Vec::new();
//! let args = ["app", "hello", "--name", "godo"];
//! let outcome = clapsmith::dispatch_from::<Cli, _, _, _>(args, &mut out);
//! assert!(matches!(outcome, clapsmith::Outcome::Ran(_)));
//! assert_eq!(out, ["hello, godo"]);
//! ```
//!
//! Interactive mode and styled output are planned; see the README.

use std::ffi::OsString;
use std::process::ExitCode;

pub use clap;
pub use clap::{Args, Parser, Subcommand};
#[cfg(feature = "derive")]
pub use clapsmith_derive::Dispatch;

/// The error a handler can return. Printed as `error: <message>`, with exit code 1.
pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// What a handler returns.
pub type Result<T = ExitCode> = std::result::Result<T, Error>;

/// Runs a parsed command against a context: config, clients, output, or
/// whatever the program needs. Use `()` for none.
pub trait Handler<Ctx> {
    fn run(self, ctx: &mut Ctx) -> Result;
}

/// The result of parsing and running, before anything is printed. Tests
/// assert on this; [`main`] turns it into output and an exit code.
#[derive(Debug)]
pub enum Outcome {
    /// The command ran and returned this code.
    Ran(ExitCode),
    /// Help or version was asked for. `to_string()` gives the plain text.
    Help(clap::Error),
    /// Parsing failed. `to_string()` gives the message with usage.
    Usage(clap::Error),
    /// The handler failed.
    Failed(Error),
}

/// Parses `args` (including the program name) and runs the command.
pub fn dispatch_from<C, Ctx, I, T>(args: I, ctx: &mut Ctx) -> Outcome
where
    C: Parser + Handler<Ctx>,
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match C::try_parse_from(args) {
        Ok(command) => match command.run(ctx) {
            Ok(code) => Outcome::Ran(code),
            Err(error) => Outcome::Failed(error),
        },
        Err(error) if error.use_stderr() => Outcome::Usage(error),
        Err(error) => Outcome::Help(error),
    }
}

/// The whole program: parses `std::env::args_os()`, runs the command with
/// `ctx`, prints help, usage errors, or handler errors, and returns the exit
/// code. `fn main() -> ExitCode { clapsmith::main::<Cli, _>(&mut ()) }`
pub fn main<C, Ctx>(ctx: &mut Ctx) -> ExitCode
where
    C: Parser + Handler<Ctx>,
{
    report(dispatch_from::<C, Ctx, _, _>(std::env::args_os(), ctx))
}

/// Prints an [`Outcome`] the way [`main`] does and returns its exit code.
/// Help goes to stdout and usage errors to stderr, coloured only when that
/// stream is a terminal.
pub fn report(outcome: Outcome) -> ExitCode {
    match outcome {
        Outcome::Ran(code) => code,
        Outcome::Help(error) | Outcome::Usage(error) => {
            // If the stream is closed there is nowhere left to report to.
            let _ = error.print();
            ExitCode::from(u8::try_from(error.exit_code()).unwrap_or(2))
        }
        Outcome::Failed(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
