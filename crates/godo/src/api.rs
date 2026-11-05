// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! What agents can do with godo, shared by the CLI (`--detach`, `godo
//! wait`, `godo logs`, `--json`) and `godo mcp`. Everything here returns
//! data; callers decide how to show it.
//!
//! Logs come back as plain text, drawn the way a terminal would draw them
//! ([`LogView`](crate::tui::LogView)): colours dropped, progress bars
//! collapsed to their last state. Lines are numbered from 1 so a caller
//! can ask for what's new since the last call.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use crate::config::Config;
use crate::proto::{Home, Origin, ProcessInfo};

#[derive(Serialize, Debug)]
pub struct TargetInfo {
    pub name: String,
    pub path: String,
    /// The Makefile's targets, which `godo <directive> <target>` runs.
    pub directives: Vec<DirectiveInfo>,
    /// Why the directives couldn't be read, if they couldn't.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct DirectiveInfo {
    pub name: String,
    /// The `## description` on the rule's line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<String>,
}

/// Every registered target and its directives.
pub fn targets(config: &Config) -> Vec<TargetInfo> {
    config
        .targets
        .iter()
        .map(|t| {
            let makefile = config.dir(t).join("Makefile");
            let (directives, error) = match std::fs::read_to_string(&makefile) {
                Ok(text) => (
                    crate::targets::make_targets(&text)
                        .into_iter()
                        .map(|m| DirectiveInfo {
                            name: m.name,
                            about: m.about,
                        })
                        .collect(),
                    None,
                ),
                Err(e) => (
                    Vec::new(),
                    Some(format!("can't read {}: {e}", makefile.display())),
                ),
            };
            TargetInfo {
                name: t.name.clone(),
                path: t.path.clone(),
                directives,
                error,
            }
        })
        .collect()
}

/// A job, as agents and `--json` see it.
#[derive(Serialize, Debug, Clone)]
pub struct JobView {
    pub id: u64,
    pub target: String,
    pub directive: String,
    pub command: String,
    /// `running`, `success`, `failed`, or `stopped` (ended with the daemon).
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub origin: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
    pub pty: bool,
    /// Local time, `YYYY-MM-DD HH:MM:SS`.
    pub started: String,
    pub start_ms: u64,
    /// So far, if it's still running.
    pub duration_ms: u64,
}

impl From<&ProcessInfo> for JobView {
    fn from(p: &ProcessInfo) -> Self {
        let end = p.end_ms.unwrap_or_else(crate::proto::now_ms);
        Self {
            id: p.id,
            target: p.target.clone(),
            directive: p.directive.clone(),
            command: p.command.clone(),
            status: match crate::client::status(p) {
                "Running" => "running",
                "Success" => "success",
                "Failed" => "failed",
                _ => "stopped",
            },
            exit_code: p.exit_code,
            origin: p.origin.as_str(),
            client: p.client.clone(),
            project: p.project.clone(),
            pty: p.pty,
            started: crate::proto::local_time(p.start_ms),
            start_ms: p.start_ms,
            duration_ms: end.saturating_sub(p.start_ms),
        }
    }
}

impl JobView {
    pub fn running(&self) -> bool {
        self.status == "running"
    }
}

#[derive(Default)]
pub struct JobFilter {
    /// Only jobs from this project.
    pub project: Option<PathBuf>,
    pub running: bool,
    pub limit: Option<usize>,
}

/// Jobs, newest first.
pub fn jobs(home: &Home, filter: &JobFilter) -> clapsmith::Result<Vec<JobView>> {
    let mut jobs = crate::client::jobs(home)?;
    jobs.reverse();
    Ok(jobs
        .iter()
        .filter(|p| !filter.running || p.running)
        .filter(|p| match &filter.project {
            Some(root) => p.project.as_deref() == Some(root.as_path()),
            None => true,
        })
        .take(filter.limit.unwrap_or(usize::MAX))
        .map(JobView::from)
        .collect())
}

pub fn job(home: &Home, id: u64) -> clapsmith::Result<JobView> {
    crate::client::jobs(home)?
        .iter()
        .find(|p| p.id == id)
        .map(JobView::from)
        .ok_or_else(|| format!("no process with ID {id}").into())
}

/// What to start, for [`start`].
pub struct Start<'a> {
    pub target: &'a str,
    pub directive: &'a str,
    /// For make: `VAR=value`, more goals, flags.
    pub args: &'a [String],
    pub pty: bool,
    pub origin: Origin,
    pub client: Option<String>,
    /// Return the same job if it's already running, instead of starting
    /// another.
    pub reuse: bool,
}

#[derive(Serialize, Debug)]
pub struct Started {
    pub id: u64,
    /// It was already running; see [`Start::reuse`].
    pub reused: bool,
}

/// Starts `make <directive>` in a target, in the daemon, without following
/// it.
pub fn start(home: &Home, config: &Config, s: Start) -> clapsmith::Result<Started> {
    let argv = crate::targets::make_argv(config, s.target, s.directive, s.args)?;
    let job = crate::targets::job(
        argv,
        config,
        s.target,
        s.directive,
        s.pty,
        s.origin,
        s.client,
    )?;
    let (id, reused) = crate::client::start(home, job, s.reuse)?;
    Ok(Started { id, reused })
}

/// Waits up to `timeout` (forever if `None`) for a job to end, and
/// returns it, still running if the wait timed out.
pub fn wait(home: &Home, id: u64, timeout: Option<Duration>) -> clapsmith::Result<JobView> {
    crate::client::wait(home, id, timeout).map(|p| JobView::from(&p))
}

pub fn kill(home: &Home, id: u64) -> clapsmith::Result<()> {
    crate::client::kill(home, id)
}

/// Which lines of a log to return, for [`logs`].
#[derive(Default)]
pub struct LogQuery {
    /// The first line number wanted, from 1: pass the previous page's
    /// `next` to get only what's new.
    pub from: Option<usize>,
    /// Only the last this many lines (after `grep`).
    pub tail: Option<usize>,
    /// Only lines matching.
    pub grep: Option<regex::Regex>,
    /// At most this many lines. With `from`, the first ones (and `next`
    /// continues after them); otherwise the last ones.
    pub max: Option<usize>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LogLine {
    pub n: usize,
    pub text: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct LogPage {
    pub id: u64,
    pub running: bool,
    /// Complete lines in the log. A running job's last, unfinished line
    /// isn't counted; it's in `live`.
    pub total: usize,
    /// The oldest line still kept; very long logs drop their start.
    pub first: usize,
    pub lines: Vec<LogLine>,
    /// Lines that matched but weren't returned because of `tail` or `max`.
    pub omitted: usize,
    /// Where to continue: pass as `from` for what comes after this page.
    pub next: usize,
    /// The line being written now, such as a progress bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<String>,
}

/// A page of a job's log, as plain text.
pub fn logs(home: &Home, id: u64, query: &LogQuery) -> clapsmith::Result<LogPage> {
    let job = job(home, id)?;
    let raw = crate::logs::read(&home.log(id));
    Ok(page(id, job.running(), &raw, query))
}

fn page(id: u64, running: bool, raw: &str, query: &LogQuery) -> LogPage {
    let mut view = crate::tui::LogView::default();
    view.push(raw);
    let (dropped, mut lines) = view.plain();
    // The cursor's line: empty after a final newline, else unfinished.
    let last = lines.pop().unwrap_or_default();
    let live = (running && !last.is_empty()).then(|| last.clone());
    if !running && !last.is_empty() {
        lines.push(last);
    }
    let first = dropped + 1;
    let total = dropped + lines.len();
    let from = query.from.unwrap_or(first).max(first);
    let mut picked: Vec<LogLine> = lines
        .into_iter()
        .enumerate()
        .map(|(i, text)| LogLine { n: first + i, text })
        .filter(|l| l.n >= from)
        .filter(|l| query.grep.as_ref().is_none_or(|re| re.is_match(&l.text)))
        .collect();
    let mut omitted = 0;
    let mut next = total + 1;
    if let Some(tail) = query.tail
        && picked.len() > tail
    {
        omitted += picked.len() - tail;
        picked.drain(..picked.len() - tail);
    }
    if let Some(max) = query.max
        && picked.len() > max
    {
        omitted += picked.len() - max;
        if query.from.is_some() {
            picked.truncate(max);
            next = picked.last().map_or(from, |l| l.n + 1);
        } else {
            picked.drain(..picked.len() - max);
        }
    }
    LogPage {
        id,
        running,
        total,
        first,
        lines: picked,
        omitted,
        next,
        live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(p: &LogPage) -> Vec<(usize, &str)> {
        p.lines.iter().map(|l| (l.n, l.text.as_str())).collect()
    }

    #[test]
    fn progress_bars_collapse_and_the_live_line_is_separate() {
        let raw = "\x1b[32mone\x1b[0m\n10%\r50%\r\x1b[Kdone\nbuilding 3/9";
        let p = page(1, true, raw, &LogQuery::default());
        assert_eq!(texts(&p), [(1, "one"), (2, "done")]);
        assert_eq!(p.live.as_deref(), Some("building 3/9"));
        assert_eq!((p.total, p.next), (2, 3));
        let p = page(1, false, raw, &LogQuery::default());
        assert_eq!(p.total, 3, "a finished job's last line counts");
        assert_eq!(p.live, None);
    }

    #[test]
    fn from_returns_only_whats_new() {
        let raw = "a\nb\nc\n";
        let p = page(
            1,
            true,
            raw,
            &LogQuery {
                from: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(texts(&p), [(3, "c")]);
        assert_eq!(p.next, 4);
        let p = page(
            1,
            true,
            raw,
            &LogQuery {
                from: Some(4),
                ..Default::default()
            },
        );
        assert!(p.lines.is_empty());
        assert_eq!(p.next, 4);
    }

    #[test]
    fn max_pages_forward_from_a_cursor_and_backward_without_one() {
        let raw: String = (1..=10).map(|i| format!("l{i}\n")).collect();
        let q = LogQuery {
            from: Some(1),
            max: Some(3),
            ..Default::default()
        };
        let p = page(1, false, &raw, &q);
        assert_eq!(texts(&p), [(1, "l1"), (2, "l2"), (3, "l3")]);
        assert_eq!((p.next, p.omitted), (4, 7));
        let p = page(
            1,
            false,
            &raw,
            &LogQuery {
                max: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(texts(&p), [(9, "l9"), (10, "l10")]);
        assert_eq!((p.next, p.omitted), (11, 8));
    }

    #[test]
    fn grep_and_tail_keep_line_numbers() {
        let raw = "ok\nerror: one\nok\nerror: two\nerror: three\n";
        let q = LogQuery {
            grep: Some(regex::Regex::new("^error").unwrap()),
            tail: Some(2),
            ..Default::default()
        };
        let p = page(1, false, raw, &q);
        assert_eq!(texts(&p), [(4, "error: two"), (5, "error: three")]);
        assert_eq!(p.omitted, 1);
    }
}
