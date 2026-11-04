// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Commands that change the registry: `init`, `register`, `unregister`,
//! `scaff`, and `migrate`.
//!
//! They only write `godo.toml`, editing it in place so comments and layout
//! survive. A project still on the C++ godo's `.godo/config` has to run
//! `godo migrate` first, so the C++ godo's registry is never replaced behind
//! its back.

use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use toml_edit::{DocumentMut, Item, Table, value};

use crate::config::{self, Config, LEGACY_PATH, TOML_PATH, Target};

/// What `godo init` writes.
const NEW_REGISTRY: &str = "\
# godo's registry: `godo <directive> <target>` runs `make <directive>` in the
# target's directory. Add targets with `godo register -n <name> -p <path>`.

[targets]
";

/// `godo init [-p path]`: start a godo project in `path` (default: here).
pub fn init(cwd: &Path, path: Option<&str>) -> clapsmith::Result {
    let dir = resolve(cwd, path.unwrap_or("."));
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    let file = dir.join(TOML_PATH);
    if file.exists() {
        return Err(format!("{} already exists", file.display()).into());
    }
    if dir.join(LEGACY_PATH).is_file() {
        return Err(format!(
            "{} is already a godo project with the C++ godo's {LEGACY_PATH}; run `godo migrate` there to convert it",
            dir.display()
        )
        .into());
    }
    write(&file, NEW_REGISTRY)?;
    done(&format!("Initialized godo in {}", shown(cwd, &dir)));
    Ok(ExitCode::SUCCESS)
}

/// `godo register -n name [-p path]`: add or update a target. The path is
/// relative to the current directory (default: the current directory).
pub fn register(config: &Config, cwd: &Path, name: &str, path: Option<&str>) -> clapsmith::Result {
    check_name(name)?;
    let dir = resolve(cwd, path.unwrap_or("."));
    if !dir.is_dir() {
        return Err(format!("{} isn't a directory", dir.display()).into());
    }
    let stored = relative_to_root(&config.root, &dir);
    let mut doc = load(config)?;
    let table = targets(&mut doc)?;
    let previous = match table.get_mut(name) {
        // Update in place: replacing the entry would drop its comments.
        Some(item) => {
            let old = item.as_str().map(str::to_string);
            let decor = item.as_value().map(|v| v.decor().clone());
            *item = value(stored.as_str());
            if let (Some(decor), Some(new)) = (decor, item.as_value_mut()) {
                *new.decor_mut() = decor;
            }
            old
        }
        None => {
            table.insert(name, value(stored.as_str()));
            None
        }
    };
    save(config, &doc)?;
    match previous {
        Some(old) if old == stored => done(&format!("`{name}` is already registered at {stored}")),
        Some(old) => done(&format!("Updated `{name}`: {old} → {stored}")),
        None => done(&format!("Registered `{name}` at {stored}")),
    }
    if !dir.join("Makefile").is_file() {
        warn(&format!(
            "{stored} has no Makefile yet; `godo scaff -n {name}` creates one"
        ));
    }
    Ok(ExitCode::SUCCESS)
}

/// `godo unregister -n name`: remove a target. Its directory is left alone.
pub fn unregister(config: &Config, name: &str) -> clapsmith::Result {
    let mut doc = load(config)?;
    let Some(old) = targets(&mut doc)?.remove(name) else {
        return Err(crate::targets::unknown_target(config, name).into());
    };
    save(config, &doc)?;
    let path = old.as_str().unwrap_or("?");
    done(&format!("Unregistered `{name}` ({path} is left as it is)"));
    Ok(ExitCode::SUCCESS)
}

/// `godo scaff -n name [-p path]`: create a directory with a starter
/// Makefile and register it. The path defaults to `./<name>`.
pub fn scaff(config: &Config, cwd: &Path, name: &str, path: Option<&str>) -> clapsmith::Result {
    check_name(name)?;
    // Fail before touching the disk if the registry can't be written.
    load(config)?;
    let dir = resolve(cwd, path.unwrap_or(name));
    let makefile = dir.join("Makefile");
    if makefile.exists() {
        return Err(format!(
            "{} already exists; register it with `godo register -n {name} -p {}`",
            makefile.display(),
            path.unwrap_or(name)
        )
        .into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("can't create {}: {e}", dir.display()))?;
    write(&makefile, &starter_makefile(name))?;
    done(&format!("Created {}", shown(cwd, &makefile)));
    let dir = dir.to_string_lossy().into_owned();
    register(config, cwd, name, Some(&dir))
}

fn starter_makefile(name: &str) -> String {
    format!(
        "\
.PHONY: build

build: ## Build {name}
\t@echo \"{name}: the build step isn't written yet\"
"
    )
}

/// `godo migrate`: write `godo.toml` from the C++ godo's `.godo/config`,
/// next to it. The old file is kept for the C++ godo; godo reads
/// `godo.toml` from then on.
pub fn migrate(cwd: &Path) -> clapsmith::Result {
    let config = Config::discover(cwd)?;
    let file = config.root.join(TOML_PATH);
    if config.file == file {
        return Err(format!("{} already uses {TOML_PATH}", config.root.display()).into());
    }
    let mut doc: DocumentMut = NEW_REGISTRY.parse().expect("valid TOML");
    let table = targets(&mut doc)?;
    for Target { name, path } in &config.targets {
        table.insert(name, value(path.as_str()));
    }
    write(&file, &doc.to_string())?;
    done(&format!(
        "Wrote {TOML_PATH} with {} targets from {LEGACY_PATH}",
        config.targets.len()
    ));
    warn(&format!(
        "{LEGACY_PATH} is kept for the C++ godo, but godo now reads {TOML_PATH}; delete {LEGACY_PATH} once you no longer need it"
    ));
    Ok(ExitCode::SUCCESS)
}

/// The registry document, if this project's registry is `godo.toml`.
fn load(config: &Config) -> clapsmith::Result<DocumentMut> {
    if config.file.file_name() != Some(TOML_PATH.as_ref()) {
        return Err(format!(
            "this project's registry is the C++ godo's {LEGACY_PATH}; run `godo migrate` to convert it to {TOML_PATH} first"
        )
        .into());
    }
    let text = std::fs::read_to_string(&config.file)
        .map_err(|e| format!("can't read {}: {e}", config.file.display()))?;
    text.parse::<DocumentMut>().map_err(|e| {
        format!(
            "{} is not valid TOML: {}",
            config.file.display(),
            e.message()
        )
        .into()
    })
}

fn save(config: &Config, doc: &DocumentMut) -> clapsmith::Result<()> {
    let text = doc.to_string();
    // Never write a registry godo itself couldn't read back.
    config::parse_toml(&text).map_err(|e| format!("refusing to write a broken registry: {e}"))?;
    write(&config.file, &text)
}

/// The `[targets]` table, created if missing.
fn targets(doc: &mut DocumentMut) -> clapsmith::Result<&mut Table> {
    let item = doc
        .entry("targets")
        .or_insert_with(|| Item::Table(Table::new()));
    if let Some(inline) = item.as_inline_table() {
        *item = Item::Table(inline.clone().into_table());
    }
    item.as_table_mut()
        .ok_or_else(|| "`targets` in godo.toml should be a table".into())
}

/// Writes through a temporary file and a rename, so a failure part way
/// never leaves a half-written file.
fn write(path: &Path, text: &str) -> clapsmith::Result<()> {
    let tmp = path.with_extension("tmp-godo");
    std::fs::write(&tmp, text).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("can't write {}: {e}", path.display()))?;
    Ok(())
}

fn check_name(name: &str) -> clapsmith::Result<()> {
    if name.is_empty() || name.contains(char::is_whitespace) {
        return Err(format!(
            "`{name}` can't be a target name: use a non-empty name without spaces"
        )
        .into());
    }
    Ok(())
}

/// `path` as given on the command line, relative to `cwd`, without `.` and
/// `..` components.
fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in cwd.join(path).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// How a target's directory is stored: relative to the root when it's
/// inside it (`.` for the root itself), absolute otherwise. Symlinks are
/// resolved on both sides so `/tmp` and `/private/tmp` agree.
fn relative_to_root(root: &Path, dir: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (root, dir) = (canon(root), canon(dir));
    match dir.strip_prefix(&root) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => dir.display().to_string(),
    }
}

/// `path` relative to `cwd` when it's inside it, for messages.
fn shown(cwd: &Path, path: &Path) -> String {
    match path.strip_prefix(cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn done(message: &str) {
    let style = clapsmith::Theme::default().command;
    anstream::println!("{style}✓{style:#} {message}");
}

fn warn(message: &str) {
    let style = clapsmith::Theme::default().invalid;
    anstream::eprintln!("{style}note:{style:#} {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_drops_dot_and_dotdot() {
        assert_eq!(resolve(Path::new("/a/b"), "../c/./d"), Path::new("/a/c/d"));
        assert_eq!(resolve(Path::new("/a/b"), "."), Path::new("/a/b"));
        assert_eq!(resolve(Path::new("/a/b"), "/x/y"), Path::new("/x/y"));
    }

    #[test]
    fn paths_are_stored_relative_to_the_root() {
        let root = Path::new("/nonexistent/root");
        assert_eq!(relative_to_root(root, root), ".");
        assert_eq!(relative_to_root(root, &root.join("apps/web")), "apps/web");
        assert_eq!(
            relative_to_root(root, Path::new("/elsewhere/x")),
            "/elsewhere/x"
        );
    }

    #[test]
    fn names_with_spaces_are_rejected() {
        assert!(check_name("web").is_ok());
        assert!(check_name("ml:tok").is_ok());
        assert!(check_name("").is_err());
        assert!(check_name("my app").is_err());
    }

    #[test]
    fn new_registry_parses_and_is_empty() {
        assert!(config::parse_toml(NEW_REGISTRY).unwrap().is_empty());
    }

    #[test]
    fn inline_targets_table_becomes_a_normal_one() {
        let mut doc: DocumentMut = "targets = { a = \"x\" }\n".parse().unwrap();
        targets(&mut doc).unwrap().insert("b", value("y"));
        let parsed = config::parse_toml(&doc.to_string()).unwrap();
        assert_eq!(parsed.len(), 2);
    }
}
