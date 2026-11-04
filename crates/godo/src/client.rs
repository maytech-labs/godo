// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Talking to the daemon: starting it when needed, running and attaching
//! to jobs, and `ps`, `stop`, `restart`, and `history`.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::proto::{self, Envelope, Event, Home, ProcessInfo, Request};

/// A second Ctrl+C within this long kills the job instead of detaching.
const DOUBLE_PRESS: Duration = Duration::from_millis(500);
/// How long to wait for a second Ctrl+C before detaching.
const DETACH_WAIT: Duration = Duration::from_millis(600);

/// Whether runs go through the daemon. `GODO_DAEMON=1` or `0` decides; by
/// default only in a terminal, so CI, pipes, and scripts get plain
/// foreground runs with make's output and exit code.
pub fn use_daemon() -> bool {
    match std::env::var("GODO_DAEMON").as_deref() {
        Ok("1" | "always" | "true") => true,
        Ok("0" | "never" | "false") => false,
        _ => {
            use std::io::IsTerminal;
            io::stdin().is_terminal() && io::stdout().is_terminal()
        }
    }
}

/// A connection to a running daemon, or `None` if none answers.
pub fn connect(home: &Home) -> Option<UnixStream> {
    UnixStream::connect(home.socket()).ok()
}

/// A connection to the daemon, starting it first if needed.
pub fn connect_or_start(home: &Home) -> clapsmith::Result<UnixStream> {
    if let Some(stream) = connect(home) {
        return Ok(stream);
    }
    home.check_socket_path()?;
    start_daemon(home)?;
    for _ in 0..50 {
        if let Some(stream) = connect(home) {
            return Ok(stream);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!(
        "the godo daemon didn't start; see {}",
        home.log(0).display()
    )
    .into())
}

/// Starts `godo daemon` in the background, in its own process group so a
/// Ctrl+C in this terminal never reaches it.
fn start_daemon(home: &Home) -> clapsmith::Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg("daemon")
        .env("GODO_HOME", &home.dir)
        .current_dir(&home.dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("can't start the godo daemon: {e}"))?;
    Ok(())
}

/// Sends a request and reads the first reply. A daemon of another version
/// with no running jobs is replaced first; one with running jobs is left
/// alone and reported. `None` if no daemon runs and `start` is false.
fn open(
    home: &Home,
    request: &Request,
    start: bool,
) -> clapsmith::Result<Option<(UnixStream, Event)>> {
    let mut replaced = false;
    loop {
        let mut stream = match (connect(home), start) {
            (Some(stream), _) => stream,
            (None, true) => connect_or_start(home)?,
            (None, false) => return Ok(None),
        };
        proto::write(&mut stream, &Envelope::new(request.clone()))?;
        let first =
            proto::read::<Event>(&mut stream)?.ok_or("the godo daemon closed the connection")?;
        match first {
            Event::VersionMismatch {
                daemon_version,
                running,
            } if running == 0 && !replaced => {
                replace_daemon(home, &daemon_version)?;
                replaced = true;
            }
            Event::VersionMismatch {
                daemon_version,
                running,
            } => {
                return Err(format!(
                    "the godo daemon is version {daemon_version} and has {running} running job{}, but this godo is {}; let {} finish, or run `godo restart --force` to stop {}",
                    if running == 1 { "" } else { "s" },
                    proto::wire_version(),
                    if running == 1 { "it" } else { "them" },
                    if running == 1 { "it" } else { "them" },
                )
                .into());
            }
            first => return Ok(Some((stream, first))),
        }
    }
}

/// Stops an idle daemon of another version and starts this one.
fn replace_daemon(home: &Home, old: &str) -> clapsmith::Result<()> {
    if let Some(mut stream) = connect(home) {
        proto::write(
            &mut stream,
            &Envelope::new(Request::Shutdown { force: false }),
        )?;
        let _ = proto::read::<Event>(&mut stream);
    }
    wait_until_stopped(home);
    eprintln!(
        "godo: restarted the daemon ({old} → {})",
        proto::wire_version()
    );
    connect_or_start(home).map(drop)
}

fn wait_until_stopped(home: &Home) {
    for _ in 0..100 {
        if connect(home).is_none() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends one request and reads one reply.
fn ask(home: &Home, request: Request, start: bool) -> clapsmith::Result<Option<Event>> {
    match open(home, &request, start)? {
        None => Ok(None),
        Some((_, Event::Error { message })) => Err(message.into()),
        Some((_, event)) => Ok(Some(event)),
    }
}

/// Every job the daemon knows, with the daemon first. Empty if no daemon
/// is running.
pub fn processes(home: &Home) -> clapsmith::Result<Vec<ProcessInfo>> {
    match ask(home, Request::Ps, false)? {
        None => Ok(Vec::new()),
        Some(Event::Processes { processes }) => Ok(processes),
        Some(other) => Err(format!("unexpected reply from the daemon: {other:?}").into()),
    }
}

/// What a job runs and where, for [`run`].
pub struct Job {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub target: String,
    pub directive: String,
}

/// Runs a job in the daemon and follows its output. Ctrl+C detaches and
/// leaves it running; Ctrl+C twice quickly kills it. Returns the job's
/// exit code, or success when detached.
pub fn run(home: &Home, job: Job) -> clapsmith::Result {
    let (cols, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let request = Request::Run {
        argv: job.argv,
        cwd: job.cwd,
        env: job.env,
        target: job.target,
        directive: job.directive,
        cols,
        rows,
    };
    follow(home, request, true)
}

/// `godo attach -i <id>`: replays a job's output and follows it.
pub fn attach(home: &Home, id: &str) -> clapsmith::Result {
    let id: u64 = id
        .trim_matches('"')
        .parse()
        .map_err(|_| format!("`{id}` isn't a process ID; see `godo ps`"))?;
    if connect(home).is_none() {
        println!("Daemon not running.");
        return Ok(ExitCode::SUCCESS);
    }
    follow(home, Request::Attach { id }, false)
}

enum Msg {
    Event(Event),
    Closed,
    Interrupt,
}

fn follow(home: &Home, request: Request, start: bool) -> clapsmith::Result {
    let Some((stream, first)) = open(home, &request, start)? else {
        println!("Daemon not running.");
        return Ok(ExitCode::SUCCESS);
    };
    let (tx, rx) = mpsc::channel();
    let _ = tx.send(Msg::Event(first));
    let mut reader = stream.try_clone()?;
    let events = tx.clone();
    std::thread::spawn(move || {
        while let Ok(Some(event)) = proto::read::<Event>(&mut reader) {
            if events.send(Msg::Event(event)).is_err() {
                return;
            }
        }
        let _ = events.send(Msg::Closed);
    });
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT])?;
    let handle = signals.handle();
    std::thread::spawn(move || {
        for _ in signals.forever() {
            if tx.send(Msg::Interrupt).is_err() {
                return;
            }
        }
    });
    let result = follow_events(home, &rx);
    handle.close();
    result
}

fn follow_events(home: &Home, rx: &mpsc::Receiver<Msg>) -> clapsmith::Result {
    let mut id = None;
    let mut first_interrupt: Option<Instant> = None;
    let mut out = io::stdout();
    loop {
        let msg = match first_interrupt {
            Some(at) => match rx.recv_timeout(DETACH_WAIT.saturating_sub(at.elapsed())) {
                Ok(msg) => msg,
                // No second Ctrl+C: detach, leaving the job running.
                Err(_) => return Ok(ExitCode::SUCCESS),
            },
            None => rx.recv().map_err(|_| "lost the connection to the daemon")?,
        };
        match msg {
            Msg::Event(Event::Started { id: started }) => {
                id = Some(started);
                say(&format!(
                    "Process started with ID: {started} (Ctrl+C to detach, Ctrl+C Ctrl+C to kill)"
                ));
            }
            Msg::Event(Event::Attached { id: attached }) => {
                id = Some(attached);
                say(&format!("Attached to process ID: {attached}"));
            }
            Msg::Event(Event::Log { stream, data }) => {
                let _ = out.write_all(crate::logs::styled(&stream, &data).as_bytes());
                let _ = out.flush();
            }
            Msg::Event(Event::Exit { code }) => {
                say(&format!("\nProcess exited with code: {code}"));
                return Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)));
            }
            Msg::Event(Event::Error { message }) => return Err(message.into()),
            Msg::Event(_) => {}
            Msg::Closed => return Err("lost the connection to the daemon".into()),
            Msg::Interrupt => match first_interrupt {
                Some(at) if at.elapsed() < DOUBLE_PRESS => {
                    if let Some(id) = id {
                        let _ = ask(home, Request::Kill { id }, false);
                    }
                    say("Killing process...");
                    return Ok(ExitCode::from(130));
                }
                _ => {
                    first_interrupt = Some(Instant::now());
                    say("\nDetaching (press Ctrl+C again quickly to kill)...");
                }
            },
        }
    }
}

/// Prints a line, ignoring a closed stdout (`godo build web | head`):
/// the job's fate mustn't depend on whether anyone still reads.
fn say(line: &str) {
    let mut out = io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// `godo ps`: running jobs as a table.
pub fn ps(home: &Home) -> clapsmith::Result {
    if connect(home).is_none() {
        println!("Daemon not running.");
        return Ok(ExitCode::SUCCESS);
    }
    let running: Vec<ProcessInfo> = processes(home)?
        .into_iter()
        .filter(|p| p.running && p.id != 0)
        .collect();
    let rows: Vec<Vec<String>> = running
        .iter()
        .map(|p| {
            vec![
                p.id.to_string(),
                p.pid.to_string(),
                p.target.clone(),
                p.directive.clone(),
                "Running".to_string(),
            ]
        })
        .collect();
    print!(
        "{}",
        table(&["ID", "PID", "TARGET", "DIRECTIVE", "STATUS"], &rows)
    );
    if rows.is_empty() {
        println!("No running processes.");
    }
    Ok(ExitCode::SUCCESS)
}

/// `godo history`: every job, newest first.
pub fn history(home: &Home) -> clapsmith::Result {
    if connect(home).is_none() {
        println!("Daemon not running.");
        return Ok(ExitCode::SUCCESS);
    }
    let mut jobs: Vec<ProcessInfo> = processes(home)?.into_iter().filter(|p| p.id != 0).collect();
    jobs.reverse();
    let rows: Vec<Vec<String>> = jobs
        .iter()
        .map(|p| {
            let status = match (p.running, p.exit_code) {
                (true, _) => "Running",
                (false, Some(0)) => "Success",
                _ => "Failed",
            };
            vec![
                p.id.to_string(),
                p.target.clone(),
                p.directive.clone(),
                status.to_string(),
                proto::local_time(p.start_ms),
            ]
        })
        .collect();
    print!(
        "{}",
        table(
            &["ID", "TARGET", "DIRECTIVE", "STATUS", "START TIME"],
            &rows
        )
    );
    Ok(ExitCode::SUCCESS)
}

pub fn history_clear(home: &Home) -> clapsmith::Result {
    match ask(home, Request::HistoryClear, false)? {
        None => println!("Daemon not running."),
        Some(_) => println!("History cleared."),
    }
    Ok(ExitCode::SUCCESS)
}

/// `godo stop [--force]`: refuses while jobs run, unless forced.
pub fn stop(home: &Home, force: bool) -> clapsmith::Result {
    match ask(home, Request::Shutdown { force }, false)? {
        None => println!("Daemon not running."),
        Some(Event::Busy { running }) => {
            let jobs: Vec<String> = running
                .iter()
                .map(|p| format!("{} ({} {})", p.id, p.directive, p.target))
                .collect();
            return Err(format!(
                "the daemon has {} running job{}: {}. Stopping it would stop them too; use --force for that, or `godo attach -i <id>` to see them",
                running.len(),
                if running.len() == 1 { "" } else { "s" },
                jobs.join(", ")
            )
            .into());
        }
        Some(_) => {
            // Wait for the socket to go, so a following start doesn't race it.
            wait_until_stopped(home);
            println!("Daemon shutdown requested.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

pub fn restart(home: &Home, force: bool) -> clapsmith::Result {
    stop(home, force)?;
    connect_or_start(home)?;
    println!("Daemon started.");
    Ok(ExitCode::SUCCESS)
}

/// A table drawn like the C++ godo's: grey `┌┬┐` borders, cyan bold
/// headers, one space of padding in each cell.
pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    use unicode_width::UnicodeWidthStr;
    let border = "\x1b[38;5;240m";
    let header = "\x1b[1;38;5;6m";
    let reset = "\x1b[0m";
    let mut widths: Vec<usize> = headers.iter().map(|h| h.width()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.width());
        }
    }
    let rule = |l: &str, m: &str, r: &str| {
        let parts: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
        format!("{border}{l}{}{r}{reset}\n", parts.join(m))
    };
    let line = |cells: Vec<&str>, style: &str| {
        let mut out = format!("{border}│{reset}");
        for (cell, w) in cells.iter().zip(&widths) {
            let pad = w - cell.width();
            out.push_str(&format!(
                " {style}{cell}{}{} {border}│{reset}",
                if style.is_empty() { "" } else { reset },
                " ".repeat(pad)
            ));
        }
        out.push('\n');
        out
    };
    let mut out = rule("┌", "┬", "┐");
    out.push_str(&line(headers.to_vec(), header));
    out.push_str(&rule("├", "┼", "┤"));
    for row in rows {
        out.push_str(&line(row.iter().map(String::as_str).collect(), ""));
    }
    out.push_str(&rule("└", "┴", "┘"));
    if colour_stdout() {
        out
    } else {
        anstream::adapter::strip_str(&out).to_string()
    }
}

fn colour_stdout() -> bool {
    use std::io::IsTerminal;
    std::env::var_os("NO_COLOR").is_none()
        && (std::env::var_os("CLICOLOR_FORCE").is_some() || io::stdout().is_terminal())
}
