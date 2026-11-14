use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::library::Library;
use crate::Result;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryEntry {
    /// The manifest id (`library.json`), stable across machines and mount paths.
    pub id: String,
    pub name: String,
    /// Last known path on this machine.
    pub path: String,
    #[ts(type = "number")]
    pub last_opened_at: i64,
}

/// `libraries.json`: every library this install has opened, and which one is current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryRegistry {
    pub version: u32,
    pub libraries: Vec<LibraryEntry>,
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryStatus {
    pub entry: LibraryEntry,
    /// `library.db` exists at `entry.path` right now (the disk is connected).
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryListing {
    pub libraries: Vec<LibraryStatus>,
    pub current: Option<String>,
}

impl Default for LibraryRegistry {
    fn default() -> Self {
        Self { version: 2, libraries: Vec::new(), current: None }
    }
}

impl LibraryRegistry {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                let mut r: Self = serde_json::from_str(&t)?;
                r.version = 2;
                Ok(r)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Add or refresh the entry for `id`, make it current, and drop any other entry that
    /// claims the same path (a Phase 0 hash-keyed entry, or a library re-created in place).
    pub fn upsert(&mut self, id: &str, name: &str, path: &Path, now: i64) -> &LibraryEntry {
        let path_str = path.to_string_lossy().into_owned();
        self.libraries.retain(|e| e.id == id || e.path != path_str);
        let idx = match self.libraries.iter().position(|e| e.id == id) {
            Some(i) => {
                let e = &mut self.libraries[i];
                e.name = name.to_string();
                e.path = path_str;
                e.last_opened_at = now;
                i
            }
            None => {
                self.libraries.push(LibraryEntry {
                    id: id.to_string(),
                    name: name.to_string(),
                    path: path_str,
                    last_opened_at: now,
                });
                self.libraries.len() - 1
            }
        };
        self.current = Some(id.to_string());
        &self.libraries[idx]
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.libraries.len();
        self.libraries.retain(|e| e.id != id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
        self.libraries.len() != before
    }

    pub fn find_by_path(&self, path: &Path) -> Option<&LibraryEntry> {
        let p = path.to_string_lossy();
        self.libraries.iter().find(|e| e.path == p)
    }

    pub fn current_entry(&self) -> Option<&LibraryEntry> {
        let id = self.current.as_ref()?;
        self.libraries.iter().find(|e| &e.id == id)
    }

    pub fn listing(&self) -> LibraryListing {
        let mut libraries: Vec<LibraryStatus> = self
            .libraries
            .iter()
            .map(|e| LibraryStatus { available: Library::exists_at(Path::new(&e.path)), entry: e.clone() })
            .collect();
        libraries.sort_by_key(|s| std::cmp::Reverse(s.entry.last_opened_at));
        LibraryListing { libraries, current: self.current.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_by_id_replaces_same_path_entries() {
        let mut r = LibraryRegistry::default();
        r.upsert("old-hash", "A", Path::new("/l/a"), 1);
        r.upsert("uuid-a", "A", Path::new("/l/a"), 2);
        assert_eq!(r.libraries.len(), 1);
        assert_eq!(r.current.as_deref(), Some("uuid-a"));
        r.upsert("uuid-a", "A", Path::new("/mnt/disk/a"), 3);
        assert_eq!(r.libraries[0].path, "/mnt/disk/a");
        assert!(r.find_by_path(Path::new("/mnt/disk/a")).is_some());
        assert!(r.remove("uuid-a"));
        assert!(r.current.is_none());
        assert!(!r.remove("uuid-a"));
    }

    #[test]
    fn listing_marks_unavailable_paths() {
        let t = tempfile::tempdir().unwrap();
        let mut r = LibraryRegistry::default();
        r.upsert("x", "X", &t.path().join("nope"), 1);
        let l = r.listing();
        assert!(!l.libraries[0].available);
        assert_eq!(l.current.as_deref(), Some("x"));
    }

    #[test]
    fn load_missing_is_empty_and_save_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("libraries.json");
        let mut r = LibraryRegistry::load(&p).unwrap();
        assert!(r.libraries.is_empty() && r.current.is_none());
        r.upsert("id", "X", Path::new("/x"), 1);
        r.save(&p).unwrap();
        let r2 = LibraryRegistry::load(&p).unwrap();
        assert_eq!(r, r2);
        assert_eq!(r2.version, 2);
    }
}
