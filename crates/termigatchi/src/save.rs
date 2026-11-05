// SPDX-License-Identifier: LicenseRef-REAL-1.0

//! Where the pet lives between visits.

use std::fs;
use std::io;
use std::path::PathBuf;

use crate::pet::Pet;

pub struct Store {
    path: PathBuf,
}

impl Store {
    /// `$TERMIGATCHI_HOME/pet.json`, or the platform's data directory.
    pub fn locate() -> io::Result<Store> {
        let dir = if let Some(d) = std::env::var_os("TERMIGATCHI_HOME") {
            PathBuf::from(d)
        } else {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| io::Error::other("HOME isn't set"))?;
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/termigatchi")
            } else if let Some(x) = std::env::var_os("XDG_DATA_HOME").filter(|x| !x.is_empty()) {
                PathBuf::from(x).join("termigatchi")
            } else {
                home.join(".local/share/termigatchi")
            }
        };
        Ok(Store {
            path: dir.join("pet.json"),
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Option<Pet>> {
        match fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s)
                .map(Some)
                .map_err(|e| io::Error::other(format!("{} is damaged: {e}", self.path.display()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Writes atomically, so a crash never leaves half a pet.
    pub fn save(&self, pet: &Pet) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(
            &tmp,
            serde_json::to_string_pretty(pet).map_err(io::Error::other)?,
        )?;
        fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
impl Store {
    pub fn at(path: PathBuf) -> Store {
        Store { path }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let dir = std::env::temp_dir().join(format!("termigatchi-test-{}", std::process::id()));
        let store = Store::at(dir.join("pet.json"));
        assert!(store.load().unwrap().is_none());
        let pet = Pet::new("Bean", 42);
        store.save(&pet).unwrap();
        assert_eq!(store.load().unwrap(), Some(pet));
        fs::write(store.path(), "nope").unwrap();
        assert!(store.load().is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
