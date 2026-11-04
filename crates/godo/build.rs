// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Sets `GODO_VERSION`: the crate version for a release build, and
//! `<version>+<commits>.g<sha>[.dirty]` for anything else built from git, so
//! a development build never claims to be the release it came after.
//!
//! A build is a release when HEAD is the `godo-v<version>` tag. Without git
//! (a crates.io or archive build) it's the crate version as it is.

use std::process::Command;

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets this");
    let described = git(&[
        "describe", "--tags", "--match", "godo-v*", "--dirty", "--long",
    ]);
    let full = match described.as_deref().and_then(|d| parse(d, &version)) {
        Some(suffix) if suffix.is_empty() => version,
        Some(suffix) => format!("{version}+{suffix}"),
        None => match git(&["rev-parse", "--short", "HEAD"]) {
            // No release tag in reach: name the commit.
            Some(sha) => format!("{version}+g{}", sha.trim()),
            None => version,
        },
    };
    println!("cargo:rustc-env=GODO_VERSION={full}");
    // Rebuild when HEAD moves or tags change; ignore failures outside git.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]) {
        let dir = dir.trim();
        println!("cargo:rerun-if-changed={dir}/HEAD");
        println!("cargo:rerun-if-changed={dir}/refs/tags");
        println!("cargo:rerun-if-changed={dir}/index");
    }
}

include!("build/version.rs");

/// Runs git in this crate's directory, but only if that's godo's own
/// repository, not some other one that happens to contain the build.
fn git(args: &[&str]) -> Option<String> {
    let dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let top = run(&dir, &["rev-parse", "--show-toplevel"])?;
    let ours = std::path::Path::new(top.trim()).join("crates/godo");
    if ours.canonicalize().ok()? != dir.canonicalize().ok()? {
        return None;
    }
    run(&dir, args)
}

fn run(dir: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
