// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! End to end: a godo project under cargo's per-test temp directory
//! (`target/tmp`), and the real `godo` binary running real `make`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The registry bytes for `(name, path)` pairs, as the C++ godo writes them.
fn registry(targets: &[(&str, &str)]) -> Vec<u8> {
    let field = |out: &mut Vec<u8>, number: u8, value: &[u8]| {
        out.push(number << 3 | 2);
        out.push(u8::try_from(value.len()).unwrap());
        out.extend_from_slice(value);
    };
    let mut out = Vec::new();
    for (name, path) in targets {
        let mut target = Vec::new();
        field(&mut target, 1, name.as_bytes());
        field(&mut target, 2, path.as_bytes());
        field(&mut out, 1, &target);
    }
    out
}

/// A fresh directory for one test, inside `target/tmp`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("godo-{test}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Project {
    root: PathBuf,
}

impl Project {
    fn new(test: &str) -> Self {
        let root = scratch(test);
        std::fs::create_dir_all(root.join(".godo")).unwrap();
        std::fs::create_dir_all(root.join("apps/web/src")).unwrap();
        std::fs::write(
            root.join(".godo/config"),
            registry(&[("web", "apps/web/"), ("ml:tok", "apps/web")]),
        )
        .unwrap();
        std::fs::write(
            root.join("apps/web/Makefile"),
            "\
.PHONY: dev fail
dev: ## Start the dev server
\t@echo \"dev in $$(basename $$(pwd)) root=$(GODO_ROOT) args=$(ARGS)\"
fail:
\t@exit 7
",
        )
        .unwrap();
        Self { root }
    }

    fn godo(&self, cwd: &Path, args: &[&str]) -> Output {
        godo_in(cwd, args)
    }
}

fn godo_in(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_godo"))
        .args(args)
        .current_dir(cwd)
        .env_remove("GODO_ROOT")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn runs_the_directive_in_the_target_from_a_subdirectory() {
    let p = Project::new("run");
    let out = p.godo(&p.root.join("apps/web/src"), &["dev", "web"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("dev in web"), "{text}");
    assert!(!text.contains("Entering directory"), "{text}");
    assert!(
        text.contains(&format!("root={}", p.root.display())),
        "GODO_ROOT should be the project root: {text}"
    );
}

#[test]
fn passes_extra_words_to_make() {
    let p = Project::new("extra");
    let out = p.godo(&p.root, &["dev", "web", "ARGS=hello"]);
    assert!(stdout(&out).contains("args=hello"), "{}", stdout(&out));
}

#[test]
fn target_names_can_contain_colons() {
    let p = Project::new("colon");
    let out = p.godo(&p.root, &["dev", "ml:tok"]);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn exit_code_is_makes() {
    let p = Project::new("exit");
    let out = p.godo(&p.root, &["fail", "web"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "make exits 2 when a recipe fails"
    );
}

#[test]
fn unknown_target_suggests_close_names() {
    let p = Project::new("unknown");
    let out = p.godo(&p.root, &["dev", "we"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("Did you mean: web"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn directive_without_target_explains_itself() {
    let p = Project::new("lonely");
    let out = p.godo(&p.root, &["dev"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("godo dev <target>"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn outside_a_project_says_so() {
    // Not under target/tmp: that's inside this repo, which is a godo project.
    let dir = std::env::temp_dir().join(format!("godo-outside-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = godo_in(&dir, &["dev", "web"]);
    std::fs::remove_dir(&dir).unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("no godo.toml or .godo/config") && err.contains("any parent directory"),
        "{err}"
    );
}

#[test]
fn godo_toml_is_read_and_wins_over_the_legacy_registry() {
    let p = Project::new("toml");
    // The legacy file names `web`; godo.toml renames it, so only `site` works.
    std::fs::write(p.root.join("godo.toml"), "[targets]\nsite = \"apps/web\"\n").unwrap();
    let out = p.godo(&p.root, &["dev", "site"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("dev in web"), "{}", stdout(&out));
    let out = p.godo(&p.root, &["dev", "web"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
}

#[test]
fn malformed_godo_toml_names_the_file() {
    let p = Project::new("bad-toml");
    std::fs::write(p.root.join("godo.toml"), "[targets]\nweb = 3\n").unwrap();
    let err = stderr(&p.godo(&p.root, &["dev", "web"]));
    assert!(err.contains("godo.toml is not a godo registry"), "{err}");
}

#[test]
fn builtin_commands_win_over_directives() {
    let p = Project::new("builtin");
    let out = p.godo(&p.root, &["version"]);
    assert!(stdout(&out).starts_with("godo "), "{}", stdout(&out));
}

#[test]
fn list_shows_targets_and_makefile_rules() {
    let p = Project::new("list");
    let all = stdout(&p.godo(&p.root, &["list"]));
    assert!(all.contains("web") && all.contains("apps/web/"), "{all}");
    let rules = stdout(&p.godo(&p.root, &["list", "-n", "web"]));
    assert!(
        rules.contains("dev") && rules.contains("Start the dev server"),
        "{rules}"
    );
    assert!(
        rules.contains("fail") && !rules.contains("PHONY"),
        "{rules}"
    );
}

#[test]
fn help_explains_directives() {
    let p = Project::new("help");
    let text = stdout(&p.godo(&p.root, &["--help"]));
    assert!(text.contains("godo <directive> <target>"), "{text}");
}

#[test]
fn godo_flags_work_after_the_target() {
    let p = Project::new("flag-after");
    let out = p.godo(&p.root, &["dev", "web", "-v"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("godo: make --no-print-directory -C"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn make_flags_go_after_dashdash() {
    let p = Project::new("dashdash");
    let out = p.godo(&p.root, &["dev", "web", "-j8"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("godo dev web -- -j8"),
        "{}",
        stderr(&out)
    );
    // After --, make gets it: -s is make's "silent", ARGS reaches the recipe.
    let out = p.godo(&p.root, &["dev", "web", "--", "-s", "ARGS=after"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("args=after"), "{}", stdout(&out));
}
