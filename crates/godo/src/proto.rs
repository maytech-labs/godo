// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Where the daemon keeps its state, and what the client and daemon say to
//! each other.
//!
//! The daemon lives in `$GODO_HOME`, else `$XDG_STATE_HOME/godo`, else
//! `~/.local/state/godo`: the socket, `logs/<id>.log`, and `history.json`.
//! The directory is private (0700), so only its owner can run commands
//! through the daemon. It is separate from the C++ godo's `/tmp/godo.sock`
//! and `~/.godo`, so both can run side by side.
//!
//! Messages are JSON, each prefixed with its length as a big-endian u32.
//! Every connection starts with an [`Envelope`] naming the client's version;
//! a daemon of another version answers with an error instead of guessing.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Bumped when messages change incompatibly.
pub const PROTOCOL: u32 = 1;
/// The largest message accepted, so a bad length can't exhaust memory.
const MAX_MESSAGE: u32 = 64 << 20;

/// The daemon's directory.
#[derive(Clone, Debug)]
pub struct Home {
    pub dir: PathBuf,
}

impl Home {
    /// Finds the directory and creates it, private to the user, if needed.
    pub fn resolve() -> io::Result<Self> {
        let dir = if let Some(dir) = std::env::var_os("GODO_HOME") {
            PathBuf::from(dir)
        } else if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
            PathBuf::from(state).join("godo")
        } else {
            let home =
                std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME isn't set"))?;
            PathBuf::from(home).join(".local/state/godo")
        };
        Self::at(dir)
    }

    pub fn at(dir: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(dir.join("logs"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { dir })
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.join("godo.sock")
    }

    /// Unix sockets have a short path limit (104 bytes on macOS, 108 on
    /// Linux, counting a terminating NUL). Fails with what to do about it.
    pub fn check_socket_path(&self) -> io::Result<()> {
        const LIMIT: usize = 103;
        let len = self.socket().as_os_str().len();
        if len > LIMIT {
            return Err(io::Error::other(format!(
                "the godo daemon's socket path {} is {len} bytes, over the {LIMIT}-byte limit for sockets; set GODO_HOME to a shorter directory",
                self.socket().display()
            )));
        }
        Ok(())
    }

    pub fn logs(&self) -> PathBuf {
        self.dir.join("logs")
    }

    pub fn log(&self, id: u64) -> PathBuf {
        self.logs().join(format!("{id}.log"))
    }

    pub fn history(&self) -> PathBuf {
        self.dir.join("history.json")
    }
}

/// The first and only message a client sends on a connection.
#[derive(Serialize, Deserialize, Debug)]
pub struct Envelope {
    pub protocol: u32,
    pub version: String,
    pub request: Request,
}

impl Envelope {
    pub fn new(request: Request) -> Self {
        Self {
            protocol: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").to_string(),
            request,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Start a job and stream its output on this connection.
    Run {
        argv: Vec<String>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        target: String,
        directive: String,
        cols: u16,
        rows: u16,
    },
    /// Every job the daemon knows, with the daemon itself first.
    Ps,
    History,
    HistoryClear,
    /// Replay a job's output, then stream it if it's still running.
    Attach {
        id: u64,
    },
    /// Stop a job and everything it started.
    Kill {
        id: u64,
    },
    /// Stream output of every job for `directive target`, as it happens.
    Subscribe {
        target: String,
        directive: String,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started { id: u64 },
    Attached { id: u64 },
    Subscribed,
    Log { stream: String, data: String },
    Exit { code: i32 },
    Processes { processes: Vec<ProcessInfo> },
    Ok,
    Error { message: String },
}

/// A job, as `ps`, `history`, and `watch` see it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub id: u64,
    pub pid: u32,
    pub command: String,
    pub target: String,
    pub directive: String,
    pub running: bool,
    /// `None` while running, or if the daemon stopped before the job ended.
    pub exit_code: Option<i32>,
    /// Milliseconds since the Unix epoch.
    pub start_ms: u64,
    pub end_ms: Option<u64>,
}

impl ProcessInfo {
    /// How `watch` groups jobs: `"<directive> <target>"`.
    pub fn key(&self) -> String {
        format!("{} {}", self.directive, self.target)
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// `YYYY-MM-DD HH:MM:SS` in local time, like the C++ godo printed.
pub fn local_time(ms: u64) -> String {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which outlives the call.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            return String::new();
        }
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

pub fn write<T: Serialize>(out: &mut impl Write, message: &T) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    let len = u32::try_from(body.len()).map_err(|_| io::Error::other("message too large"))?;
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(&body);
    out.write_all(&frame)?;
    out.flush()
}

/// Reads one message. `Ok(None)` means the other side closed cleanly.
pub fn read<T: for<'de> Deserialize<'de>>(input: &mut impl Read) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes(len);
    if len > MAX_MESSAGE {
        return Err(io::Error::other(format!(
            "message of {len} bytes is too large"
        )));
    }
    let mut body = vec![0u8; len as usize];
    input.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

/// Turns a byte stream into text without splitting a UTF-8 character
/// across two chunks. Invalid bytes become U+FFFD.
#[derive(Default)]
pub struct Utf8Chunker {
    pending: Vec<u8>,
}

impl Utf8Chunker {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    out.push_str(text);
                    self.pending.clear();
                    return out;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.push_str(
                        std::str::from_utf8(&self.pending[..valid]).expect("valid prefix"),
                    );
                    match e.error_len() {
                        // An incomplete character at the end: wait for the rest.
                        None => {
                            self.pending.drain(..valid);
                            return out;
                        }
                        Some(bad) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid + bad);
                        }
                    }
                }
            }
        }
    }
}

/// Removes a socket file only if nothing answers on it.
pub fn remove_stale_socket(path: &Path) -> io::Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Ok(false);
    }
    std::fs::remove_file(path)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_frames() {
        let mut buf = Vec::new();
        let request = Envelope::new(Request::Kill { id: 7 });
        write(&mut buf, &request).unwrap();
        write(&mut buf, &Event::Exit { code: 2 }).unwrap();
        let mut input = buf.as_slice();
        let back: Envelope = read(&mut input).unwrap().unwrap();
        assert_eq!(back.request, Request::Kill { id: 7 });
        assert_eq!(
            read::<Event>(&mut input).unwrap(),
            Some(Event::Exit { code: 2 })
        );
        assert_eq!(read::<Event>(&mut input).unwrap(), None);
    }

    #[test]
    fn oversized_lengths_are_rejected() {
        let frame = [0xff, 0xff, 0xff, 0xff];
        assert!(read::<Event>(&mut frame.as_slice()).is_err());
    }

    #[test]
    fn utf8_split_across_chunks_is_kept_whole() {
        let mut c = Utf8Chunker::default();
        let bytes = "a●b".as_bytes();
        assert_eq!(c.push(&bytes[..2]), "a");
        assert_eq!(c.push(&bytes[2..]), "●b");
        assert_eq!(c.push(b"\xffz"), "\u{FFFD}z");
    }
}
