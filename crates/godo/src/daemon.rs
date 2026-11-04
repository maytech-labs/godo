// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! The godo daemon: runs jobs in pseudo-terminals, keeps their output in
//! memory and in `logs/<id>.log`, and streams it to clients that run,
//! attach to, or watch them.
//!
//! Unlike the C++ daemon, every job runs with the working directory,
//! environment, and terminal size of the client that started it (not the
//! daemon's), and killing a job signals its whole process group.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::logs::LogWriter;
use crate::proto::{self, Envelope, Event, Home, PROTOCOL, ProcessInfo, Request};

/// Output kept in memory per job, for attaching; the rest is on disk.
const BUFFER_LIMIT: usize = 1 << 20;
/// How long a client may stall a write before it's dropped.
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
/// Time between SIGTERM and SIGKILL when killing a job.
const KILL_GRACE: Duration = Duration::from_secs(3);

/// The id the daemon itself has in `ps` and `watch`.
const DAEMON_ID: u64 = 0;

struct Job {
    info: ProcessInfo,
    /// Recent output, at most [`BUFFER_LIMIT`] bytes.
    buffer: String,
    /// Clients that ran or attached to this job.
    clients: Vec<UnixStream>,
    /// Kept for the job's lifetime; closing it hangs up the terminal.
    master: Option<Box<dyn portable_pty::MasterPty + Send>>,
}

struct State {
    next_id: u64,
    /// Every job since history was last cleared, by id.
    jobs: BTreeMap<u64, Job>,
    /// Clients of `watch`, by `"<directive> <target>"`.
    subscribers: HashMap<String, Vec<UnixStream>>,
    daemon: ProcessInfo,
}

struct Daemon {
    home: Home,
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
    let jobs = load_history(&home);
    let next_id = jobs.keys().next_back().map_or(1, |id| id + 1);
    let daemon = Arc::new(Daemon {
        state: Mutex::new(State {
            next_id,
            jobs,
            subscribers: HashMap::new(),
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
            },
        }),
        home,
    });
    daemon.log(&format!("daemon started; socket={}", socket.display()));
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
        let envelope = match proto::read::<Envelope>(&mut stream) {
            Ok(Some(envelope)) => envelope,
            _ => return,
        };
        let ours = env!("CARGO_PKG_VERSION");
        if envelope.protocol != PROTOCOL || envelope.version != ours {
            let message = format!(
                "the godo daemon is version {ours} but this godo is {}; run `godo restart`",
                envelope.version
            );
            let _ = proto::write(&mut stream, &Event::Error { message });
            return;
        }
        let reply = match envelope.request {
            Request::Run {
                argv,
                cwd,
                env,
                target,
                directive,
                cols,
                rows,
            } => {
                let spec = Spec {
                    argv,
                    cwd,
                    env,
                    target,
                    directive,
                    cols,
                    rows,
                };
                match self.start(spec, stream) {
                    Ok(()) => return,
                    Err((mut stream, message)) => {
                        let _ = proto::write(&mut stream, &Event::Error { message });
                        return;
                    }
                }
            }
            Request::Attach { id } => return self.attach(id, stream),
            Request::Subscribe { target, directive } => {
                if proto::write(&mut stream, &Event::Subscribed).is_ok() {
                    let key = format!("{directive} {target}");
                    self.lock().subscribers.entry(key).or_default().push(stream);
                }
                return;
            }
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
            Request::Shutdown => {
                self.log("shutdown requested");
                let _ = proto::write(&mut stream, &Event::Ok);
                self.shutdown();
            }
        };
        let _ = proto::write(&mut stream, &reply);
    }

    fn start(self: &Arc<Self>, spec: Spec, stream: UnixStream) -> Result<(), (UnixStream, String)> {
        let Some(program) = spec.argv.first() else {
            return Err((stream, "nothing to run".into()));
        };
        let pair = match native_pty_system().openpty(PtySize {
            rows: spec.rows.max(1),
            cols: spec.cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(pair) => pair,
            Err(e) => return Err((stream, format!("can't open a terminal: {e}"))),
        };
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
        let reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(e) => return Err((stream, format!("can't read the terminal: {e}"))),
        };

        let mut state = self.lock();
        let id = state.next_id;
        let mut child = match pair.slave.spawn_command(cmd) {
            Ok(child) => child,
            Err(e) => {
                drop(state);
                return Err((stream, format!("can't start {program}: {e}")));
            }
        };
        // The job has its own copy; ours would keep the terminal open.
        drop(pair.slave);
        state.next_id += 1;
        let mut stream = stream;
        if proto::write(&mut stream, &Event::Started { id }).is_err() {
            // The client left already; the job still runs, like a detach.
        }
        let info = ProcessInfo {
            id,
            pid: child.process_id().unwrap_or(0),
            command: spec.argv.join(" "),
            target: spec.target,
            directive: spec.directive,
            running: true,
            exit_code: None,
            start_ms: proto::now_ms(),
            end_ms: None,
        };
        state.jobs.insert(
            id,
            Job {
                info,
                buffer: String::new(),
                clients: vec![stream],
                master: Some(pair.master),
            },
        );
        self.save_history(&state);
        drop(state);

        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let daemon = Arc::clone(self);
        std::thread::spawn(move || {
            daemon.pump(id, reader);
            let _ = done_tx.send(());
        });
        let daemon = Arc::clone(self);
        std::thread::spawn(move || {
            let status = child.wait();
            // Let the output finish. A background process the job left
            // behind can keep the terminal open, so don't wait forever.
            let _ = done_rx.recv_timeout(Duration::from_secs(2));
            let code = match status {
                Ok(status) if status.signal().is_some() => 128 + signal_number(status.signal()),
                Ok(status) => status.exit_code() as i32,
                Err(_) => -1,
            };
            daemon.finish(id, code);
        });
        Ok(())
    }

    /// Copies a job's output to its log, its buffer, and its clients.
    fn pump(&self, id: u64, mut reader: Box<dyn Read + Send>) {
        let mut writer = LogWriter::open(&self.home.log(id)).ok();
        let mut chunker = proto::Utf8Chunker::default();
        let mut buf = [0u8; 4096];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let text = normalize_newlines(&chunker.push(&buf[..n]));
            if text.is_empty() {
                continue;
            }
            if let Some(w) = writer.as_mut() {
                let _ = w.write("out", &text);
            }
            let event = Event::Log {
                stream: "out".into(),
                data: text.clone(),
            };
            let mut state = self.lock();
            let key = match state.jobs.get_mut(&id) {
                Some(job) => {
                    job.buffer.push_str(&text);
                    trim_front(&mut job.buffer, BUFFER_LIMIT);
                    broadcast(&mut job.clients, &event);
                    job.info.key()
                }
                None => continue,
            };
            if let Some(subs) = state.subscribers.get_mut(&key) {
                broadcast(subs, &event);
            }
        }
    }

    fn finish(&self, id: u64, code: i32) {
        let event = Event::Exit { code };
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(&id) else {
            return;
        };
        job.info.running = false;
        job.info.exit_code = Some(code);
        job.info.end_ms = Some(proto::now_ms());
        job.master = None;
        broadcast(&mut job.clients, &event);
        job.clients.clear();
        let key = job.info.key();
        if let Some(subs) = state.subscribers.get_mut(&key) {
            broadcast(subs, &event);
        }
        self.save_history(&state);
    }

    fn attach(&self, id: u64, mut stream: UnixStream) {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(&id) else {
            drop(state);
            let message = format!("no process with ID {id}");
            let _ = proto::write(&mut stream, &Event::Error { message });
            return;
        };
        let backlog = if job.info.running {
            job.buffer.clone()
        } else {
            crate::logs::read(&self.home.log(id))
        };
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
            job.clients.push(stream);
        } else {
            let code = job.info.exit_code.unwrap_or(-1);
            let _ = proto::write(&mut stream, &Event::Exit { code });
        }
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

    fn shutdown(&self) -> ! {
        let state = self.lock();
        for job in state.jobs.values().filter(|j| j.info.running) {
            signal_group(job.info.pid, libc::SIGTERM);
        }
        let _ = std::fs::remove_file(self.home.socket());
        drop(state);
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
                buffer: String::new(),
                clients: Vec::new(),
                master: None,
            };
            (job.info.id, job)
        })
        .collect()
}

/// Sends `event` to every client, dropping the ones that are gone.
fn broadcast(clients: &mut Vec<UnixStream>, event: &Event) {
    clients.retain_mut(|client| proto::write(client, event).is_ok());
}

/// As the C++ godo did: `\r\n` and lone `\r` both become `\n`.
fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Keeps the last `limit` bytes, cut at a character boundary.
fn trim_front(text: &mut String, limit: usize) {
    if text.len() <= limit {
        return;
    }
    let mut cut = text.len() - limit;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    text.drain(..cut);
}

fn signal_group(pid: u32, signal: libc::c_int) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: kill() only sends a signal. Jobs lead their own session (the
    // terminal setup calls setsid), so -pid is the job's process group.
    unsafe {
        if libc::kill(-pid, signal) != 0 {
            libc::kill(pid, signal);
        }
    }
}

fn signal_number(name: Option<&str>) -> i32 {
    let signals = [
        ("Hangup", libc::SIGHUP),
        ("Interrupt", libc::SIGINT),
        ("Killed", libc::SIGKILL),
        ("Terminated", libc::SIGTERM),
        ("Broken pipe", libc::SIGPIPE),
        ("Abort", libc::SIGABRT),
    ];
    let name = name.unwrap_or_default();
    signals
        .iter()
        .find(|(n, _)| name.contains(n))
        .map_or(libc::SIGTERM, |(_, s)| *s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carriage_returns_become_newlines() {
        assert_eq!(normalize_newlines("a\r\nb\rc\n"), "a\nb\nc\n");
    }

    #[test]
    fn buffer_keeps_the_end_at_a_char_boundary() {
        // ● is 3 bytes: cutting 3 bytes from the front would split it.
        let mut s = "ab●cd".to_string();
        trim_front(&mut s, 4);
        assert_eq!(s, "cd");
        let mut s = "abcdef".to_string();
        trim_front(&mut s, 3);
        assert_eq!(s, "def");
    }
}
