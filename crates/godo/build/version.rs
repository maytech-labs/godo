// SPDX-License-Identifier: LicenseRef-REAL-1.0

// Shared by build.rs and the tests in src/main.rs.

/// `godo-v0.1.1-3-gabc1234[-dirty]` → `3.gabc1234[.dirty]`, or empty for
/// the release itself (`godo-v<version>-0-g...`, not dirty).
fn parse(described: &str, version: &str) -> Option<String> {
    let rest = described.trim().strip_prefix("godo-v")?;
    let (rest, dirty) = match rest.strip_suffix("-dirty") {
        Some(r) => (r, true),
        None => (rest, false),
    };
    let mut parts = rest.rsplitn(3, '-');
    let sha = parts.next()?;
    let commits = parts.next()?;
    let tag = parts.next()?;
    if tag == version && commits == "0" && !dirty {
        return Some(String::new());
    }
    let mut suffix = format!("{commits}.{sha}");
    if dirty {
        suffix.push_str(".dirty");
    }
    Some(suffix)
}
