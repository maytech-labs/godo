// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Repo automation, run with `cargo xtask <task>`.

mod notices;

use std::process::{Command, ExitCode};

const USAGE: &str = "\
cargo xtask <task>

Tasks:
  check    Everything CI runs: fmt, clippy, tests, docs, third-party notices, real-check
  notices  Regenerate THIRD-PARTY-NOTICES.md from Cargo.lock
";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("check") => check(),
        Some("notices") => notices::run(std::env::args().skip(2).collect()),
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Runs the same steps as CI, stopping at the first failure.
fn check() -> ExitCode {
    let cargo = env!("CARGO");
    let steps: &[(&str, &[&str])] = &[
        (cargo, &["fmt", "--all", "--check"]),
        (
            cargo,
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        (cargo, &["test", "--workspace"]),
        // clapsmith must stand alone: built, tested, and documented without godo.
        (
            cargo,
            &[
                "test",
                "--package",
                "clapsmith",
                "--no-default-features",
                "--lib",
            ],
        ),
        (cargo, &["doc", "--package", "clapsmith", "--no-deps"]),
        (cargo, &["xtask", "notices", "--check"]),
        ("real-check", &["repo", "."]),
    ];
    for (program, args) in steps {
        let shown = if *program == cargo { "cargo" } else { program };
        eprintln!("==> {shown} {}", args.join(" "));
        let ok = Command::new(program)
            .args(*args)
            .status()
            .is_ok_and(|s| s.success());
        if !ok {
            eprintln!("{shown} {} failed", args[0]);
            if *program == "real-check" {
                eprintln!("install it with `brew install maytech-labs/tap/real-check`");
            }
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
