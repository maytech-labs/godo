// SPDX-License-Identifier: LicenseRef-REAL-1.0

use std::process::ExitCode;

use clapsmith::{Args, Dispatch, Handler, Outcome, Parser, dispatch_from};

#[derive(Parser, Dispatch)]
#[command(name = "tool", version = "1.2.3")]
enum Tool {
    /// Add two numbers
    Add(Add),
    /// Always fails
    Fail(Fail),
}

#[derive(Args)]
struct Add {
    a: i64,
    b: i64,
}

#[derive(Args)]
struct Fail;

#[derive(Default)]
struct Ctx {
    sums: Vec<i64>,
}

impl Handler<Ctx> for Add {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        ctx.sums.push(self.a + self.b);
        Ok(ExitCode::SUCCESS)
    }
}

impl Handler<Ctx> for Fail {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        Err("it broke".into())
    }
}

fn run(args: &[&str]) -> (Outcome, Ctx) {
    let mut ctx = Ctx::default();
    let argv = std::iter::once("tool").chain(args.iter().copied());
    let outcome = dispatch_from::<Tool, _, _, _>(argv, &mut ctx);
    (outcome, ctx)
}

#[test]
fn dispatches_to_the_matching_handler() {
    let (outcome, ctx) = run(&["add", "2", "3"]);
    assert!(matches!(outcome, Outcome::Ran(_)));
    assert_eq!(ctx.sums, [5]);
}

#[test]
fn handler_errors_are_returned_not_printed() {
    let (outcome, _) = run(&["fail"]);
    let Outcome::Failed(error) = outcome else {
        panic!("expected Failed, got {outcome:?}");
    };
    assert_eq!(error.to_string(), "it broke");
}

#[test]
fn help_and_version_are_outcomes() {
    let (help, _) = run(&["--help"]);
    let Outcome::Help(text) = help else {
        panic!("expected Help, got {help:?}");
    };
    assert!(text.to_string().contains("Add two numbers"));
    assert_eq!(text.exit_code(), 0);
    let (version, _) = run(&["--version"]);
    assert!(matches!(version, Outcome::Help(text) if text.to_string().contains("1.2.3")));
}

#[test]
fn usage_errors_carry_clap_exit_code() {
    let (outcome, ctx) = run(&["add", "two", "3"]);
    assert!(
        matches!(&outcome, Outcome::Usage(e) if e.exit_code() == 2),
        "{outcome:?}"
    );
    assert!(ctx.sums.is_empty());
}

#[test]
fn plain_text_has_no_colour_codes() {
    let (outcome, _) = run(&["bogus"]);
    let Outcome::Usage(error) = outcome else {
        panic!("expected Usage, got {outcome:?}");
    };
    assert!(!error.to_string().contains('\x1b'));
}
