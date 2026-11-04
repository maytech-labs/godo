// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! godo: make, but every run is a detachable, logged, watchable job.
//!
//! The command tree matches the C++ godo. Working so far: running a
//! directive in a target (`godo build web`), the registry commands (`init`,
//! `list`, `register`, `unregister`, `scaff`, `migrate`), and `version`. The
//! daemon commands say they aren't ported yet.

mod config;
mod registry;
mod targets;

use std::ffi::OsString;
use std::process::ExitCode;

use clapsmith::clap::builder::StyledStr;
use clapsmith::{Args, Dispatch, Handler, Parser, Theme};

use crate::config::Config;

/// godo is a blacksmith for all of your build related needs
#[derive(Parser)]
#[command(name = "godo", version, after_help = directive_help())]
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
struct Stop;

#[derive(Args)]
struct Restart;

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
struct Version;

impl Handler<Ctx> for Version {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        println!("godo {}", env!("CARGO_PKG_VERSION"));
        Ok(ExitCode::SUCCESS)
    }
}

/// Commands that exist in the C++ godo but aren't ported yet.
macro_rules! not_ported {
    ($($command:ident => $name:literal),* $(,)?) => {$(
        impl Handler<Ctx> for $command {
            fn run(self, _: &mut Ctx) -> clapsmith::Result {
                Err(concat!("`godo ", $name, "` isn't ported to Rust yet").into())
            }
        }
    )*};
}

not_ported! {
    Ps => "ps", Stop => "stop", Restart => "restart",
    Run => "run", Watch => "watch", History => "history", Attach => "attach",
}

fn main() -> ExitCode {
    clapsmith::main::<Cli, _>(&mut Ctx::default())
}
