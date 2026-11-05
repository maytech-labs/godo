// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! `godo mcp`: godo for coding agents, as an MCP server on stdin and
//! stdout. Agents list targets, start builds without holding a terminal,
//! wait for them with a timeout, page through their logs as plain text,
//! and read exit codes.
//!
//! Jobs run in the same daemon as everyone else's, marked as started by an
//! agent, so `godo watch` shows them live. The project is the one godo
//! finds from the directory `godo mcp` starts in, re-read on every call.
//! `[agents]` in `godo.toml` limits what agents may run and kill
//! ([`Agents`](crate::config::Agents)).

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::api::{self, JobView, LogPage, LogQuery};
use crate::config::{Config, Kill};
use crate::proto::{Home, Origin};

/// How long `run` waits by default before returning a running job.
const RUN_WAIT: u64 = 30;
/// How long `wait` waits by default.
const WAIT: u64 = 60;
/// The longest any call waits, under typical MCP client timeouts.
const MAX_WAIT: u64 = 600;
/// Lines of output returned when a job ends.
const TAIL: usize = 40;
/// Lines `logs` returns by default, and at most.
const LOG_LINES: usize = 200;
const MAX_LOG_LINES: usize = 2000;

const INSTRUCTIONS: &str = "\
godo runs Makefile targets in a godo project as background jobs. \
`godo <directive> <target>` is `make <directive>` in the target's directory. \
Use list_targets to see what can run, then run. Builds that take longer than \
wait_seconds come back running: call wait (or logs with from_line = the \
previous next_line) until they end. Jobs show live in the user's `godo watch`.";

pub fn serve(cwd: PathBuf) -> clapsmith::Result {
    let home = Home::resolve()?;
    let server = Godo {
        cwd,
        home,
        tool_router: Godo::tool_router(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("godo mcp: {e}"))?;
        service
            .waiting()
            .await
            .map_err(|e| format!("godo mcp: {e}"))?;
        Ok::<_, clapsmith::Error>(())
    })?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Clone)]
struct Godo {
    cwd: PathBuf,
    home: Home,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
struct RunParams {
    /// A registered target, from list_targets.
    target: String,
    /// The Makefile target to run in it, e.g. build or test.
    directive: String,
    /// Extra arguments for make: VAR=value, more goals, or flags. Only if
    /// the project allows it (`[agents] make_args = true`).
    #[serde(default)]
    args: Vec<String>,
    /// How long to wait for it to end before returning it running.
    /// Default 30, at most 600.
    wait_seconds: Option<u64>,
    /// Run on plain pipes instead of a pseudo-terminal. Default false.
    #[serde(default)]
    no_pty: bool,
    /// Start another even if the same job is already running. By default
    /// the running one is returned.
    #[serde(default)]
    new: bool,
}

#[derive(Deserialize, JsonSchema)]
struct WaitParams {
    /// The job's id.
    id: u64,
    /// How long to wait. Default 60, at most 600.
    timeout_seconds: Option<u64>,
    /// Lines of output to return from the end. Default 40.
    tail_lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct LogsParams {
    /// The job's id.
    id: u64,
    /// The first line number wanted. Pass a previous call's next_line to
    /// get only what's new.
    from_line: Option<usize>,
    /// Only the last N lines (after grep).
    tail: Option<usize>,
    /// Only lines matching this regex, e.g. "error|warning".
    grep: Option<String>,
    /// At most this many lines. Default 200, at most 2000.
    max_lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct JobsParams {
    /// Only running jobs.
    #[serde(default)]
    running_only: bool,
    /// Jobs from every project, not just this one.
    #[serde(default)]
    all_projects: bool,
    /// At most this many, newest first. Default 20.
    limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
struct KillParams {
    /// The job's id.
    id: u64,
}

type ToolResult = Result<CallToolResult, ErrorData>;

/// Runs blocking godo work off the async runtime. Its errors go to the
/// agent as a failed tool call, which it can read and act on.
async fn blocking(f: impl FnOnce() -> Result<String, String> + Send + 'static) -> ToolResult {
    let result = tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
    Ok(match result {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(text) => CallToolResult::error(vec![ContentBlock::text(text)]),
    })
}

fn config(cwd: &std::path::Path) -> Result<Config, String> {
    Config::discover(cwd).map_err(|e| e.to_string())
}

fn seconds(asked: Option<u64>, default: u64) -> Duration {
    Duration::from_secs(asked.unwrap_or(default).min(MAX_WAIT))
}

#[tool_router]
impl Godo {
    #[tool(
        description = "The project's targets and the directives (Makefile targets) each can run, with their ## descriptions."
    )]
    async fn list_targets(&self) -> ToolResult {
        let cwd = self.cwd.clone();
        blocking(move || {
            let config = config(&cwd)?;
            let mut out = format!("project {}\n", config.root.display());
            for t in api::targets(&config) {
                let _ = writeln!(out, "\n{} ({})", t.name, t.path);
                if let Some(e) = t.error {
                    let _ = writeln!(out, "  {e}");
                }
                for d in t.directives {
                    if !config.agents.allows(&d.name, &t.name) {
                        continue;
                    }
                    match d.about {
                        Some(about) => {
                            let _ = writeln!(out, "  {}: {about}", d.name);
                        }
                        None => {
                            let _ = writeln!(out, "  {}", d.name);
                        }
                    }
                }
            }
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Run `make <directive>` in a target as a background job. Waits up to wait_seconds; returns the exit code and the end of the output if it ended, else the job id to pass to wait or logs. If the same job is already running, returns that one unless new is true."
    )]
    async fn run(
        &self,
        Parameters(p): Parameters<RunParams>,
        ctx: RequestContext<RoleServer>,
    ) -> ToolResult {
        let (cwd, home) = (self.cwd.clone(), self.home.clone());
        let client = ctx.client_info().map(|c| c.name);
        blocking(move || {
            let config = config(&cwd)?;
            if !config.agents.allows(&p.directive, &p.target) {
                return Err(format!(
                    "this project doesn't let agents run `{} {}`; see [agents] in {}",
                    p.directive,
                    p.target,
                    config.file.display()
                ));
            }
            if !p.args.is_empty() && !config.agents.make_args {
                return Err(format!(
                    "this project doesn't let agents pass arguments to make; run without args, or set `[agents] make_args = true` in {}",
                    config.file.display()
                ));
            }
            let started = api::start(
                &home,
                &config,
                api::Start {
                    target: &p.target,
                    directive: &p.directive,
                    args: &p.args,
                    pty: !p.no_pty && crate::client::default_pty(),
                    origin: Origin::Agent,
                    client,
                    reuse: !p.new,
                },
            )
            .map_err(|e| e.to_string())?;
            let job = api::wait(&home, started.id, Some(seconds(p.wait_seconds, RUN_WAIT)))
                .map_err(|e| e.to_string())?;
            let mut out = String::new();
            if started.reused {
                out.push_str("already running; returning that job\n");
            }
            report(&home, &job, TAIL, &mut out)?;
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Wait for a job to end, up to timeout_seconds. Returns its status and exit code, and the end of its output if it ended."
    )]
    async fn wait(&self, Parameters(p): Parameters<WaitParams>) -> ToolResult {
        let home = self.home.clone();
        blocking(move || {
            let job = api::wait(&home, p.id, Some(seconds(p.timeout_seconds, WAIT)))
                .map_err(|e| e.to_string())?;
            let mut out = String::new();
            report(&home, &job, p.tail_lines.unwrap_or(TAIL), &mut out)?;
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "A job's output as plain text, with line numbers. Without from_line or tail, the last max_lines lines. Pass from_line = the previous call's next_line for only what's new; grep to search."
    )]
    async fn logs(&self, Parameters(p): Parameters<LogsParams>) -> ToolResult {
        let home = self.home.clone();
        blocking(move || {
            let grep = match &p.grep {
                Some(re) => Some(regex::Regex::new(re).map_err(|e| format!("grep: {e}"))?),
                None => None,
            };
            let query = LogQuery {
                from: p.from_line,
                tail: p.tail,
                grep,
                max: Some(p.max_lines.unwrap_or(LOG_LINES).min(MAX_LOG_LINES)),
            };
            let page = api::logs(&home, p.id, &query).map_err(|e| e.to_string())?;
            let job = api::job(&home, p.id).map_err(|e| e.to_string())?;
            let mut out = String::new();
            summary(&job, &mut out);
            write_page(&page, &mut out);
            Ok(out)
        })
        .await
    }

    #[tool(
        description = "Jobs in this project, newest first: running and finished, with exit codes."
    )]
    async fn list_jobs(&self, Parameters(p): Parameters<JobsParams>) -> ToolResult {
        let (cwd, home) = (self.cwd.clone(), self.home.clone());
        blocking(move || {
            let project = if p.all_projects {
                None
            } else {
                Some(config(&cwd)?.root)
            };
            let filter = api::JobFilter {
                project,
                running: p.running_only,
                limit: Some(p.limit.unwrap_or(20)),
            };
            let jobs = api::jobs(&home, &filter).map_err(|e| e.to_string())?;
            if jobs.is_empty() {
                return Ok("no jobs".into());
            }
            let mut out = String::new();
            for job in &jobs {
                summary(job, &mut out);
            }
            Ok(out)
        })
        .await
    }

    #[tool(description = "Stop a running job and everything it started.")]
    async fn kill(&self, Parameters(p): Parameters<KillParams>) -> ToolResult {
        let (cwd, home) = (self.cwd.clone(), self.home.clone());
        blocking(move || {
            let config = config(&cwd)?;
            let job = api::job(&home, p.id).map_err(|e| e.to_string())?;
            let allowed = match config.agents.kill {
                Kill::All => true,
                Kill::Agent => job.origin == Origin::Agent.as_str(),
                Kill::None => false,
            };
            if !allowed {
                return Err(format!(
                    "this project doesn't let agents kill job {} (started {}); see `[agents] kill` in {}",
                    p.id,
                    started_by(job.origin),
                    config.file.display()
                ));
            }
            if !job.running() {
                return Err(format!("job {} isn't running: it {}", p.id, job.status));
            }
            api::kill(&home, p.id).map_err(|e| e.to_string())?;
            Ok(format!("stopping job {}", p.id))
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Godo {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("godo", crate::proto::VERSION))
            .with_instructions(INSTRUCTIONS)
    }
}

/// One line about a job: `job 12: build web: failed (exit 2) after 3.1s`.
fn summary(job: &JobView, out: &mut String) {
    let secs = job.duration_ms as f64 / 1000.0;
    let status = match (job.status, job.exit_code) {
        ("running", _) => format!("running for {secs:.1}s"),
        ("success", _) => format!("succeeded after {secs:.1}s"),
        (_, Some(code)) => format!("failed (exit {code}) after {secs:.1}s"),
        _ => "stopped when the daemon did".into(),
    };
    let _ = writeln!(
        out,
        "job {}: {} {}: {status}; started {} {}",
        job.id,
        job.directive,
        job.target,
        job.started,
        started_by(job.origin)
    );
}

fn started_by(origin: &str) -> &'static str {
    match origin {
        "agent" => "by an agent",
        "shell" => "from a shell",
        _ => "from a terminal",
    }
}

/// A job's summary, then the end of its output, or how to follow it.
fn report(home: &Home, job: &JobView, tail: usize, out: &mut String) -> Result<(), String> {
    summary(job, out);
    let query = LogQuery {
        tail: Some(tail),
        ..Default::default()
    };
    let page = api::logs(home, job.id, &query).map_err(|e| e.to_string())?;
    write_page(&page, out);
    if job.running() {
        let _ = writeln!(
            out,
            "still running: call wait with id {}, or logs with from_line {}",
            job.id, page.next
        );
    }
    Ok(())
}

fn write_page(page: &LogPage, out: &mut String) {
    match (page.lines.first(), page.lines.last()) {
        (Some(first), Some(last)) => {
            let _ = write!(out, "output lines {}-{} of {}", first.n, last.n, page.total);
        }
        _ => {
            let _ = write!(out, "no output lines to show ({} in all)", page.total);
        }
    }
    if page.omitted > 0 {
        let _ = write!(out, ", {} not shown", page.omitted);
    }
    let _ = writeln!(out, "; next_line {}", page.next);
    let width = page.lines.last().map_or(1, |l| l.n.to_string().len());
    for line in &page.lines {
        let _ = writeln!(out, "{:>width$}| {}", line.n, line.text);
    }
    if let Some(live) = &page.live {
        let _ = writeln!(out, "{:>width$}| {live}", "now");
    }
}
