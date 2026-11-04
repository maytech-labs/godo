// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! godo: make, but every run is a detachable, logged, watchable job.
//!
//! The command tree matches the C++ godo. Working so far: running a
//! directive in a target (`godo build web`), the registry commands (`init`,
//! `list`, `register`, `unregister`, `scaff`, `migrate`), the daemon (`run`,
//! `ps`, `attach`, `stop`, `restart`, `history`), `watch`, and `version`.

mod client;
mod config;
mod daemon;
mod logs;
mod proto;
mod registry;
mod targets;
#[cfg(test)]
mod testdir;

#[cfg(test)]
mod version_tests {
    #![allow(dead_code)]
    include!("../build/version.rs");

    #[test]
    fn a_release_build_is_the_plain_version() {
        assert_eq!(
            parse("godo-v0.2.0-0-gabc1234", "0.2.0").as_deref(),
            Some("")
        );
    }

    #[test]
    fn anything_else_names_the_commit() {
        assert_eq!(
            parse("godo-v0.1.1-3-gabc1234", "0.1.1").as_deref(),
            Some("3.gabc1234")
        );
        assert_eq!(
            parse("godo-v0.2.0-0-gabc1234-dirty", "0.2.0").as_deref(),
            Some("0.gabc1234.dirty"),
            "uncommitted changes on the release commit"
        );
        assert_eq!(
            parse("godo-v0.1.1-0-gabc1234", "0.2.0").as_deref(),
            Some("0.gabc1234"),
            "the version was bumped but not tagged"
        );
        assert_eq!(parse("something-else", "0.1.1"), None);
    }
}
mod watch;

use std::ffi::OsString;
use std::process::ExitCode;

use clapsmith::clap::builder::StyledStr;
use clapsmith::{Args, Dispatch, Handler, Parser, Theme};

use crate::config::Config;

/// godo is a blacksmith for all of your build related needs
#[derive(Parser)]
#[command(name = "godo", version = proto::VERSION, after_help = directive_help())]
struct Cli {
    #[command(flatten)]
    global: Global,
    #[command(subcommand)]
    command: Godo,
}

/// The "Targets:" section of `godo --help`, in the same style as the
/// sections clapsmith draws.
fn directive_help() -> StyledStr {
    use std::fmt::Write;
    let t = Theme::default();
    let (title, cmd, flag, dim) = (t.title, t.command, t.flag, t.placeholder);
    let mut out = StyledStr::new();
    let _ = writeln!(out, "{}Targets:{:#}", t.heading, t.heading);
    let _ = writeln!(
        out,
        "  $ {title}godo{title:#} {dim}<directive> <target> [make args]{dim:#}"
    );
    let _ = writeln!(
        out,
        "  $ {title}godo{title:#} {cmd}build{cmd:#} {flag}web{flag:#}"
    );
    let _ = writeln!(
        out,
        "  Runs {cmd}make <directive>{cmd:#} in a registered target's directory."
    );
    let _ = write!(out, "  Built-in commands take precedence over directives.");
    out
}

/// What every handler gets.
#[derive(Default)]
struct Ctx {
    verbose: bool,
    config: Option<Config>,
}

impl Ctx {
    /// The registry, read on first use so commands that don't need it work
    /// outside a godo project.
    fn config(&mut self) -> clapsmith::Result<&Config> {
        if self.config.is_none() {
            let cwd = std::env::current_dir()?;
            self.config = Some(Config::discover(&cwd)?);
        }
        Ok(self.config.as_ref().expect("just set"))
    }
}

impl Handler<Ctx> for Cli {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        ctx.verbose = self.global.verbose;
        self.command.run(ctx)
    }
}

#[derive(clapsmith::Subcommand, Dispatch)]
enum Godo {
    /// Initialize godo in the current directory or a specified path.
    Init(Init),
    /// List all build targets.
    List(List),
    /// Set up scaffolding for a new target.
    Scaff(Scaff),
    /// Register new build targets.
    Register(Register),
    /// Unregister existing build target.
    Unregister(Unregister),
    /// Convert the C++ godo's .godo/config to godo.toml.
    Migrate(Migrate),
    /// List running processes in the daemon.
    Ps(Ps),
    /// Stop the godo daemon.
    Stop(Stop),
    /// Restart the godo daemon.
    Restart(Restart),
    /// Run an arbitrary command in the daemon.
    Run(Run),
    /// Real-time TUI for monitoring processes.
    Watch(Watch),
    /// View history of executed commands.
    History(History),
    /// Attach to a running process in the daemon.
    Attach(Attach),
    /// Run the godo daemon in the foreground (started automatically).
    #[command(hide = true)]
    Daemon(DaemonCmd),
    /// Print the version.
    Version(Version),
    /// `godo <directive> <target>`: anything that isn't a command above.
    #[command(external_subcommand)]
    Directive(Vec<OsString>),
}

impl Handler<Ctx> for Vec<OsString> {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        let verbose = ctx.verbose;
        targets::run(ctx.config()?, &self, verbose)
    }
}

impl Handler<Ctx> for List {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        targets::list(ctx.config()?, self.name.as_deref())
    }
}

impl Handler<Ctx> for Init {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        registry::init(&std::env::current_dir()?, self.path.as_deref())
    }
}

impl Handler<Ctx> for Register {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        let cwd = std::env::current_dir()?;
        registry::register(ctx.config()?, &cwd, &self.name, self.path.as_deref())
    }
}

impl Handler<Ctx> for Unregister {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        registry::unregister(ctx.config()?, &self.name)
    }
}

impl Handler<Ctx> for Scaff {
    fn run(self, ctx: &mut Ctx) -> clapsmith::Result {
        let cwd = std::env::current_dir()?;
        registry::scaff(ctx.config()?, &cwd, &self.name, self.path.as_deref())
    }
}

impl Handler<Ctx> for Migrate {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        registry::migrate(&std::env::current_dir()?)
    }
}

/// Flags every command accepts.
#[derive(Args)]
struct Global {
    /// display verbose logging output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Args)]
struct Init {
    /// the directory to initialize godo in [default: here]
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(Args)]
struct List {
    /// the name of the target
    #[arg(short, long)]
    name: Option<String>,
}

#[derive(Args)]
struct Scaff {
    /// the name of the target
    #[arg(short, long)]
    name: String,
    /// the directory to create [default: ./<name>]
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(Args)]
struct Register {
    /// the name of the target
    #[arg(short, long)]
    name: String,
    /// the target's directory [default: here]
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(Args)]
struct Unregister {
    /// the name of the target
    #[arg(short, long)]
    name: String,
}

#[derive(Args)]
struct Migrate;

#[derive(Args)]
struct Ps;

#[derive(Args)]
struct Stop {
    /// stop running jobs too
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct Restart {
    /// stop running jobs too
    #[arg(short, long)]
    force: bool,
}

#[derive(Args)]
struct Run {
    /// the command to run
    #[arg(short, long)]
    command: String,
    /// the logical target name for grouping logs
    #[arg(short, long)]
    target: String,
    /// the directive name for grouping logs
    #[arg(short, long, default_value = "run")]
    directive: String,
}

#[derive(Args)]
struct Watch;

#[derive(Args)]
struct History {
    /// clear all history entries
    #[arg(long)]
    clear: bool,
}

#[derive(Args)]
struct Attach {
    /// the id of the process to attach to
    #[arg(short, long)]
    id: String,
}

#[derive(Args)]
struct DaemonCmd;

impl Handler<Ctx> for DaemonCmd {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        daemon::run(proto::Home::resolve()?)?;
        Ok(ExitCode::SUCCESS)
    }
}

impl Handler<Ctx> for Ps {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        client::ps(&proto::Home::resolve()?)
    }
}

impl Handler<Ctx> for Stop {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        client::stop(&proto::Home::resolve()?, self.force)
    }
}

impl Handler<Ctx> for Restart {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        client::restart(&proto::Home::resolve()?, self.force)
    }
}

impl Handler<Ctx> for Watch {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        watch::run(proto::Home::resolve()?)
    }
}

impl Handler<Ctx> for History {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let home = proto::Home::resolve()?;
        if self.clear {
            client::history_clear(&home)
        } else {
            client::history(&home)
        }
    }
}

impl Handler<Ctx> for Attach {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        client::attach(&proto::Home::resolve()?, &self.id)
    }
}

impl Handler<Ctx> for Run {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let argv = vec!["sh".to_string(), "-c".to_string(), self.command];
        if !client::use_daemon() {
            let status = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .status()
                .map_err(|e| format!("can't run sh: {e}"))?;
            return Ok(ExitCode::from(
                u8::try_from(status.code().unwrap_or(1)).unwrap_or(1),
            ));
        }
        let job = client::Job {
            argv,
            cwd: std::env::current_dir()?,
            env: std::env::vars().collect(),
            target: self.target,
            directive: self.directive,
        };
        client::run(&proto::Home::resolve()?, job)
    }
}

#[derive(Args)]
struct Version;

impl Handler<Ctx> for Version {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        println!("godo {}", proto::VERSION);
        Ok(ExitCode::SUCCESS)
    }
}

fn main() -> ExitCode {
    clapsmith::main::<Cli, _>(&mut Ctx::default())
}
