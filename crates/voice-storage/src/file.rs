//! One JSON document on disk: tolerant load, atomic save.
use serde::{de::DeserializeOwned, Serialize};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct JsonFile {
    path: PathBuf,
}

impl JsonFile {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `None` when the file is missing, unreadable or corrupt. A corrupt file is
    /// copied to `<name>.bak` so nothing the user had is silently lost.
    pub(crate) fn load<T: DeserializeOwned>(&self) -> Option<T> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("cannot read {}: {e}", self.path.display());
                return None;
            }
        };
        match serde_json::from_str(&text) {
            Ok(value) => Some(value),
            Err(e) => {
                log::warn!("{} is corrupt ({e}); using defaults", self.path.display());
                self.keep_backup();
                None
            }
        }
    }

    pub(crate) fn save<T: Serialize>(&self, value: &T) -> Result<(), String> {
        let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let tmp = sibling(&self.path, "tmp");
        fs::write(&tmp, json).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        fs::rename(&tmp, &self.path)
            .map_err(|e| format!("cannot replace {}: {e}", self.path.display()))
    }

    fn keep_backup(&self) {
        let bak = sibling(&self.path, "bak");
        if let Err(e) = fs::copy(&self.path, &bak) {
            log::warn!("cannot back up corrupt file to {}: {e}", bak.display());
        }
    }
}

/// `settings.json` → `settings.json.<ext>`
fn sibling(path: &Path, ext: &str) -> PathBuf {
    let mut name: OsString = path.file_name().unwrap_or_default().to_owned();
    name.push(".");
    name.push(ext);
    path.with_file_name(name)
}
