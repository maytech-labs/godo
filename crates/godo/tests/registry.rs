// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! End to end: `init`, `register`, `unregister`, `scaff`, and `migrate`
//! against projects under cargo's per-test temp directory (`target/tmp`).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test, inside `target/tmp`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("registry-{test}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn godo(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_godo"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

/// Runs godo and fails the test, with its output, unless it succeeds.
fn ok(cwd: &Path, args: &[&str]) -> String {
    let out = godo(cwd, args);
    assert!(
        out.status.success(),
        "godo {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Runs godo, expects exit code 1, and returns stderr.
fn fails(cwd: &Path, args: &[&str]) -> String {
    let out = godo(cwd, args);
    assert_eq!(
        out.status.code(),
        Some(1),
        "godo {args:?}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn registry(root: &Path) -> String {
    std::fs::read_to_string(root.join("godo.toml")).unwrap()
}

#[test]
fn init_creates_an_empty_registry_once() {
    let root = scratch("init");
    ok(&root, &["init"]);
    assert!(registry(&root).contains("[targets]"));
    assert!(ok(&root, &["list"]).contains("No targets registered"));
    assert!(fails(&root, &["init"]).contains("already exists"));
}

#[test]
fn init_takes_a_path_and_creates_it() {
    let root = scratch("init-path");
    ok(&root, &["init", "-p", "new/project"]);
    assert!(root.join("new/project/godo.toml").is_file());
}

#[test]
fn register_from_a_subdirectory_stores_paths_relative_to_the_root() {
    let root = scratch("register");
    ok(&root, &["init"]);
    std::fs::create_dir_all(root.join("apps/web/src")).unwrap();
    std::fs::write(root.join("apps/web/Makefile"), "dev:\n\t@echo dev ok\n").unwrap();

    // -p is relative to where you are, like any CLI.
    ok(&root.join("apps"), &["register", "-n", "web", "-p", "web"]);
    // No -p: the current directory.
    ok(&root, &["register", "-n", "root"]);

    let text = registry(&root);
    assert!(text.contains("web = \"apps/web\""), "{text}");
    assert!(text.contains("root = \".\""), "{text}");
    assert!(ok(&root.join("apps/web/src"), &["dev", "web"]).contains("dev ok"));
}

#[test]
fn register_keeps_comments_and_updates_existing_names() {
    let root = scratch("register-update");
    std::fs::create_dir_all(root.join("a")).unwrap();
    std::fs::create_dir_all(root.join("b")).unwrap();
    std::fs::write(
        root.join("godo.toml"),
        "# my notes\n[targets]\n# the app\napp = \"a\"\n",
    )
    .unwrap();
    let out = ok(&root, &["register", "-n", "app", "-p", "b"]);
    assert!(out.contains("Updated `app`: a → b"), "{out}");
    let text = registry(&root);
    assert!(
        text.contains("# my notes") && text.contains("# the app"),
        "{text}"
    );
    assert!(text.contains("app = \"b\""), "{text}");
}

#[test]
fn register_warns_about_a_missing_makefile() {
    let root = scratch("register-no-makefile");
    ok(&root, &["init"]);
    std::fs::create_dir_all(root.join("lib")).unwrap();
    let out = godo(&root, &["register", "-n", "lib", "-p", "lib"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no Makefile"));
}

#[test]
fn register_rejects_missing_directories_and_bad_names() {
    let root = scratch("register-bad");
    ok(&root, &["init"]);
    assert!(fails(&root, &["register", "-n", "x", "-p", "nope"]).contains("isn't a directory"));
    assert!(fails(&root, &["register", "-n", "my app"]).contains("can't be a target name"));
    assert!(!registry(&root).contains("x ="));
}

#[test]
fn unregister_removes_only_the_entry() {
    let root = scratch("unregister");
    ok(&root, &["init"]);
    std::fs::create_dir_all(root.join("web")).unwrap();
    ok(&root, &["register", "-n", "web", "-p", "web"]);
    ok(&root, &["unregister", "-n", "web"]);
    assert!(!registry(&root).contains("web ="));
    assert!(root.join("web").is_dir(), "the directory stays");
    assert!(fails(&root, &["unregister", "-n", "web"]).contains("no target named `web`"));
}

#[test]
fn scaff_creates_a_runnable_target() {
    let root = scratch("scaff");
    ok(&root, &["init"]);
    ok(&root, &["scaff", "-n", "tool"]);
    assert!(root.join("tool/Makefile").is_file());
    assert!(registry(&root).contains("tool = \"tool\""));
    assert!(ok(&root, &["build", "tool"]).contains("tool: the build step isn't written yet"));
    assert!(ok(&root, &["list", "-n", "tool"]).contains("Build tool"));
}

#[test]
fn scaff_never_overwrites_a_makefile() {
    let root = scratch("scaff-existing");
    ok(&root, &["init"]);
    std::fs::create_dir_all(root.join("app")).unwrap();
    std::fs::write(root.join("app/Makefile"), "mine:\n").unwrap();
    assert!(fails(&root, &["scaff", "-n", "app"]).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(root.join("app/Makefile")).unwrap(),
        "mine:\n"
    );
}

/// The C++ godo's registry bytes for one target.
fn legacy(name: &str, path: &str) -> Vec<u8> {
    let mut target = vec![0x0a, name.len() as u8];
    target.extend_from_slice(name.as_bytes());
    target.extend([0x12, path.len() as u8]);
    target.extend_from_slice(path.as_bytes());
    let mut out = vec![0x0a, target.len() as u8];
    out.extend(target);
    out
}

#[test]
fn legacy_projects_must_migrate_before_editing() {
    let root = scratch("legacy");
    std::fs::create_dir_all(root.join(".godo")).unwrap();
    std::fs::create_dir_all(root.join("io/web")).unwrap();
    let bytes = legacy("ml:web", "io/web/");
    std::fs::write(root.join(".godo/config"), &bytes).unwrap();

    assert!(fails(&root, &["register", "-n", "x"]).contains("run `godo migrate`"));
    assert!(fails(&root, &["scaff", "-n", "x"]).contains("run `godo migrate`"));
    assert!(
        !root.join("x").exists(),
        "scaff must not create anything first"
    );
    assert!(fails(&root, &["init"]).contains("godo migrate"));

    let out = ok(&root.join("io"), &["migrate"]);
    assert!(out.contains("1 targets"), "{out}");
    assert!(
        registry(&root).contains("\"ml:web\" = \"io/web/\""),
        "{}",
        registry(&root)
    );
    assert_eq!(
        std::fs::read(root.join(".godo/config")).unwrap(),
        bytes,
        "the C++ registry is kept"
    );
    assert!(ok(&root, &["list"]).contains("ml:web"));
    ok(&root, &["register", "-n", "x", "-p", "io"]);
    assert!(fails(&root, &["migrate"]).contains("already uses godo.toml"));
}
