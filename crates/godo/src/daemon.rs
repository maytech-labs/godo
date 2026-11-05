// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! The godo daemon: runs jobs in pseudo-terminals, writes their output to
//! `logs/<id>.log`, and streams it to clients that run or attach to them.
//!
//! Unlike the C++ daemon, every job runs with the working directory,
//! environment, and terminal size of the client that started it (not the
//! daemon's), and killing a job signals its whole process group.
//!
//! A job's output is written to its log and sent to its clients under one
//! lock, and attaching reads the log and joins the clients under the same
//! lock, so an attached client sees everything exactly once. Output is
//! kept as the job wrote it, `\r` and all, for terminals to draw.
//!
//! Clients following a job can type into it and resize it: they send
//! [`ClientMsg`]s on the same connection. With several attached, all can
//! type, and the latest resize wins.
//!
//! A job can instead run on plain pipes (`pty: false`), for servers and
//! logs that aren't read in a terminal: stdout and stderr are logged as
//! `out` and `err`, stdin is empty, and typing and resizing do nothing.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::logs::LogWriter;
use crate::proto::{self, ClientMsg, Event, Home, PROTOCOL, ProcessInfo, RawEnvelope, Request};

/// How long a client may stall a write before it's dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Time between SIGTERM and SIGKILL when killing a job.
const KILL_GRACE: Duration = Duration::from_secs(3);
/// Finished jobs kept, with their logs, unless `GODO_HISTORY_LIMIT` says.
const HISTORY_LIMIT: usize = 500;
/// The daemon's own log is cut back to [`DAEMON_LOG_KEEP`] past this.
const DAEMON_LOG_LIMIT: u64 = 1 << 20;
const DAEMON_LOG_KEEP: usize = 256 << 10;

/// The id the daemon itself has in `ps` and `watch`.
const DAEMON_ID: u64 = 0;

struct Job {
    info: ProcessInfo,
    /// Clients that ran or attached to this job.
    clients: Vec<UnixStream>,
    /// The job's log, open while it runs.
    log: Option<LogWriter>,
    /// Kept for the job's lifetime; closing it hangs up the terminal.
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    /// Where clients' keystrokes go.
    input: Option<Box<dyn Write + Send>>,
    /// Clients waiting for it to end ([`Request::Wait`]).
    waiters: Vec<UnixStream>,
}

struct State {
    next_id: u64,
    /// Jobs still running and the newest finished ones, by id.
    jobs: BTreeMap<u64, Job>,
    daemon: ProcessInfo,
}

struct Daemon {
    home: Home,
    /// What this daemon answers to; clients of another version are told.
    version: String,
    history_limit: usize,
    state: Mutex<State>,
}

/// Runs the daemon until it's asked to shut down. Returns early, without
/// error, if another daemon already answers on the socket.
pub fn run(home: Home) -> io::Result<()> {
    let result = listen(home.clone());
    if let Err(e) = &result {
        // The daemon has no terminal; its log is where this gets seen.
        if let Ok(mut w) = LogWriter::open(&home.log(DAEMON_ID)) {
            let _ = w.write("err", &format!("daemon failed: {e}\n"));
        }
    }
    result
}

fn listen(home: Home) -> io::Result<()> {
    home.check_socket_path()?;
    let socket = home.socket();
    if !proto::remove_stale_socket(&socket)? {
        return Ok(());
    }
    let listener = UnixListener::bind(&socket)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    trim_log(&home.log(DAEMON_ID), DAEMON_LOG_LIMIT, DAEMON_LOG_KEEP);
    let history_limit = std::env::var("GODO_HISTORY_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(HISTORY_LIMIT);
    let version = proto::wire_version();
    let jobs = load_history(&home);
    let next_id = jobs.keys().next_back().map_or(1, |id| id + 1);
    let daemon = Arc::new(Daemon {
        state: Mutex::new(State {
            next_id,
            jobs,
            daemon: ProcessInfo {
                id: DAEMON_ID,
                pid: std::process::id(),
                command: "godo daemon".into(),
                target: "daemon".into(),
                directive: "system".into(),
                running: true,
                exit_code: None,
                start_ms: proto::now_ms(),
                end_ms: None,
                origin: proto::Origin::Terminal,
                client: None,
                project: None,
                pty: false,
            },
        }),
        home,
        version,
        history_limit,
    });
    {
        let mut state = daemon.lock();
        daemon.prune(&mut state);
        daemon.save_history(&state);
    }
    daemon.log(&format!(
        "daemon {} started; socket={}",
        daemon.version,
        socket.display()
    ));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let daemon = Arc::clone(&daemon);
        std::thread::spawn(move || daemon.serve(stream));
    }
    Ok(())
}

impl Daemon {
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panicking job thread must not take the daemon down with it.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Daemon events go to log 0, which `watch` shows as `system daemon`.
    fn log(&self, message: &str) {
        if let Ok(mut w) = LogWriter::open(&self.home.log(DAEMON_ID)) {
            let _ = w.write("out", &format!("{message}\n"));
        }
    }

    fn serve(self: Arc<Self>, mut stream: UnixStream) {
        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
        let raw = match proto::read::<RawEnvelope>(&mut stream) {
            Ok(Some(raw)) => raw,
            _ => return,
        };
        let same = raw.protocol == PROTOCOL && raw.version == self.version;
        let request = if same {
            match serde_json::from_value::<Request>(raw.request) {
                Ok(request) => request,
                Err(e) => {
                    let message = format!("the godo daemon can't read this request: {e}");
                    let _ = proto::write(&mut stream, &Event::Error { message });
                    return;
                }
            }
        } else if raw.request.get("type").and_then(|t| t.as_str()) == Some("shutdown") {
            // Every version honours shutdown, so a newer godo can always
            // replace this daemon.
            let force = raw.request.get("force").and_then(|f| f.as_bool()) == Some(true);
            Request::Shutdown { force }
        } else {
            let running = self.running().len();
            let reply = Event::VersionMismatch {
                daemon_version: self.version.clone(),
                running,
            };
            let _ = proto::write(&mut stream, &reply);
            return;
        };
        let reply = match request {
            Request::Run {
                argv,
                cwd,
                env,
                target,
                directive,
                cols,
                rows,
                pty,
                origin,
                client,
                project,
                reuse,
            } => {
                let spec = Spec {
                    argv,
                    cwd,
                    env,
                    target,
                    directive,
                    cols,
                    rows,
                    pty,
                    origin,
                    client,
                    project,
                    reuse,
                };
                if let Err((mut stream, message)) = self.start(spec, stream) {
                    let _ = proto::write(&mut stream, &Event::Error { message });
                }
                return;
            }
            Request::Attach { id } => return self.attach(id, stream),
            Request::Wait { id } => return self.wait(id, stream),
            Request::Ps | Request::History => {
                let state = self.lock();
                let mut processes = vec![state.daemon.clone()];
                processes.extend(state.jobs.values().map(|j| j.info.clone()));
                Event::Processes { processes }
            }
            Request::HistoryClear => {
                self.clear_history();
                Event::Ok
            }
            Request::Kill { id } => match self.kill(id) {
                true => Event::Ok,
                false => Event::Error {
                    message: format!("no running process with ID {id}"),
                },
            },
            Request::Shutdown { force } => {
                let running = self.running();
                if !running.is_empty() && !force {
                    Event::Busy { running }
                } else {
                    self.log(&format!(
                        "shutdown requested{}",
                        if running.is_empty() {
                            ""
                        } else {
                            "; stopping running jobs"
                        }
                    ));
                    let _ = proto::write(&mut stream, &Event::Ok);
                    self.shutdown();
                }
            }
        };
        let _ = proto::write(&mut stream, &reply);
    }

    fn running(&self) -> Vec<ProcessInfo> {
        self.lock()
            .jobs
            .values()
            .filter(|j| j.info.running)
            .map(|j| j.info.clone())
            .collect()
    }

    fn start(self: &Arc<Self>, spec: Spec, stream: UnixStream) -> Result<(), (UnixStream, String)> {
        if spec.argv.is_empty() {
            return Err((stream, "nothing to run".into()));
        }
        let mut stream = stream;
        if spec.reuse {
            let state = self.lock();
            let same = state.jobs.values().find(|j| {
                let i = &j.info;
                i.running
                    && i.command == spec.argv.join(" ")
                    && i.target == spec.target
                    && i.directive == spec.directive
                    && i.project == spec.project
            });
            if let Some(job) = same {
                let id = job.info.id;
                drop(state);
                let _ = proto::write(&mut stream, &Event::Started { id, reused: true });
                return Ok(());
            }
        }
        let mut state = self.lock();
        let id = state.next_id;
        let log = match LogWriter::open(&self.home.log(id)) {
            Ok(log) => log,
            Err(e) => return Err((stream, format!("can't open the job's log: {e}"))),
        };
        let spawned = match if spec.pty {
            spawn_pty(&spec)
        } else {
            spawn_pipes(&spec)
        } {
            Ok(spawned) => spawned,
            Err(message) => return Err((stream, message)),
        };
        state.next_id += 1;
        // If the client left already, the job still runs, as if detached.
        let mut clients = Vec::new();
        let started = Event::Started { id, reused: false };
        if proto::write(&mut stream, &started).is_ok() && !spec.reuse {
            if spec.pty {
                self.listen_to(id, &stream);
            }
            clients.push(stream);
        }
        let info = ProcessInfo {
            id,
            pid: spawned.pid,
            command: spec.argv.join(" "),
            target: spec.target,
            directive: spec.directive,
            running: true,
            exit_code: None,
            start_ms: proto::now_ms(),
            end_ms: None,
            origin: spec.origin,
            client: spec.client,
            project: spec.project,
            pty: spec.pty,
        };
        state.jobs.insert(
            id,
            Job {
                info,
                clients,
                log: Some(log),
                master: spawned.master,
                input: spawned.input,
                waiters: Vec::new(),
            },
        );
        self.save_history(&state);
        drop(state);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let readers = spawned.readers.len();
        for (name, reader) in spawned.readers {
            let daemon = Arc::clone(self);
            let done = done_tx.clone();
            std::thread::spawn(move || {
                daemon.pump(id, name, reader);
                let _ = done.send(());
            });
        }
        drop(done_tx);
        let daemon = Arc::clone(self);
        let wait = spawned.wait;
        std::thread::spawn(move || {
            let code = wait();
            // Let the output finish. A background process the job left
            // behind can keep the terminal open, so don't wait forever.
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            for _ in 0..readers {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if done_rx.recv_timeout(left).is_err() {
                    break;
                }
            }
            daemon.finish(id, code);
        });
        Ok(())
    }

    /// Copies a job's output to its log and its clients.
    fn pump(&self, id: u64, stream: &'static str, mut reader: Box<dyn Read + Send>) {
        let mut chunker = proto::Utf8Chunker::default();
        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let text = chunker.push(&buf[..n]);
            if text.is_empty() {
                continue;
            }
            let mut state = self.lock();
            let Some(job) = state.jobs.get_mut(&id) else {
                continue;
            };
            if let Some(log) = job.log.as_mut() {
                let _ = log.write(stream, &text);
            }
            let event = Event::Log {
                stream: stream.into(),
                data: text,
            };
            broadcast(&mut job.clients, &event);
        }
    }

    fn finish(&self, id: u64, code: i32) {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(&id) else {
            return;
        };
        job.info.running = false;
        job.info.exit_code = Some(code);
        job.info.end_ms = Some(proto::now_ms());
        job.master = None;
        job.input = None;
        job.log = None;
        broadcast(&mut job.clients, &Event::Exit { code });
        job.clients.clear();
        let done = Event::Done {
            process: job.info.clone(),
        };
        broadcast(&mut job.waiters, &done);
        job.waiters.clear();
        self.prune(&mut state);
        self.save_history(&state);
    }

    fn attach(self: &Arc<Self>, id: u64, mut stream: UnixStream) {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(&id) else {
            drop(state);
            let message = format!("no process with ID {id}");
            let _ = proto::write(&mut stream, &Event::Error { message });
            return;
        };
        // Under the lock, the log is everything sent so far: nothing is
        // missed between reading it and joining the clients.
        let backlog = crate::logs::read(&self.home.log(id));
        let ok = proto::write(&mut stream, &Event::Attached { id }).is_ok()
            && (backlog.is_empty()
                || proto::write(
                    &mut stream,
                    &Event::Log {
                        stream: "out".into(),
                        data: backlog,
                    },
                )
                .is_ok());
        if !ok {
            return;
        }
        if job.info.running {
            self.listen_to(id, &stream);
            job.clients.push(stream);
        } else {
            let code = job.info.exit_code.unwrap_or(-1);
            let _ = proto::write(&mut stream, &Event::Exit { code });
        }
    }

    fn wait(&self, id: u64, mut stream: UnixStream) {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(&id) else {
            drop(state);
            let message = format!("no process with ID {id}");
            let _ = proto::write(&mut stream, &Event::Error { message });
            return;
        };
        if !job.info.running {
            let process = job.info.clone();
            drop(state);
            let _ = proto::write(&mut stream, &Event::Done { process });
            return;
        }
        if proto::write(&mut stream, &Event::Ok).is_ok() {
            // Waiting can take as long as the job; only the reply is timed.
            let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
            job.waiters.push(stream);
        }
    }

    /// Reads a following client's keystrokes and resizes until it leaves.
    fn listen_to(self: &Arc<Self>, id: u64, stream: &UnixStream) {
        let Ok(mut stream) = stream.try_clone() else {
            return;
        };
        let daemon = Arc::clone(self);
        std::thread::spawn(move || {
            while let Ok(Some(msg)) = proto::read::<ClientMsg>(&mut stream) {
                let mut state = daemon.lock();
                let Some(job) = state.jobs.get_mut(&id).filter(|j| j.info.running) else {
                    return;
                };
                match msg {
                    ClientMsg::Input { data } => {
                        if let Some(input) = job.input.as_mut() {
                            let _ = input.write_all(&data);
                            let _ = input.flush();
                        }
                    }
                    ClientMsg::Resize { cols, rows } => {
                        if let Some(master) = job.master.as_ref() {
                            let _ = master.resize(PtySize {
                                rows: rows.max(1),
                                cols: cols.max(1),
                                pixel_width: 0,
                                pixel_height: 0,
                            });
                        }
                    }
                }
            }
        });
    }

    /// SIGTERM to the job's process group, then SIGKILL if it's still
    /// there after a grace period.
    fn kill(self: &Arc<Self>, id: u64) -> bool {
        let pid = {
            let state = self.lock();
            match state.jobs.get(&id) {
                Some(job) if job.info.running && job.info.pid > 0 => job.info.pid,
                _ => return false,
            }
        };
        signal_group(pid, libc::SIGTERM);
        let daemon = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(KILL_GRACE);
            let still = daemon.lock().jobs.get(&id).is_some_and(|j| j.info.running);
            if still {
                signal_group(pid, libc::SIGKILL);
            }
        });
        true
    }

    /// Drops the oldest finished jobs, and their logs, past the limit.
    fn prune(&self, state: &mut State) {
        let finished: Vec<u64> = state
            .jobs
            .iter()
            .filter(|(_, j)| !j.info.running)
            .map(|(id, _)| *id)
            .collect();
        let excess = finished.len().saturating_sub(self.history_limit);
        for id in &finished[..excess] {
            state.jobs.remove(id);
            let _ = std::fs::remove_file(self.home.log(*id));
        }
    }

    fn clear_history(&self) {
        let mut state = self.lock();
        let finished: Vec<u64> = state
            .jobs
            .iter()
            .filter(|(_, j)| !j.info.running)
            .map(|(id, _)| *id)
            .collect();
        for id in finished {
            state.jobs.remove(&id);
            let _ = std::fs::remove_file(self.home.log(id));
        }
        self.save_history(&state);
        drop(state);
        let _ = std::fs::remove_file(self.home.log(DAEMON_ID));
        self.log("history cleared");
    }

    /// Stops running jobs, gives them a moment to end, and exits.
    fn shutdown(&self) -> ! {
        let pids: Vec<u32> = self.running().iter().map(|p| p.pid).collect();
        for pid in &pids {
            signal_group(*pid, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + KILL_GRACE;
        while !self.running().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        for job in self.running() {
            signal_group(job.pid, libc::SIGKILL);
        }
        let _ = std::fs::remove_file(self.home.socket());
        self.log("daemon stopped");
        std::process::exit(0);
    }

    fn save_history(&self, state: &State) {
        let infos: Vec<&ProcessInfo> = state.jobs.values().map(|j| &j.info).collect();
        if let Ok(text) = serde_json::to_string_pretty(&infos) {
            let tmp = self.home.history().with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, self.home.history());
            }
        }
    }
}

struct Spec {
    argv: Vec<String>,
    cwd: std::path::PathBuf,
    env: Vec<(String, String)>,
    target: String,
    directive: String,
    cols: u16,
    rows: u16,
    pty: bool,
    origin: proto::Origin,
    client: Option<String>,
    project: Option<std::path::PathBuf>,
    reuse: bool,
}

/// A started job's process, before it's recorded.
struct Spawned {
    pid: u32,
    /// Output to log, with the stream each is logged as.
    readers: Vec<(&'static str, Box<dyn Read + Send>)>,
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
    input: Option<Box<dyn Write + Send>>,
    /// Blocks until the job ends; returns its exit code, or 128 + signal.
    wait: Box<dyn FnOnce() -> i32 + Send>,
}

fn spawn_pty(spec: &Spec) -> Result<Spawned, String> {
    let program = &spec.argv[0];
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: spec.rows.max(1),
            cols: spec.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("can't open a terminal: {e}"))?;
    let mut cmd = CommandBuilder::new(program);
    cmd.args(&spec.argv[1..]);
    cmd.cwd(&spec.cwd);
    cmd.env_clear();
    for (key, value) in &spec.env {
        cmd.env(key, value);
    }
    // As the C++ godo did: tools see a colour terminal.
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("CLICOLOR_FORCE", "1");
    cmd.env("BAZEL_FORCE_COLOR", "1");
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("can't read the terminal: {e}"))?;
    let input = pair
        .master
        .take_writer()
        .map_err(|e| format!("can't write to the terminal: {e}"))?;
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("can't start {program}: {e}"))?;
    // The job has its own copy; ours would keep the terminal open.
    drop(pair.slave);
    let pid = child.process_id().unwrap_or(0);
    Ok(Spawned {
        pid,
        readers: vec![("out", reader)],
        master: Some(pair.master),
        input: Some(input),
        wait: Box::new(move || match child.wait() {
            Ok(status) if status.signal().is_some() => 128 + signal_number(status.signal()),
            Ok(status) => status.exit_code() as i32,
            Err(_) => -1,
        }),
    })
}

/// The job on pipes, leading its own process group so killing it reaches
/// everything it started, as with a terminal.
fn spawn_pipes(spec: &Spec) -> Result<Spawned, String> {
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    let program = &spec.argv[0];
    let mut child = Command::new(program)
        .args(&spec.argv[1..])
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("can't start {program}: {e}"))?;
    let out = child.stdout.take().expect("piped");
    let err = child.stderr.take().expect("piped");
    Ok(Spawned {
        pid: child.id(),
        readers: vec![("out", Box::new(out)), ("err", Box::new(err))],
        master: None,
        input: None,
        wait: Box::new(move || match child.wait() {
            Ok(status) => status
                .code()
                .or_else(|| status.signal().map(|s| 128 + s))
                .unwrap_or(-1),
            Err(_) => -1,
        }),
    })
}

/// History from a previous daemon. Jobs it left marked running can't be
/// followed any more, so they're recorded as ended, without an exit code.
fn load_history(home: &Home) -> BTreeMap<u64, Job> {
    let infos: Vec<ProcessInfo> = std::fs::read_to_string(home.history())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    infos
        .into_iter()
        .filter(|info| info.id != DAEMON_ID)
        .map(|mut info| {
            if info.running {
                info.running = false;
                info.end_ms.get_or_insert(proto::now_ms());
            }
            let job = Job {
                info,
                clients: Vec::new(),
                log: None,
                master: None,
                input: None,
                waiters: Vec::new(),
            };
            (job.info.id, job)
        })
        .collect()
}

/// Cuts a log back to about its last `keep` bytes once it passes `limit`,
/// at a line boundary so every remaining line is whole.
fn trim_log(path: &std::path::Path, limit: u64, keep: usize) {
    let Ok(len) = std::fs::metadata(path).map(|m| m.len()) else {
        return;
    };
    if len <= limit {
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let from = bytes.len().saturating_sub(keep);
    let start = bytes[from..]
        .iter()
        .position(|&b| b == b'\n')
        .map_or(bytes.len(), |i| from + i + 1);
    let tmp = path.with_extension("log.tmp");
    if std::fs::write(&tmp, &bytes[start..]).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Sends `event` to every client, dropping the ones that are gone.
fn broadcast(clients: &mut Vec<UnixStream>, event: &Event) {
    clients.retain_mut(|client| proto::write(client, event).is_ok());
}

fn signal_group(pid: u32, signal: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if pid <= 0 {
        return;
    }
    // SAFETY: kill() only sends a signal. Jobs lead their own session (the
    // terminal setup calls setsid), so -pid is the job's process group.
    unsafe {
        if libc::kill(-pid, signal) != 0 {
            libc::kill(pid, signal);
        }
    }
}

/// The signal number from a description like `Terminated: 15` (macOS) or
/// `Terminated` (Linux).
fn signal_number(name: Option<&str>) -> i32 {
    let name = name.unwrap_or_default();
    if let Some(n) = name
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|d| d.parse().ok())
    {
        return n;
    }
    let signals = [
        ("Hangup", libc::SIGHUP),
        ("Interrupt", libc::SIGINT),
        ("Killed", libc::SIGKILL),
        ("Terminated", libc::SIGTERM),
        ("Broken pipe", libc::SIGPIPE),
        ("Abort", libc::SIGABRT),
    ];
    signals
        .iter()
        .find(|(n, _)| name.contains(n))
        .map_or(libc::SIGTERM, |(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_numbers_from_either_platforms_wording() {
        assert_eq!(signal_number(Some("Terminated: 15")), 15);
        assert_eq!(signal_number(Some("Killed: 9")), 9);
        assert_eq!(signal_number(Some("Hangup")), libc::SIGHUP);
        assert_eq!(signal_number(Some("Killed")), libc::SIGKILL);
    }

    #[test]
    fn trimming_a_log_keeps_whole_lines_from_the_end() {
        let dir = crate::testdir::TestDir::new("trim");
        let path = dir.path().join("0.log");
        let lines: String = (0..100).map(|i| format!("line {i:03}\n")).collect();
        std::fs::write(&path, &lines).unwrap();
        trim_log(&path, 10_000, 50);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            lines,
            "under the limit"
        );
        trim_log(&path, 100, 50);
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.len() <= 50 && kept.ends_with("line 099\n"), "{kept:?}");
        assert!(
            kept.starts_with("line "),
            "starts on a whole line: {kept:?}"
        );
    }
}
