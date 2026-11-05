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

/// Whether jobs run in a pseudo-terminal unless asked otherwise: yes,
/// unless `GODO_PTY` is `0`, `never`, or `false` (for servers, where
/// nothing reads the output in a terminal).
pub fn default_pty() -> bool {
    !matches!(
        std::env::var("GODO_PTY").as_deref(),
        Ok("0" | "never" | "false")
    )
}

/// The size a detached job's terminal gets when there's no terminal here
/// to copy: wide, so tools wrap less in logs agents read.
const DETACHED_SIZE: (u16, u16) = (120, 40);

/// What a job runs and where, for [`run`] and [`start`].
pub struct Job {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub target: String,
    pub directive: String,
    pub pty: bool,
    pub origin: proto::Origin,
    pub client: Option<String>,
    pub project: Option<PathBuf>,
}

impl Job {
    fn request(self, (cols, rows): (u16, u16), reuse: bool) -> Request {
        Request::Run {
            argv: self.argv,
            cwd: self.cwd,
            env: self.env,
            target: self.target,
            directive: self.directive,
            cols,
            rows,
            pty: self.pty,
            origin: self.origin,
            client: self.client,
            project: self.project,
            reuse,
        }
    }
}

/// Runs a job in the daemon and follows its output. Ctrl+C detaches and
/// leaves it running; Ctrl+C twice quickly kills it. Returns the job's
/// exit code, or success when detached.
pub fn run(home: &Home, job: Job) -> clapsmith::Result {
    let size = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    follow(home, job.request(size, false), true)
}

/// Starts a job in the daemon without following it. With `reuse`, an
/// identical job that's already running is returned instead. Returns the
/// job's id and whether it was reused.
pub fn start(home: &Home, job: Job, reuse: bool) -> clapsmith::Result<(u64, bool)> {
    let size = ratatui::crossterm::terminal::size().unwrap_or(DETACHED_SIZE);
    let Some((_, first)) = open(home, &job.request(size, reuse), true)? else {
        unreachable!("open starts the daemon");
    };
    match first {
        Event::Started { id, reused } => Ok((id, reused)),
        Event::Error { message } => Err(message.into()),
        other => Err(format!("unexpected reply from the daemon: {other:?}").into()),
    }
}

/// Waits up to `timeout` (forever if `None`) for a job to end. Returns the
/// job, which is still running if the wait timed out.
pub fn wait(home: &Home, id: u64, timeout: Option<Duration>) -> clapsmith::Result<ProcessInfo> {
    let saved = || {
        saved_history(home)
            .into_iter()
            .find(|p| p.id == id)
            .ok_or_else(|| clapsmith::Error::from(format!("no process with ID {id}")))
    };
    let Some((mut stream, first)) = open(home, &Request::Wait { id }, false)? else {
        // Without the daemon nothing runs; its history says how jobs ended.
        return saved();
    };
    match first {
        Event::Done { process } => return Ok(process),
        Event::Ok => {}
        Event::Error { message } => return Err(message.into()),
        other => return Err(format!("unexpected reply from the daemon: {other:?}").into()),
    }
    stream.set_read_timeout(timeout)?;
    match proto::read::<Event>(&mut stream) {
        Ok(Some(Event::Done { process })) => Ok(process),
        Ok(Some(other)) => Err(format!("unexpected reply from the daemon: {other:?}").into()),
        // The daemon stopped, and the job with it.
        Ok(None) => saved(),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            jobs(home)?
                .into_iter()
                .find(|p| p.id == id)
                .ok_or_else(|| format!("no process with ID {id}").into())
        }
        Err(e) => Err(e.into()),
    }
}

/// Stops a running job and everything it started.
pub fn kill(home: &Home, id: u64) -> clapsmith::Result<()> {
    match ask(home, Request::Kill { id }, false)? {
        None => Err(format!("no running process with ID {id} (the daemon isn't running)").into()),
        Some(_) => Ok(()),
    }
}

/// A process ID as typed: `3` or `"3"`.
pub fn parse_id(id: &str) -> clapsmith::Result<u64> {
    id.trim_matches('"')
        .parse()
        .map_err(|_| format!("`{id}` isn't a process ID; see `godo ps`").into())
}

/// `godo attach -i <id>`: replays a job's output and follows it.
pub fn attach(home: &Home, id: &str) -> clapsmith::Result {
    let id = parse_id(id)?;
    if connect(home).is_none() {
        // Nothing runs without the daemon, so replay the saved log.
        let Some(job) = saved_history(home).into_iter().find(|p| p.id == id) else {
            return Err(format!("no process with ID {id}").into());
        };
        println!("Attached to process ID: {id}");
        print!("{}", crate::logs::read(&home.log(id)));
        return Ok(match job.exit_code {
            Some(code) => {
                println!("\nProcess exited with code: {code}");
                ExitCode::from(u8::try_from(code).unwrap_or(1))
            }
            None => {
                println!("\nProcess stopped when the daemon did.");
                ExitCode::FAILURE
            }
        });
    }
    follow(home, Request::Attach { id }, false)
}

enum Msg {
    Event(Event),
    Closed,
    Interrupt,
}

/// Raw mode for following a job in a terminal: keystrokes go to the job
/// as they're typed, and Ctrl+C arrives as a byte godo handles itself.
/// Restored when dropped.
struct RawMode;

impl RawMode {
    fn enter() -> Option<Self> {
        use std::io::IsTerminal;
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return None;
        }
        ratatui::crossterm::terminal::enable_raw_mode().ok()?;
        Some(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
    }
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
    let writer = std::sync::Arc::new(std::sync::Mutex::new(stream));
    let raw = RawMode::enter();
    if raw.is_some() {
        forward_keys(std::sync::Arc::clone(&writer), tx.clone());
    }
    let mut signals = signal_hook::iterator::Signals::new([libc::SIGINT, libc::SIGWINCH])?;
    let handle = signals.handle();
    let resizes = std::sync::Arc::clone(&writer);
    std::thread::spawn(move || {
        for signal in signals.forever() {
            if signal == libc::SIGWINCH {
                send_size(&resizes);
            } else if tx.send(Msg::Interrupt).is_err() {
                return;
            }
        }
    });
    let out = Out { raw: raw.is_some() };
    let result = follow_events(home, &rx, &out);
    handle.close();
    drop(raw);
    result
}

/// Sends the job this terminal's size, so it draws to fit.
fn send_size(writer: &std::sync::Mutex<UnixStream>) {
    if let Ok((cols, rows)) = ratatui::crossterm::terminal::size() {
        let mut stream = writer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = proto::write(&mut *stream, &proto::ClientMsg::Resize { cols, rows });
    }
}

/// Copies keystrokes to the job, except Ctrl+C, which is godo's: once
/// detaches, twice kills.
fn forward_keys(writer: std::sync::Arc<std::sync::Mutex<UnixStream>>, tx: mpsc::Sender<Msg>) {
    send_size(&writer);
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = io::stdin();
        let mut buf = [0u8; 1024];
        loop {
            let n = match stdin.read(&mut buf) {
                Ok(0) | Err(_) => return,
                Ok(n) => n,
            };
            for part in split_interrupts(&buf[..n]) {
                let sent = match part {
                    Some(data) => {
                        let mut stream = writer.lock().unwrap_or_else(|e| e.into_inner());
                        proto::write(&mut *stream, &proto::ClientMsg::Input { data }).is_ok()
                    }
                    None => tx.send(Msg::Interrupt).is_ok(),
                };
                if !sent {
                    return;
                }
            }
        }
    });
}

/// Splits typed bytes at each Ctrl+C (`None`), keeping the rest in order.
fn split_interrupts(bytes: &[u8]) -> Vec<Option<Vec<u8>>> {
    let mut out = Vec::new();
    for (i, chunk) in bytes.split(|&b| b == 0x03).enumerate() {
        if i > 0 {
            out.push(None);
        }
        if !chunk.is_empty() {
            out.push(Some(chunk.to_vec()));
        }
    }
    out
}

/// godo's own messages while following, with `\r\n` line ends in raw mode.
struct Out {
    raw: bool,
}

impl Out {
    fn say(&self, line: &str) {
        let text = if self.raw {
            format!("{}\r\n", line.replace('\n', "\r\n"))
        } else {
            format!("{line}\n")
        };
        let mut out = io::stdout();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

fn follow_events(home: &Home, rx: &mpsc::Receiver<Msg>, out: &Out) -> clapsmith::Result {
    let mut id = None;
    let mut first_interrupt: Option<Instant> = None;
    let mut stdout = io::stdout();
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
            Msg::Event(Event::Started { id: started, .. }) => {
                id = Some(started);
                out.say(&format!(
                    "Process started with ID: {started} (Ctrl+C to detach, Ctrl+C Ctrl+C to kill)"
                ));
            }
            Msg::Event(Event::Attached { id: attached }) => {
                id = Some(attached);
                out.say(&format!("Attached to process ID: {attached}"));
            }
            Msg::Event(Event::Log { stream, data }) => {
                let _ = stdout.write_all(crate::logs::styled(&stream, &data).as_bytes());
                let _ = stdout.flush();
            }
            Msg::Event(Event::Exit { code }) => {
                out.say(&format!("\nProcess exited with code: {code}"));
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
                    out.say("Killing process...");
                    return Ok(ExitCode::from(130));
                }
                _ => {
                    first_interrupt = Some(Instant::now());
                    out.say("\nDetaching (press Ctrl+C again quickly to kill)...");
                }
            },
        }
    }
}

/// Every job: from the daemon if it runs, else from its saved history,
/// where nothing can still be running. The daemon itself isn't included.
pub fn jobs(home: &Home) -> clapsmith::Result<Vec<ProcessInfo>> {
    let jobs = if connect(home).is_some() {
        processes(home)?
    } else {
        saved_history(home)
    };
    Ok(jobs.into_iter().filter(|p| p.id != 0).collect())
}

/// `history.json` as the daemon left it. Jobs it recorded as running
/// ended when it stopped, so they're shown as stopped.
pub fn saved_history(home: &Home) -> Vec<ProcessInfo> {
    let mut jobs: Vec<ProcessInfo> = std::fs::read_to_string(home.history())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    for job in &mut jobs {
        job.running = false;
    }
    jobs
}

/// How `history` describes a job.
pub fn status(p: &ProcessInfo) -> &'static str {
    match (p.running, p.exit_code) {
        (true, _) => "Running",
        (false, Some(0)) => "Success",
        (false, Some(_)) => "Failed",
        // Ended without an exit code: the daemon stopped while it ran.
        (false, None) => "Stopped",
    }
}

/// `godo ps`: running jobs as a table.
pub fn ps(home: &Home) -> clapsmith::Result {
    if connect(home).is_none() {
        println!("No running processes (the daemon isn't running).");
        return Ok(ExitCode::SUCCESS);
    }
    let rows: Vec<Vec<String>> = jobs(home)?
        .iter()
        .filter(|p| p.running)
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

/// `godo history` outside a terminal: every job, newest first, as a table.
pub fn history(home: &Home) -> clapsmith::Result {
    let mut jobs = jobs(home)?;
    jobs.reverse();
    let rows: Vec<Vec<String>> = jobs
        .iter()
        .map(|p| {
            vec![
                p.id.to_string(),
                p.target.clone(),
                p.directive.clone(),
                status(p).to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_c_is_split_out_of_typed_input() {
        assert_eq!(split_interrupts(b"ab"), [Some(b"ab".to_vec())]);
        assert_eq!(split_interrupts(b"\x03"), [None]);
        assert_eq!(
            split_interrupts(b"a\x03\x03b"),
            [Some(b"a".to_vec()), None, None, Some(b"b".to_vec())]
        );
    }
}
