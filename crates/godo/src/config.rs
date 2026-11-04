// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Finding the godo root and reading its target registry.
//!
//! The root is the nearest directory, from the current one upwards, that
//! holds a registry. Two formats are read:
//!
//! - `godo.toml`, the Rust godo's format:
//!
//!   ```toml
//!   [targets]
//!   web = "apps/web"
//!   ```
//!
//! - `.godo/config`, the C++ godo's: protobuf
//!   `godo.proto.Config { repeated Target targets = 1; }` with
//!   `Target { string name = 1; string path = 2; }`, read here without a
//!   protobuf dependency.
//!
//! If a directory has both, `godo.toml` wins.

use std::fmt;
use std::path::{Path, PathBuf};

/// The registry file, relative to the root.
pub const TOML_PATH: &str = "godo.toml";
/// The C++ godo's registry, relative to the root.
pub const LEGACY_PATH: &str = ".godo/config";

/// A named directory holding a Makefile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub name: String,
    /// As written in the registry: relative to the root, or absolute.
    pub path: String,
}

#[derive(Debug)]
pub struct Config {
    /// The directory holding the registry. Exported to make as `GODO_ROOT`.
    pub root: PathBuf,
    /// The registry file that was read.
    pub file: PathBuf,
    pub targets: Vec<Target>,
}

#[derive(Debug)]
pub enum Error {
    NoRoot {
        from: PathBuf,
    },
    Read {
        path: PathBuf,
        error: std::io::Error,
    },
    Malformed {
        path: PathBuf,
        reason: String,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoRoot { from } => write!(
                f,
                "no {TOML_PATH} or {LEGACY_PATH} in {} or any parent directory; run godo from inside a godo project",
                from.display()
            ),
            Error::Read { path, error } => write!(f, "can't read {}: {error}", path.display()),
            Error::Malformed { path, reason } => {
                write!(f, "{} is not a godo registry: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for Error {}

impl Config {
    /// Finds the root from `start` upwards and reads its registry.
    pub fn discover(start: &Path) -> Result<Self, Error> {
        for dir in start.ancestors() {
            let toml = dir.join(TOML_PATH);
            if toml.is_file() {
                let text = std::fs::read_to_string(&toml).map_err(|error| Error::Read {
                    path: toml.clone(),
                    error,
                })?;
                return Self::read(dir, toml, |_| parse_toml(&text));
            }
            let legacy = dir.join(LEGACY_PATH);
            if legacy.is_file() {
                let bytes = std::fs::read(&legacy).map_err(|error| Error::Read {
                    path: legacy.clone(),
                    error,
                })?;
                return Self::read(dir, legacy, |_| decode(&bytes).map_err(String::from));
            }
        }
        Err(Error::NoRoot {
            from: start.to_path_buf(),
        })
    }

    fn read(
        root: &Path,
        file: PathBuf,
        parse: impl FnOnce(&Path) -> Result<Vec<Target>, String>,
    ) -> Result<Self, Error> {
        let targets = parse(&file).map_err(|reason| Error::Malformed {
            path: file.clone(),
            reason,
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            file,
            targets,
        })
    }

    pub fn target(&self, name: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.name == name)
    }

    /// The target's directory on disk.
    pub fn dir(&self, target: &Target) -> PathBuf {
        self.root.join(&target.path)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlRegistry {
    #[serde(default)]
    targets: toml::Table,
}

/// Parses `godo.toml`. Targets keep the order they're written in.
pub fn parse_toml(text: &str) -> Result<Vec<Target>, String> {
    let registry: TomlRegistry = toml::from_str(text).map_err(|e| e.message().to_string())?;
    registry
        .targets
        .into_iter()
        .map(|(name, path)| match path {
            toml::Value::String(path) => Ok(Target { name, path }),
            other => Err(format!(
                "target `{name}` should be a path in quotes, not a {}",
                other.type_str()
            )),
        })
        .collect()
}

/// Decodes the registry. Unknown fields are skipped, as protobuf requires.
pub fn decode(bytes: &[u8]) -> Result<Vec<Target>, &'static str> {
    let mut targets = Vec::new();
    for field in Fields(bytes) {
        if let (1, Value::Bytes(message)) = field? {
            let mut name = None;
            let mut path = None;
            for field in Fields(message) {
                match field? {
                    (1, Value::Bytes(b)) => name = Some(text(b)?),
                    (2, Value::Bytes(b)) => path = Some(text(b)?),
                    _ => {}
                }
            }
            match (name, path) {
                (Some(name), Some(path)) => targets.push(Target { name, path }),
                _ => return Err("a target is missing its name or path"),
            }
        }
    }
    Ok(targets)
}

/// Encodes targets the way [`decode`] reads them.
#[cfg(test)]
pub fn encode(targets: &[Target]) -> Vec<u8> {
    fn bytes(out: &mut Vec<u8>, field: u8, value: &[u8]) {
        out.push(field << 3 | 2);
        varint(out, value.len() as u64);
        out.extend_from_slice(value);
    }
    fn varint(out: &mut Vec<u8>, mut n: u64) {
        while n >= 0x80 {
            out.push((n as u8) | 0x80);
            n >>= 7;
        }
        out.push(n as u8);
    }
    let mut out = Vec::new();
    for target in targets {
        let mut message = Vec::new();
        bytes(&mut message, 1, target.name.as_bytes());
        bytes(&mut message, 2, target.path.as_bytes());
        bytes(&mut out, 1, &message);
    }
    out
}

fn text(bytes: &[u8]) -> Result<String, &'static str> {
    String::from_utf8(bytes.to_vec()).map_err(|_| "a name or path isn't UTF-8")
}

enum Value<'a> {
    Varint,
    Bytes(&'a [u8]),
    Fixed,
}

/// Protobuf fields as `(number, value)`.
struct Fields<'a>(&'a [u8]);

impl<'a> Iterator for Fields<'a> {
    type Item = Result<(u64, Value<'a>), &'static str>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }
        Some(self.field())
    }
}

impl<'a> Fields<'a> {
    fn field(&mut self) -> Result<(u64, Value<'a>), &'static str> {
        let key = self.varint()?;
        let value = match key & 7 {
            0 => {
                self.varint()?;
                Value::Varint
            }
            1 => {
                self.take(8)?;
                Value::Fixed
            }
            2 => {
                let len = usize::try_from(self.varint()?).map_err(|_| "length too large")?;
                Value::Bytes(self.take(len)?)
            }
            5 => {
                self.take(4)?;
                Value::Fixed
            }
            _ => return Err("unsupported field type"),
        };
        Ok((key >> 3, value))
    }

    fn varint(&mut self) -> Result<u64, &'static str> {
        let mut n = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = self.0.split_first().ok_or("truncated")?;
            self.0 = rest;
            n |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(n);
            }
        }
        Err("varint too long")
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        if self.0.len() < len {
            return Err("truncated");
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(name: &str, path: &str) -> Target {
        Target {
            name: name.into(),
            path: path.into(),
        }
    }

    #[test]
    fn reads_bytes_written_by_the_cpp_godo() {
        // The first two entries of maytech's .godo/config.
        let bytes =
            b"\x0a\x19\x0a\x04godo\x12\x11io/may/tech/godo/\x0a\x0f\x0a\x05tools\x12\x06tools/";
        assert_eq!(
            decode(bytes).unwrap(),
            [
                target("godo", "io/may/tech/godo/"),
                target("tools", "tools/")
            ]
        );
    }

    #[test]
    fn round_trips_long_and_odd_names() {
        let targets = vec![
            target("ml:tokenizer", "io/may/tech/ml/tokenizer/"),
            target("faker", &"/x".repeat(100)),
        ];
        assert_eq!(decode(&encode(&targets)).unwrap(), targets);
    }

    #[test]
    fn toml_keeps_order_and_allows_colons_in_quoted_names() {
        let text = "[targets]\nweb = \"apps/web\"\n\"ml:tok\" = \"ml/tok\"\napi = \"/abs/api\"\n";
        assert_eq!(
            parse_toml(text).unwrap(),
            [
                target("web", "apps/web"),
                target("ml:tok", "ml/tok"),
                target("api", "/abs/api")
            ]
        );
    }

    #[test]
    fn toml_errors_name_the_problem() {
        let err = parse_toml("[targets]\nweb = 3\n").unwrap_err();
        assert!(err.contains("`web`") && err.contains("integer"), "{err}");
        let err = parse_toml("[target]\nweb = \"x\"\n").unwrap_err();
        assert!(err.contains("target"), "{err}");
        assert!(parse_toml("").unwrap().is_empty());
    }

    #[test]
    fn empty_file_is_an_empty_registry() {
        assert!(decode(b"").unwrap().is_empty());
    }

    #[test]
    fn rejects_truncated_and_incomplete_entries() {
        let bytes = encode(&[target("a", "b")]);
        assert!(decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(decode(b"\x0a\x03\x0a\x01a").is_err());
    }

    #[test]
    fn skips_unknown_fields() {
        let mut bytes = b"\x10\x05".to_vec();
        bytes.extend(encode(&[target("a", "b")]));
        assert_eq!(decode(&bytes).unwrap(), [target("a", "b")]);
    }
}
