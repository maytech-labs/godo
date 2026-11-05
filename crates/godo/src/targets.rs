// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Running a directive in a target (`godo build web`) and listing targets.
//!
//! In a terminal, runs go through the daemon, so they can be detached,
//! attached to, and watched. Elsewhere (CI, pipes, `GODO_DAEMON=0`) make
//! runs in the foreground.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::config::Config;

/// `godo <directive> <target> [make args...]` runs
/// `make -C <target dir> <directive> [make args...]` with `GODO_ROOT` set to
/// the root, through the daemon in a terminal and in the foreground elsewhere.
/// With `--detach` it always goes to the daemon and returns once started.
pub fn run(config: &Config, words: &[OsString], verbose: bool) -> clapsmith::Result {
    let parsed = parse_words(words)?;
    let verbose = verbose || parsed.verbose;
    let name = parsed.target.to_string_lossy().into_owned();
    let directive = parsed.directive.to_string_lossy().into_owned();
    let extra: Vec<String> = parsed
        .extra
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let argv = make_argv(config, &name, &directive, &extra)?;
    if verbose {
        eprintln!(
            "godo: {} (GODO_ROOT={}, from {})",
            argv.join(" "),
            config.root.display(),
            display(&config.file, &config.root),
        );
    }
    if parsed.json && !parsed.detach {
        return Err(
            "--json is for --detach; to follow a job as JSON, use `godo logs --json`".into(),
        );
    }
    let pty = !parsed.no_pty && crate::client::default_pty();
    if parsed.detach {
        let home = crate::proto::Home::resolve()?;
        let job = job(
            argv,
            config,
            &name,
            &directive,
            pty,
            crate::proto::Origin::here(),
            None,
        )?;
        let (id, _) = crate::client::start(&home, job, false)?;
        if parsed.json {
            println!("{}", serde_json::json!({ "id": id }));
        } else {
            println!("Process started with ID: {id} (`godo wait -i {id}`, `godo logs -i {id}`)");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if crate::client::use_daemon() {
        let home = crate::proto::Home::resolve()?;
        let job = job(
            argv,
            config,
            &name,
            &directive,
            pty,
            crate::proto::Origin::here(),
            None,
        )?;
        return crate::client::run(&home, job);
    }
    let mut make = Command::new("make");
    make.args(&argv[1..]).env("GODO_ROOT", &config.root);
    let status = make
        .status()
        .map_err(|error| format!("can't run make: {error}"))?;
    Ok(exit_code(status))
}

/// `make --no-print-directory -C <dir> <directive> [extra...]` for a
/// registered target.
pub fn make_argv(
    config: &Config,
    target: &str,
    directive: &str,
    extra: &[String],
) -> clapsmith::Result<Vec<String>> {
    let Some(t) = config.target(target) else {
        return Err(unknown_target(config, target).into());
    };
    let dir = config.dir(t);
    // GNU make announces "Entering directory" for -C; that's noise here.
    let mut argv: Vec<String> = vec!["make".into(), "--no-print-directory".into(), "-C".into()];
    argv.push(dir.to_string_lossy().into_owned());
    argv.push(directive.to_string());
    argv.extend(extra.iter().cloned());
    Ok(argv)
}

/// `godo <directive> <target> [words...] [-- make args...]`, taken apart.
#[derive(Debug, PartialEq)]
pub struct Words {
    pub directive: OsString,
    pub target: OsString,
    /// For make: plain words and `VAR=value`, then everything after `--`.
    pub extra: Vec<OsString>,
    pub verbose: bool,
    /// `-d`/`--detach`: start it in the daemon and return.
    pub detach: bool,
    /// `--no-pty`: run on plain pipes, not in a pseudo-terminal.
    pub no_pty: bool,
    /// `--json`: with `--detach`, print `{"id": N}`.
    pub json: bool,
}

/// godo's own flags (`-v`, `-d`, `--no-pty`, `--json`) count wherever they are, up to `--`. Other
/// words go to make, but a flag make should get has to come after `--`,
/// so a mistyped godo flag never silently becomes make's.
pub fn parse_words(words: &[OsString]) -> clapsmith::Result<Words> {
    let mut positional: Vec<OsString> = Vec::new();
    let mut after: Vec<OsString> = Vec::new();
    let mut verbose = false;
    let (mut detach, mut no_pty, mut json) = (false, false, false);
    let mut iter = words.iter();
    for word in iter.by_ref() {
        let text = word.to_string_lossy();
        match text.as_ref() {
            "--" => break,
            "-v" | "--verbose" => verbose = true,
            "-d" | "--detach" => detach = true,
            "--no-pty" => no_pty = true,
            "--json" => json = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!(
                    "`{flag}` isn't a godo flag. To pass it to make, put it after `--`: godo {} {} -- {flag}",
                    positional.first().map_or("<directive>".into(), |d| d.to_string_lossy()),
                    positional.get(1).map_or("<target>".into(), |t| t.to_string_lossy()),
                )
                .into());
            }
            _ => positional.push(word.clone()),
        }
    }
    after.extend(iter.cloned());
    let mut positional = positional.into_iter();
    let (Some(directive), target) = (positional.next(), positional.next()) else {
        return Err("missing directive and target".into());
    };
    let Some(target) = target else {
        let word = directive.to_string_lossy();
        return Err(format!(
            "`{word}` isn't a godo command. To run a Makefile target, name the target too: godo {word} <target>"
        )
        .into());
    };
    let mut extra: Vec<OsString> = positional.collect();
    extra.extend(after);
    Ok(Words {
        directive,
        target,
        extra,
        verbose,
        detach,
        no_pty,
        json,
    })
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

/// A daemon job running `argv` here, with this environment plus `GODO_ROOT`.
pub fn job(
    argv: Vec<String>,
    config: &Config,
    target: &str,
    directive: &str,
    pty: bool,
    origin: crate::proto::Origin,
    client: Option<String>,
) -> clapsmith::Result<crate::client::Job> {
    let mut env: Vec<(String, String)> =
        std::env::vars().filter(|(k, _)| k != "GODO_ROOT").collect();
    env.push((
        "GODO_ROOT".into(),
        config.root.to_string_lossy().into_owned(),
    ));
    Ok(crate::client::Job {
        argv,
        cwd: std::env::current_dir()?,
        env,
        target: target.to_string(),
        directive: directive.to_string(),
        pty,
        origin,
        client,
        project: Some(config.root.clone()),
    })
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

    fn words(args: &[&str]) -> clapsmith::Result<Words> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        parse_words(&args)
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn godo_flags_count_anywhere_before_dashdash() {
        let w = words(&["build", "web", "-v"]).unwrap();
        assert!(w.verbose);
        assert!(w.extra.is_empty());
        let w = words(&["build", "-v", "web"]).unwrap();
        assert_eq!(
            (w.directive.as_os_str(), w.target.as_os_str()),
            ("build".as_ref(), "web".as_ref())
        );
        assert!(w.verbose);
    }

    #[test]
    fn make_gets_words_vars_and_everything_after_dashdash() {
        let w = words(&["build", "web", "FOO=1", "test", "--", "-j8", "-v"]).unwrap();
        assert_eq!(w.extra, os(&["FOO=1", "test", "-j8", "-v"]));
        assert!(!w.verbose, "-v after -- is make's");
    }

    #[test]
    fn an_unknown_flag_points_at_dashdash() {
        let err = words(&["build", "web", "-j8"]).unwrap_err().to_string();
        assert!(err.contains("godo build web -- -j8"), "{err}");
    }

    #[test]
    fn a_directive_alone_explains_itself() {
        let err = words(&["dev"]).unwrap_err().to_string();
        assert!(err.contains("godo dev <target>"), "{err}");
        let err = words(&["dev", "-v"]).unwrap_err().to_string();
        assert!(err.contains("godo dev <target>"), "{err}");
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
