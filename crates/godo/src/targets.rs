// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Running a directive in a target (`godo build web`) and listing targets.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::config::Config;

/// `godo <directive> <target> [make args...]` runs
/// `make -C <target dir> <directive> [make args...]` with `GODO_ROOT` set to
/// the root. For now make runs in the foreground with godo's terminal; the
/// daemon comes later.
pub fn run(config: &Config, words: &[OsString], verbose: bool) -> clapsmith::Result {
    let [directive, target, extra @ ..] = words else {
        return Err(match words.first() {
            Some(word) => format!(
                "`{}` isn't a godo command. To run a Makefile target, name the target too: godo {} <target>",
                word.to_string_lossy(),
                word.to_string_lossy(),
            ),
            None => "missing directive and target".to_string(),
        }
        .into());
    };
    let name = target.to_string_lossy();
    let Some(target) = config.target(&name) else {
        return Err(unknown_target(config, &name).into());
    };
    let dir = config.dir(target);
    let mut make = Command::new("make");
    // GNU make announces "Entering directory" for -C; that's noise here.
    make.arg("--no-print-directory")
        .arg("-C")
        .arg(&dir)
        .arg(directive)
        .args(extra)
        .env("GODO_ROOT", &config.root);
    if verbose {
        eprintln!(
            "godo: make --no-print-directory -C {} {}{} (GODO_ROOT={}, from {})",
            dir.display(),
            directive.to_string_lossy(),
            extra
                .iter()
                .map(|a| format!(" {}", a.to_string_lossy()))
                .collect::<String>(),
            config.root.display(),
            display(&config.file, &config.root),
        );
    }
    let status = make
        .status()
        .map_err(|error| format!("can't run make: {error}"))?;
    Ok(exit_code(status))
}

pub(crate) fn unknown_target(config: &Config, name: &str) -> String {
    let mut message = format!("no target named `{name}`");
    let close: Vec<&str> = config
        .targets
        .iter()
        .map(|t| t.name.as_str())
        .filter(|t| t.contains(name) || name.contains(*t))
        .collect();
    if !close.is_empty() {
        message.push_str(&format!(". Did you mean: {}?", close.join(", ")));
    }
    message.push_str(" Run `godo list` to see them all.");
    message
}

/// make's exit code, or 128 + signal if it was killed, like a shell.
fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    if let Some(code) = status.code() {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return ExitCode::from(u8::try_from(128 + signal).unwrap_or(1));
        }
    }
    ExitCode::FAILURE
}

/// A Makefile target that `godo list -n` shows.
#[derive(Debug, PartialEq, Eq)]
pub struct MakeTarget {
    pub name: String,
    /// The text after `##` on the rule's line, if any.
    pub about: Option<String>,
}

/// The explicit targets in a Makefile: rule names at the start of a line,
/// skipping special targets (`.PHONY`), pattern rules, paths, variable
/// assignments, and recipe lines.
pub fn make_targets(makefile: &str) -> Vec<MakeTarget> {
    let mut out: Vec<MakeTarget> = Vec::new();
    for line in makefile.lines() {
        let Some(colon) = line.find(':') else {
            continue;
        };
        let name = line[..colon].trim_end();
        let after = &line[colon + 1..];
        let rule = !name.is_empty()
            && !name.starts_with(['.', '#'])
            && !name.contains(|c: char| c.is_whitespace() || "%/$=#".contains(c))
            && !after.starts_with('=');
        if !rule || out.iter().any(|t| t.name == name) {
            continue;
        }
        let about = line
            .split_once("##")
            .map(|(_, about)| about.trim().to_string())
            .filter(|about| !about.is_empty());
        out.push(MakeTarget {
            name: name.to_string(),
            about,
        });
    }
    out
}

/// `godo list`: registered targets, or with `-n`, one target's Makefile targets.
pub fn list(config: &Config, name: Option<&str>) -> clapsmith::Result {
    let style = clapsmith::Theme::default();
    let (cmd, note) = (style.command, style.note);
    let Some(name) = name else {
        if config.targets.is_empty() {
            println!("No targets registered.");
            return Ok(ExitCode::SUCCESS);
        }
        let width = config
            .targets
            .iter()
            .map(|t| t.name.len())
            .max()
            .unwrap_or(0);
        for t in &config.targets {
            anstream::println!("  {cmd}{:width$}{cmd:#}  {note}{}{note:#}", t.name, t.path);
        }
        return Ok(ExitCode::SUCCESS);
    };
    let Some(target) = config.target(name) else {
        return Err(unknown_target(config, name).into());
    };
    let makefile = config.dir(target).join("Makefile");
    let text = std::fs::read_to_string(&makefile)
        .map_err(|e| format!("can't read {}: {e}", display(&makefile, &config.root)))?;
    let targets = make_targets(&text);
    if targets.is_empty() {
        println!("No targets in {}.", display(&makefile, &config.root));
        return Ok(ExitCode::SUCCESS);
    }
    let width = targets.iter().map(|t| t.name.len()).max().unwrap_or(0);
    for t in targets {
        match t.about {
            Some(about) => anstream::println!("  {cmd}{:width$}{cmd:#}  {about}", t.name),
            None => anstream::println!("  {cmd}{}{cmd:#}", t.name),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn display(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(makefile: &str) -> Vec<String> {
        make_targets(makefile).into_iter().map(|t| t.name).collect()
    }

    #[test]
    fn finds_rules_and_their_descriptions() {
        let targets = make_targets("build: ## Build it\n\t@cargo build\ntest: build\n");
        assert_eq!(
            targets,
            [
                MakeTarget {
                    name: "build".into(),
                    about: Some("Build it".into())
                },
                MakeTarget {
                    name: "test".into(),
                    about: None
                },
            ]
        );
    }

    #[test]
    fn skips_what_isnt_a_runnable_target() {
        let makefile = "\
.PHONY: build
BAZEL := bazel
CC=cc
%.o: %.c
out/bin: src
\techo: not a rule
# comment: no
$(X): y
build:
";
        assert_eq!(names(makefile), ["build"]);
    }

    #[test]
    fn lists_each_target_once() {
        assert_eq!(names("a:\na: b\n"), ["a"]);
    }
}
