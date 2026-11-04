// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Job logs on disk: `logs/<id>.log`, one JSON object per line,
//! `{"t": ms, "s": "out" | "err", "d": "text"}`. Appending a line is a
//! single write, so readers only ever see whole lines or nothing.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Chunk {
    t: u64,
    s: String,
    d: String,
}

pub struct LogWriter {
    file: File,
}

impl LogWriter {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file })
    }

    pub fn write(&mut self, stream: &str, data: &str) -> io::Result<()> {
        let chunk = Chunk {
            t: crate::proto::now_ms(),
            s: stream.to_string(),
            d: data.to_string(),
        };
        let mut line = serde_json::to_vec(&chunk).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)
    }
}

/// How the C++ godo marked stderr in logs.
pub fn styled(stream: &str, data: &str) -> String {
    if stream == "err" {
        format!("\x1b[31merr:\x1b[0m{data}")
    } else {
        data.to_string()
    }
}

/// The text of complete lines in `bytes`, and how many bytes they used.
fn parse(bytes: &[u8]) -> (String, usize) {
    let mut out = String::new();
    let mut used = 0;
    while let Some(end) = bytes[used..].iter().position(|&b| b == b'\n') {
        let line = &bytes[used..used + end];
        used += end + 1;
        match serde_json::from_slice::<Chunk>(line) {
            Ok(chunk) => out.push_str(&styled(&chunk.s, &chunk.d)),
            // A line that isn't a chunk is shown as it is.
            Err(_) => {
                out.push_str(&String::from_utf8_lossy(line));
                out.push('\n');
            }
        }
    }
    (out, used)
}

/// The whole log of one job, or empty if it has none.
pub fn read(path: &Path) -> String {
    std::fs::read(path).map(|b| parse(&b).0).unwrap_or_default()
}

/// Follows a set of log files, returning only what's new on each call.
#[derive(Default, Clone)]
pub struct Tailer {
    offsets: HashMap<PathBuf, u64>,
}

impl Tailer {
    /// Reads every file from the start, remembering where each ended.
    pub fn read_all(&mut self, paths: &[PathBuf]) -> String {
        self.offsets.clear();
        self.read_new(paths)
    }

    /// What was appended since the last call. A file that shrank (history
    /// cleared) is read again from the start.
    pub fn read_new(&mut self, paths: &[PathBuf]) -> String {
        let mut out = String::new();
        for path in paths {
            let offset = self.offsets.entry(path.clone()).or_insert(0);
            let Ok(mut file) = File::open(path) else {
                continue;
            };
            let Ok(len) = file.metadata().map(|m| m.len()) else {
                continue;
            };
            if len < *offset {
                *offset = 0;
            }
            if len == *offset || file.seek(SeekFrom::Start(*offset)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_err() {
                continue;
            }
            let (text, used) = parse(&bytes);
            *offset += used as u64;
            out.push_str(&text);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("godo-logs-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("1.log")
    }

    #[test]
    fn written_chunks_read_back_with_stderr_marked() {
        let path = temp("roundtrip");
        let mut w = LogWriter::open(&path).unwrap();
        w.write("out", "hello\n").unwrap();
        w.write("err", "oops\n").unwrap();
        assert_eq!(read(&path), "hello\n\x1b[31merr:\x1b[0moops\n");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn tailer_returns_only_new_complete_lines() {
        let path = temp("tail");
        let mut w = LogWriter::open(&path).unwrap();
        w.write("out", "one ").unwrap();
        let mut t = Tailer::default();
        let paths = vec![path.clone()];
        assert_eq!(t.read_all(&paths), "one ");
        assert_eq!(t.read_new(&paths), "");
        w.write("out", "two").unwrap();
        // Half a line isn't returned until the rest arrives.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"{\"t\":0,\"s\":\"out\"")
            .unwrap();
        assert_eq!(t.read_new(&paths), "two");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b",\"d\":\"!\"}\n")
            .unwrap();
        assert_eq!(t.read_new(&paths), "!");
        std::fs::remove_file(&path).unwrap();
    }
}
