use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Result;

pub const MANIFEST_FILE: &str = "library.json";

/// `<library>/library.json`: the library's identity, so it can be recognised on any
/// machine or mount path. Everything else about the library lives in `library.db`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryManifest {
    pub id: String,
    pub name: String,
    #[ts(type = "number")]
    pub created_at: i64,
}

impl LibraryManifest {
    pub fn new(name: &str) -> Self {
        Self { id: uuid::Uuid::new_v4().to_string(), name: name.to_string(), created_at: now_unix() }
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(root.join(MANIFEST_FILE)) {
            Ok(t) => Ok(Some(serde_json::from_str(&t)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        std::fs::create_dir_all(root)?;
        let path = root.join(MANIFEST_FILE);
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let t = tempfile::tempdir().unwrap();
        assert!(LibraryManifest::load(t.path()).unwrap().is_none());
        let m = LibraryManifest::new("Trip");
        m.save(t.path()).unwrap();
        let l = LibraryManifest::load(t.path()).unwrap().unwrap();
        assert_eq!(l, m);
        assert_eq!(m.id.len(), 36);
        assert_eq!(l.name, "Trip");
    }
}
