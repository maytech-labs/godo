// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! godo: make, but every run is a detachable, logged, watchable job.
//!
//! The command tree matches the C++ godo. Working so far: running a
//! directive in a target (`godo build web`), the registry commands (`init`,
//! `list`, `register`, `unregister`, `scaff`, `migrate`), the daemon (`run`,
//! `ps`, `attach`, `stop`, `restart`, `history`), `watch`, and `version`,
//! plus what agents use: `--detach`, `wait`, `logs`, `kill`, and `mcp`.

mod api;
mod client;
mod config;
mod daemon;
mod history;
mod logs;
mod mcp;
mod proto;
mod registry;
mod targets;
#[cfg(test)]
mod testdir;
mod tui;

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
        "  $ {title}godo{title:#} {dim}<directive> <target> [VAR=value...] [-- make flags]{dim:#}"
    );
    let _ = writeln!(
        out,
        "  $ {title}godo{title:#} {cmd}build{cmd:#} {flag}web{flag:#}"
    );
    let _ = writeln!(
        out,
        "  Runs {cmd}make <directive>{cmd:#} in a registered target's directory."
    );
    let _ = writeln!(
        out,
        "  Built-in commands take precedence over directives. godo's flags (-v, -d to"
    );
    let _ = writeln!(
        out,
        "  detach, --no-pty, --json) can go anywhere; flags for make go after --, e.g."
    );
    let _ = write!(out, "  {cmd}godo build web -- -j8{cmd:#}.");
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
    /// Print a job's output as plain text.
    Logs(Logs),
    /// Wait for a job to end; exits with its exit code.
    Wait(Wait),
    /// Stop a running job and everything it started.
    Kill(Kill),
    /// Serve godo to coding agents over MCP, on stdin and stdout.
    Mcp(Mcp),
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
        let config = ctx.config()?;
        if !self.json {
            return targets::list(config, self.name.as_deref());
        }
        let mut all = api::targets(config);
        if let Some(name) = &self.name {
            all.retain(|t| &t.name == name);
            if all.is_empty() {
                return Err(targets::unknown_target(config, name).into());
            }
        }
        print_json(&all)
    }
}

fn print_json(value: &impl serde::Serialize) -> clapsmith::Result {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(ExitCode::SUCCESS)
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
    /// print targets and their directives as JSON
    #[arg(long)]
    json: bool,
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
struct Ps {
    /// print as JSON
    #[arg(long)]
    json: bool,
}

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
    /// start it in the daemon and return
    #[arg(long)]
    detach: bool,
    /// run on plain pipes, not in a pseudo-terminal
    #[arg(long)]
    no_pty: bool,
}

#[derive(Args)]
struct Watch {
    /// show targets with a job this recent, e.g. 30m, 2h, 7d [default: 24h]
    #[arg(long, value_name = "AGE", conflicts_with = "all")]
    since: Option<String>,
    /// show every target in history
    #[arg(long)]
    all: bool,
}

#[derive(Args)]
struct History {
    /// clear all history entries
    #[arg(long)]
    clear: bool,
    /// print every job as JSON, newest first
    #[arg(long, conflicts_with = "clear")]
    json: bool,
}

#[derive(Args)]
struct Attach {
    /// the id of the process to attach to
    #[arg(short, long)]
    id: String,
}

#[derive(Args)]
struct Logs {
    /// the id of the process
    #[arg(short, long)]
    id: String,
    /// start at this line number (a previous call's `next`)
    #[arg(long, value_name = "LINE")]
    from: Option<usize>,
    /// only the last N lines
    #[arg(long, value_name = "N")]
    tail: Option<usize>,
    /// only lines matching this regex, with their line numbers
    #[arg(long, value_name = "REGEX")]
    grep: Option<String>,
    /// print as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct Wait {
    /// the id of the process
    #[arg(short, long)]
    id: String,
    /// give up after this long, e.g. 90s, 10m (exit code 124)
    #[arg(long, value_name = "AGE")]
    timeout: Option<String>,
    /// print the last N lines of output when it ends
    #[arg(long, value_name = "N")]
    tail: Option<usize>,
    /// print as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct Kill {
    /// the id of the process
    #[arg(short, long)]
    id: String,
}

#[derive(Args)]
struct Mcp {
    /// the godo project to serve [default: the current directory's]
    #[arg(short = 'C', long, value_name = "DIR")]
    dir: Option<std::path::PathBuf>,
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
        let home = proto::Home::resolve()?;
        if self.json {
            let filter = api::JobFilter {
                running: true,
                ..Default::default()
            };
            return print_json(&api::jobs(&home, &filter)?);
        }
        client::ps(&home)
    }
}

impl Handler<Ctx> for Logs {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let home = proto::Home::resolve()?;
        let grep = match &self.grep {
            Some(re) => Some(regex::Regex::new(re).map_err(|e| format!("--grep: {e}"))?),
            None => None,
        };
        let numbered = grep.is_some();
        let query = api::LogQuery {
            from: self.from,
            tail: self.tail,
            grep,
            max: None,
        };
        let page = api::logs(&home, client::parse_id(&self.id)?, &query)?;
        if self.json {
            return print_json(&page);
        }
        let mut out = String::new();
        for line in &page.lines {
            if numbered {
                out.push_str(&format!("{}:{}\n", line.n, line.text));
            } else {
                out.push_str(&line.text);
                out.push('\n');
            }
        }
        if let Some(live) = &page.live {
            out.push_str(live);
            out.push('\n');
        }
        use std::io::Write;
        let _ = std::io::stdout().write_all(out.as_bytes());
        eprintln!(
            "godo: {} lines, job {}{}; next: --from {}",
            page.total,
            if page.running { "running" } else { "ended" },
            if page.omitted > 0 {
                format!(", {} not shown", page.omitted)
            } else {
                String::new()
            },
            page.next
        );
        Ok(ExitCode::SUCCESS)
    }
}

impl Handler<Ctx> for Wait {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let home = proto::Home::resolve()?;
        let id = client::parse_id(&self.id)?;
        let timeout = match &self.timeout {
            Some(t) => Some(watch::parse_duration(t)?),
            None => None,
        };
        let job = api::wait(&home, id, timeout)?;
        let tail = match self.tail {
            Some(n) => {
                let query = api::LogQuery {
                    tail: Some(n),
                    ..Default::default()
                };
                Some(api::logs(&home, id, &query)?)
            }
            None => None,
        };
        let code = match (job.running(), job.exit_code) {
            (true, _) => ExitCode::from(124),
            (false, Some(code)) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            (false, None) => ExitCode::FAILURE,
        };
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({ "job": job, "tail": tail }))?
            );
            return Ok(code);
        }
        if let Some(tail) = &tail {
            for line in &tail.lines {
                println!("{}", line.text);
            }
        }
        let what = format!("Process {id} ({} {})", job.directive, job.target);
        let secs = job.duration_ms as f64 / 1000.0;
        match (job.running(), job.exit_code) {
            (true, _) => println!("{what} is still running after {secs:.1}s"),
            (false, Some(0)) => println!("{what} succeeded in {secs:.1}s"),
            (false, Some(c)) => println!("{what} failed with exit code {c} after {secs:.1}s"),
            (false, None) => println!("{what} stopped when the daemon did"),
        }
        Ok(code)
    }
}

impl Handler<Ctx> for Kill {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let id = client::parse_id(&self.id)?;
        client::kill(&proto::Home::resolve()?, id)?;
        println!("Killing process {id}...");
        Ok(ExitCode::SUCCESS)
    }
}

impl Handler<Ctx> for Mcp {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        let cwd = std::env::current_dir()?;
        mcp::serve(match self.dir {
            Some(dir) => cwd.join(dir),
            None => cwd,
        })
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
        let since = match (self.all, self.since) {
            (true, _) => None,
            (false, Some(age)) => Some(watch::parse_duration(&age)?),
            (false, None) => Some(watch::DEFAULT_SINCE),
        };
        watch::run(proto::Home::resolve()?, since)
    }
}

impl Handler<Ctx> for History {
    fn run(self, _: &mut Ctx) -> clapsmith::Result {
        use std::io::IsTerminal;
        let home = proto::Home::resolve()?;
        if self.json {
            return print_json(&api::jobs(&home, &api::JobFilter::default())?);
        }
        if self.clear {
            client::history_clear(&home)
        } else if std::io::stdout().is_terminal() && std::io::stdin().is_terminal() {
            history::run(home)
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
        if !client::use_daemon() && !self.detach {
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
            pty: !self.no_pty && client::default_pty(),
            origin: proto::Origin::here(),
            client: None,
            project: Config::discover(&std::env::current_dir()?)
                .ok()
                .map(|c| c.root),
        };
        let home = proto::Home::resolve()?;
        if self.detach {
            let (id, _) = client::start(&home, job, false)?;
            println!("Process started with ID: {id}");
            return Ok(ExitCode::SUCCESS);
        }
        client::run(&home, job)
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
