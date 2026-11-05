// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! End to end: a real daemon per test, with its own `GODO_HOME` under
//! `target/tmp`, running real jobs in pseudo-terminals.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

struct Env {
    root: PathBuf,
    home: PathBuf,
}

impl Env {
    fn new(test: &str) -> Self {
        let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("d-{test}"));
        if base.exists() {
            std::fs::remove_dir_all(&base).unwrap();
        }
        let root = base.join("p");
        let home = base.join("h");
        std::fs::create_dir_all(root.join("app/sub")).unwrap();
        std::fs::write(root.join("godo.toml"), "[targets]\napp = \"app\"\n").unwrap();
        std::fs::write(
            root.join("app/Makefile"),
            "\
hello:
\t@echo \"hello from $$(basename $$(pwd)) foo=$$FOO root=$$(basename $(GODO_ROOT))\"
fail:
\t@echo failing; exit 3
slow:
\t@sleep 30 & echo $$! > slow.pid; echo slow-started; wait
ask:
\t@printf 'name? '; read n; echo \"hello $$n\"
size:
\t@stty size; trap 'stty size' WINCH; for i in 1 2 3 4 5 6 7 8 9 10; do sleep 0.3; done
both:
\t@echo to-out; echo to-err >&2; printf 'progress 1\\rprogress 2\\r\\033[Kdone\\n'
big:
\t@echo FIRST-LINE; head -c 1500000 /dev/zero | tr '\\0' x; echo; echo big-ready; sleep 30
",
        )
        .unwrap();
        Self { root, home }
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_godo"));
        cmd.args(args)
            .current_dir(cwd)
            .env("GODO_HOME", &self.home)
            .env("GODO_DAEMON", "1")
            .env("NO_COLOR", "1")
            .env_remove("GODO_ROOT");
        cmd
    }

    fn godo(&self, args: &[&str]) -> Output {
        self.command(&self.root, args).output().unwrap()
    }

    fn stdout(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.godo(args).stdout).into_owned()
    }

    /// Starts godo and waits until it prints a line containing `marker`.
    /// The caller waits on the child.
    #[allow(clippy::zombie_processes)]
    fn spawn_until(&self, args: &[&str], marker: &str) -> (Child, Vec<String>) {
        let mut child = self
            .command(&self.root, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Keep reading after the caller stops listening, so godo never
            // writes into a closed pipe.
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                // Job output comes as the terminal wrote it, with \r\n.
                let _ = tx.send(line.trim_end_matches('\r').to_string());
            }
        });
        let mut lines = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(line) = rx.recv_timeout(Duration::from_millis(100)) {
                let found = line.contains(marker);
                lines.push(line);
                if found {
                    return (child, lines);
                }
            }
        }
        let _ = child.kill();
        panic!("never saw {marker:?}; got {lines:?}");
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.godo(&["stop", "--force"]);
    }
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Waits for a child to exit, or fails after a while.
fn exits(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = child.kill();
    panic!("the process didn't exit");
}

fn interrupt(child: &Child) {
    // SAFETY: sends SIGINT to a child we started and still own.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGINT);
    }
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_for(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn runs_in_the_daemon_with_the_clients_directory_and_environment() {
    let e = Env::new("run");
    let out = e
        .command(&e.root.join("app/sub"), &["hello", "app"])
        .env("FOO", "from-client")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("Process started with ID: 1 (Ctrl+C to detach, Ctrl+C Ctrl+C to kill)"),
        "{text}"
    );
    assert!(
        text.contains("hello from app foo=from-client root=p"),
        "{text}"
    );
    assert!(text.contains("Process exited with code: 0"), "{text}");
}

#[test]
fn exit_codes_come_back_and_history_records_them() {
    let e = Env::new("exit");
    let out = e.godo(&["fail", "app"]);
    // make exits 2 when a recipe fails, whatever the recipe's own code.
    assert_eq!(
        out.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = e.godo(&["hello", "app"]);
    let history = e.stdout(&["history"]);
    assert!(
        history.contains("Failed") && history.contains("Success"),
        "{history}"
    );
    assert!(
        history.find("hello").unwrap() < history.find("fail").unwrap(),
        "newest first: {history}"
    );
    assert!(e.stdout(&["ps"]).contains("No running processes."));
}

#[test]
fn attach_replays_a_finished_job() {
    let e = Env::new("attach");
    let _ = e.godo(&["hello", "app"]);
    let out = e.godo(&["attach", "-i", "1"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Attached to process ID: 1"), "{text}");
    assert!(text.contains("hello from app"), "{text}");
    assert!(text.contains("Process exited with code: 0"), "{text}");
    let missing = e.godo(&["attach", "-i", "99"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&missing.stderr).contains("no process with ID 99"));
}

#[test]
fn ctrl_c_detaches_and_ctrl_c_twice_kills_the_whole_job() {
    let e = Env::new("kill");
    // One Ctrl+C: detach. The job keeps running.
    let (mut client, _) = e.spawn_until(&["slow", "app"], "slow-started");
    interrupt(&client);
    let status = client.wait().unwrap();
    assert!(status.success(), "detaching exits cleanly");
    let ps = e.stdout(&["ps"]);
    assert!(ps.contains("slow") && ps.contains("Running"), "{ps}");
    let pid_file = e.root.join("app/slow.pid");
    wait_for("the pid file", || pid_file.exists());
    let sleeper: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(alive(sleeper));

    // Attach, then Ctrl+C twice: kill, including the background sleep.
    let (mut client, lines) = e.spawn_until(&["attach", "-i", "1"], "slow-started");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("Attached to process ID: 1")),
        "{lines:?}"
    );
    interrupt(&client);
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&client);
    let status = client.wait().unwrap();
    assert_eq!(status.code(), Some(130));
    wait_for("the job to end", || {
        e.stdout(&["ps"]).contains("No running processes.")
    });
    wait_for("the background sleep to die", || !alive(sleeper));
}

#[test]
fn stop_and_restart_the_daemon() {
    let e = Env::new("stop");
    assert!(e.stdout(&["ps"]).contains("the daemon isn't running"));
    let _ = e.godo(&["hello", "app"]);
    assert!(e.home.join("godo.sock").exists());
    assert!(e.stdout(&["restart"]).contains("Daemon started."));
    // History survives a restart.
    assert!(e.stdout(&["history"]).contains("Success"));
    assert!(e.stdout(&["stop"]).contains("Daemon shutdown requested."));
    assert!(!e.home.join("godo.sock").exists());
    assert!(e.stdout(&["stop"]).contains("Daemon not running."));
}

#[test]
fn history_clear_keeps_nothing_finished() {
    let e = Env::new("clear");
    let _ = e.godo(&["hello", "app"]);
    assert!(e.home.join("logs/1.log").exists());
    assert!(
        e.stdout(&["history", "--clear"])
            .contains("History cleared.")
    );
    assert!(!e.stdout(&["history"]).contains("Success"));
    assert!(!e.home.join("logs/1.log").exists());
}

#[test]
fn the_daemon_directory_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new("private");
    let _ = e.godo(&["hello", "app"]);
    let mode = std::fs::metadata(&e.home).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}

#[test]
fn run_command_goes_through_the_daemon_too() {
    let e = Env::new("runcmd");
    let out = e.godo(&["run", "-c", "echo via-sh; exit 4", "-t", "misc"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&out.stdout).contains("via-sh"));
    let history = e.stdout(&["history"]);
    assert!(
        history.contains("misc") && history.contains("run"),
        "{history}"
    );
}

#[test]
fn attaching_replays_all_of_a_running_jobs_output() {
    let e = Env::new("bigattach");
    let (mut client, _) = e.spawn_until(&["big", "app"], "big-ready");
    interrupt(&client);
    assert!(client.wait().unwrap().success());
    // Over a megabyte before the last line: the first line must still come.
    let (mut client, lines) = e.spawn_until(&["attach", "-i", "1"], "big-ready");
    assert!(
        lines.iter().any(|l| l == "FIRST-LINE"),
        "missing the start: {:?}",
        &lines[..lines.len().min(3)]
    );
    assert_eq!(
        lines.iter().filter(|l| *l == "FIRST-LINE").count(),
        1,
        "and only once"
    );
    interrupt(&client);
    std::thread::sleep(Duration::from_millis(100));
    interrupt(&client);
    client.wait().unwrap();
}

#[test]
fn old_jobs_and_their_logs_are_pruned() {
    let e = Env::new("prune");
    for _ in 0..5 {
        let out = e
            .command(&e.root, &["hello", "app"])
            .env("GODO_HISTORY_LIMIT", "3")
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let history = e.stdout(&["history"]);
    assert_eq!(history.matches("Success").count(), 3, "{history}");
    let mut logs: Vec<String> = std::fs::read_dir(e.home.join("logs"))
        .unwrap()
        .map(|f| f.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    logs.sort();
    assert_eq!(logs, ["0.log", "3.log", "4.log", "5.log"]);
}

#[test]
fn stop_and_restart_refuse_while_jobs_run_unless_forced() {
    let e = Env::new("busy");
    let (mut client, _) = e.spawn_until(&["slow", "app"], "slow-started");
    interrupt(&client);
    client.wait().unwrap();
    for command in ["stop", "restart"] {
        let out = e.godo(&[command]);
        assert_eq!(out.status.code(), Some(1), "{command}");
        let err = stderr(&out);
        assert!(
            err.contains("1 running job: 1 (slow app)") && err.contains("--force"),
            "{err}"
        );
    }
    assert!(e.stdout(&["ps"]).contains("Running"), "still running");
    let pid_file = e.root.join("app/slow.pid");
    wait_for("the pid file", || pid_file.exists());
    let sleeper: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        e.stdout(&["stop", "--force"])
            .contains("Daemon shutdown requested.")
    );
    wait_for("the job's children to stop", || !alive(sleeper));
}

#[test]
fn an_idle_daemon_of_another_version_is_replaced() {
    let e = Env::new("upgrade");
    let mut old = e
        .command(e.home.parent().unwrap(), &["daemon"])
        .env("GODO_VERSION_OVERRIDE", "0.0.1-old")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_for("the old daemon", || e.home.join("godo.sock").exists());
    let out = e.godo(&["hello", "app"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("restarted the daemon (0.0.1-old →"),
        "{}",
        stderr(&out)
    );
    assert!(exits(&mut old).success(), "the old daemon stopped");
}

#[test]
fn a_busy_daemon_of_another_version_is_left_alone() {
    let e = Env::new("upgrade-busy");
    // An "older" client and daemon, with a job still running.
    let mut old = e
        .command(&e.root, &["slow", "app"])
        .env("GODO_VERSION_OVERRIDE", "0.0.1-old")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pid_file = e.root.join("app/slow.pid");
    wait_for("the old job", || pid_file.exists());
    interrupt(&old);
    old.wait().unwrap();
    let out = e.godo(&["hello", "app"]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("version 0.0.1-old and has 1 running job")
            && err.contains("godo restart --force"),
        "{err}"
    );
    // Forcing works across versions.
    assert!(
        e.stdout(&["restart", "--force"])
            .contains("Daemon started.")
    );
    assert!(e.godo(&["hello", "app"]).status.success());
}

#[test]
fn version_names_the_build() {
    let e = Env::new("version");
    let out = e.stdout(&["--version"]);
    let want = format!("godo {}", env!("CARGO_PKG_VERSION"));
    assert!(out.starts_with(&want), "{out}");
}

/// godo running in a real pseudo-terminal, as when typed at a shell: keys
/// written here arrive as keystrokes, Ctrl+C included.
struct Tty {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    output: std::sync::Arc<std::sync::Mutex<String>>,
}

impl Tty {
    fn spawn(e: &Env, args: &[&str]) -> Self {
        let pair = portable_pty::native_pty_system()
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut cmd = portable_pty::CommandBuilder::new(env!("CARGO_BIN_EXE_godo"));
        cmd.args(args);
        cmd.cwd(&e.root);
        cmd.env("GODO_HOME", &e.home);
        cmd.env("TERM", "xterm-256color");
        cmd.env_remove("GODO_DAEMON");
        let child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = std::sync::Arc::clone(&output);
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut reader, &mut buf) {
                if n == 0 {
                    break;
                }
                sink.lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        });
        Self {
            child,
            master: pair.master,
            writer,
            output,
        }
    }

    fn output(&self) -> String {
        self.output.lock().unwrap().clone()
    }

    fn wait_for(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if self.output().contains(text) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("never saw {text:?} in {:?}", self.output());
    }

    fn type_keys(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }

    fn exit_code(&mut self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.exit_code();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        panic!("godo didn't exit: {:?}", self.output());
    }
}

#[test]
fn typing_reaches_the_job() {
    let e = Env::new("tty-input");
    let mut tty = Tty::spawn(&e, &["ask", "app"]);
    tty.wait_for("name? ");
    tty.type_keys(b"bob\r");
    tty.wait_for("hello bob");
    assert_eq!(tty.exit_code(), 0);
}

#[test]
fn ctrl_c_keystrokes_detach_and_kill() {
    let e = Env::new("tty-ctrlc");
    let mut tty = Tty::spawn(&e, &["slow", "app"]);
    tty.wait_for("slow-started");
    tty.type_keys(&[0x03]);
    tty.wait_for("Detaching");
    assert_eq!(tty.exit_code(), 0);
    assert!(
        e.stdout(&["ps"]).contains("Running"),
        "detached, still running"
    );

    let mut tty = Tty::spawn(&e, &["attach", "-i", "1"]);
    tty.wait_for("slow-started");
    tty.type_keys(&[0x03, 0x03]);
    tty.wait_for("Killing process...");
    assert_eq!(tty.exit_code(), 130);
    wait_for("the job to end", || {
        e.stdout(&["ps"]).contains("No running processes.")
    });
}

#[test]
fn resizing_the_terminal_resizes_the_job() {
    let e = Env::new("tty-resize");
    let mut tty = Tty::spawn(&e, &["size", "app"]);
    tty.wait_for("24 80");
    tty.master
        .resize(portable_pty::PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    tty.wait_for("30 100");
    assert_eq!(tty.exit_code(), 0);
}

#[test]
fn progress_output_keeps_its_carriage_returns() {
    let e = Env::new("raw-output");
    let out = e
        .command(
            &e.root,
            &["run", "-c", "printf 'a 10%%\\rb 90%%\\n'", "-t", "p"],
        )
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("a 10%\rb 90%"), "{text:?}");
}

#[test]
fn ps_history_and_attach_work_without_the_daemon() {
    let e = Env::new("offline");
    let _ = e.godo(&["hello", "app"]);
    assert!(e.stdout(&["stop"]).contains("Daemon shutdown requested."));
    assert!(e.stdout(&["ps"]).contains("the daemon isn't running"));
    let history = e.stdout(&["history"]);
    assert!(
        history.contains("hello") && history.contains("Success"),
        "{history}"
    );
    let out = e.godo(&["attach", "-i", "1"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("hello from app") && text.contains("Process exited with code: 0"),
        "{text}"
    );
    assert!(out.status.success());
    assert!(
        !e.home.join("godo.sock").exists(),
        "none of that started the daemon"
    );
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not JSON ({e}): {}{}",
            String::from_utf8_lossy(&out.stdout),
            stderr(out)
        )
    })
}

/// Starts `godo <args> --detach --json` and returns the job's id.
fn detach(e: &Env, args: &[&str]) -> u64 {
    let mut args = args.to_vec();
    args.extend(["--detach", "--json"]);
    let out = e.godo(&args);
    assert!(out.status.success(), "{}", stderr(&out));
    json(&out)["id"].as_u64().unwrap()
}

#[test]
fn detached_jobs_can_be_waited_for_and_read_as_plain_text() {
    let e = Env::new("detach");
    let id = detach(&e, &["both", "app"]);
    let out = e.godo(&["wait", "-i", &id.to_string(), "--timeout", "15s", "--json"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let waited = json(&out);
    assert_eq!(waited["job"]["status"], "success", "{waited}");
    assert_eq!(waited["job"]["origin"], "shell", "{waited}");
    assert_eq!(waited["job"]["pty"], true, "{waited}");
    let logs = e.stdout(&["logs", "-i", &id.to_string()]);
    assert!(logs.contains("to-out\nto-err\ndone\n"), "{logs:?}");
    assert!(
        !logs.contains("progress") && !logs.contains('\x1b'),
        "{logs:?}"
    );
    let grep = e.stdout(&["logs", "-i", &id.to_string(), "--grep", "^to-err"]);
    assert_eq!(grep, "2:to-err\n");
}

#[test]
fn no_pty_runs_on_pipes_and_keeps_stderr_apart() {
    let e = Env::new("nopty");
    let id = detach(&e, &["both", "app", "--no-pty"]);
    let out = e.godo(&["wait", "-i", &id.to_string(), "--timeout", "15s"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let logs = e.stdout(&["logs", "-i", &id.to_string()]);
    assert!(
        logs.contains("to-out") && logs.contains("err:to-err"),
        "{logs:?}"
    );
    let history = json(&e.godo(&["history", "--json"]));
    assert_eq!(history[0]["pty"], false, "{history}");
    // GODO_PTY=0 makes pipes the default.
    let out = e
        .command(&e.root, &["both", "app", "-d", "--json"])
        .env("GODO_PTY", "0")
        .output()
        .unwrap();
    let id = json(&out)["id"].as_u64().unwrap();
    let _ = e.godo(&["wait", "-i", &id.to_string(), "--timeout", "15s"]);
    let history = json(&e.godo(&["history", "--json"]));
    assert_eq!(history[0]["pty"], false, "{history}");
}

#[test]
fn waiting_times_out_and_kill_stops_the_job() {
    let e = Env::new("waitkill");
    let id = detach(&e, &["slow", "app"]).to_string();
    let out = e.godo(&["wait", "-i", &id, "--timeout", "1s"]);
    assert_eq!(out.status.code(), Some(124), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("still running"));
    let ps = json(&e.godo(&["ps", "--json"]));
    assert_eq!(ps.as_array().unwrap().len(), 1, "{ps}");
    assert!(e.godo(&["kill", "-i", &id]).status.success());
    let out = e.godo(&["wait", "-i", &id, "--timeout", "15s", "--json"]);
    assert!(!out.status.success());
    assert_eq!(json(&out)["job"]["status"], "failed");
}

/// `godo mcp`, spoken to over its stdin and stdout.
struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    next: u64,
}

impl Mcp {
    fn start(e: &Env) -> Self {
        let mut child = e
            .command(&e.root, &["mcp"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let mut mcp = Self {
            child,
            stdin,
            lines,
            next: 1,
        };
        let init = mcp.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "test-agent", "version": "1" }
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "godo", "{init}");
        mcp.send(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        mcp
    }

    fn send(&mut self, message: serde_json::Value) {
        use std::io::Write;
        writeln!(self.stdin, "{message}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next;
        self.next += 1;
        self.send(
            serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        );
        loop {
            let line = self
                .lines
                .recv_timeout(Duration::from_secs(30))
                .expect("godo mcp answers");
            let reply: serde_json::Value = serde_json::from_str(&line).unwrap();
            if reply["id"] == id {
                return reply;
            }
        }
    }

    /// Calls a tool; returns its text and whether it's an error.
    fn call(&mut self, tool: &str, args: serde_json::Value) -> (String, bool) {
        let reply = self.request(
            "tools/call",
            serde_json::json!({ "name": tool, "arguments": args }),
        );
        let result = &reply["result"];
        assert!(result.is_object(), "{reply}");
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        (text, result["isError"] == true)
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_lists_runs_and_reports_jobs() {
    let e = Env::new("mcp");
    let mut mcp = Mcp::start(&e);
    let tools = mcp.request("tools/list", serde_json::json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in ["list_targets", "run", "wait", "logs", "list_jobs", "kill"] {
        assert!(names.contains(&name), "{names:?}");
    }
    let (targets, _) = mcp.call("list_targets", serde_json::json!({}));
    assert!(
        targets.contains("app (app)") && targets.contains("  hello"),
        "{targets}"
    );

    let (text, error) = mcp.call(
        "run",
        serde_json::json!({ "target": "app", "directive": "both" }),
    );
    assert!(!error, "{text}");
    assert!(text.contains("job 1: both app: succeeded"), "{text}");
    assert!(text.contains("by an agent"), "{text}");
    assert!(
        text.contains("2| to-err") && text.contains("3| done"),
        "{text}"
    );

    let (text, error) = mcp.call(
        "run",
        serde_json::json!({ "target": "app", "directive": "fail" }),
    );
    assert!(
        !error,
        "a failed build is a result, not a tool error: {text}"
    );
    assert!(text.contains("failed (exit 2)"), "{text}");

    let (text, _) = mcp.call("logs", serde_json::json!({ "id": 1, "grep": "^to-" }));
    assert!(
        text.contains("1| to-out") && text.contains("2| to-err") && !text.contains("done"),
        "{text}"
    );

    let (text, _) = mcp.call("list_jobs", serde_json::json!({}));
    assert!(
        text.find("job 2").unwrap() < text.find("job 1").unwrap(),
        "newest first: {text}"
    );
    let history = json(&e.godo(&["history", "--json"]));
    assert_eq!(history[0]["client"], "test-agent", "{history}");
}

#[test]
fn mcp_returns_running_jobs_reuses_them_and_kills_its_own() {
    let e = Env::new("mcp-slow");
    let mut mcp = Mcp::start(&e);
    let run = serde_json::json!({ "target": "app", "directive": "slow", "wait_seconds": 1 });
    let (text, _) = mcp.call("run", run.clone());
    assert!(text.contains("job 1: slow app: running"), "{text}");
    assert!(
        text.contains("still running: call wait with id 1"),
        "{text}"
    );
    let (text, _) = mcp.call("run", run);
    assert!(
        text.contains("already running") && text.contains("job 1:"),
        "{text}"
    );
    let (text, _) = mcp.call("wait", serde_json::json!({ "id": 1, "timeout_seconds": 1 }));
    assert!(text.contains("running"), "{text}");

    // A job from a terminal or shell isn't the agent's to kill by default.
    let other = detach(&e, &["slow", "app"]);
    let (text, error) = mcp.call("kill", serde_json::json!({ "id": other }));
    assert!(error && text.contains("doesn't let agents kill"), "{text}");
    let (text, error) = mcp.call("kill", serde_json::json!({ "id": 1 }));
    assert!(!error, "{text}");
    let (text, _) = mcp.call(
        "wait",
        serde_json::json!({ "id": 1, "timeout_seconds": 15 }),
    );
    assert!(text.contains("failed"), "{text}");
}

#[test]
fn mcp_follows_the_projects_agent_rules() {
    let e = Env::new("mcp-rules");
    std::fs::write(
        e.root.join("godo.toml"),
        "[targets]\napp = \"app\"\n\n[agents]\ndeny = [\"fail *\"]\n",
    )
    .unwrap();
    let mut mcp = Mcp::start(&e);
    let (targets, _) = mcp.call("list_targets", serde_json::json!({}));
    assert!(
        targets.contains("hello") && !targets.contains("fail"),
        "{targets}"
    );
    let (text, error) = mcp.call(
        "run",
        serde_json::json!({ "target": "app", "directive": "fail" }),
    );
    assert!(
        error && text.contains("doesn't let agents run `fail app`"),
        "{text}"
    );
    let (text, error) = mcp.call(
        "run",
        serde_json::json!({ "target": "app", "directive": "hello", "args": ["FOO=1"] }),
    );
    assert!(error && text.contains("make_args"), "{text}");
    let (text, error) = mcp.call(
        "run",
        serde_json::json!({ "target": "nope", "directive": "hello" }),
    );
    assert!(error && text.contains("no target named `nope`"), "{text}");
}
