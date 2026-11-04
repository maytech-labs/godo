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
                let _ = tx.send(line);
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
        let _ = self.godo(&["stop"]);
    }
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
    assert!(e.stdout(&["ps"]).contains("Daemon not running."));
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
