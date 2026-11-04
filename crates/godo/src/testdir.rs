// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! A fresh directory for one unit test, removed when the test ends.

use std::path::{Path, PathBuf};

pub struct TestDir(PathBuf);

impl TestDir {
    /// `<temp>/godo-test-<pid>-<name>`, empty.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("godo-test-{}-{name}", std::process::id()));
        if dir.exists() {
            std::fs::remove_dir_all(&dir).expect("clear an old test directory");
        }
        std::fs::create_dir_all(&dir).expect("create a test directory");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_directory_is_removed_afterwards() {
        let path = {
            let dir = super::TestDir::new("self-check");
            std::fs::write(dir.path().join("file"), "x").unwrap();
            dir.path().to_path_buf()
        };
        assert!(!path.exists());
    }
}
