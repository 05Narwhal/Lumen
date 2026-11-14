//! The last Develop filmstrip, remembered across reloads, restarts and relaunches (Round 21 §13):
//! what the strip showed (a scope, or the photos Develop was opened on) and the photo last open.
//! Kept in `develop-source.json` in app data, one entry per library — never in the database.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::develop_strip::StripSort;
use crate::scope::LayoutScope;
use crate::Result;

/// What Develop's filmstrip shows, as the page holds it (`DevelopSource` in `develop-source.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RememberedStrip {
    pub scope: LayoutScope,
    /// The photos Develop was opened on, in order; `None` for the whole scope.
    #[ts(type = "Array<number> | null")]
    pub only: Option<Vec<i64>>,
    /// Named in the top bar and on Home: "Selection", an album's name.
    pub label: String,
    pub sort: StripSort,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DevelopSourceFile {
    pub source: Option<RememberedStrip>,
    /// The photo last open in Develop.
    #[ts(type = "number | null")]
    pub last: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct AllLibraries {
    /// library id → its remembered strip.
    #[serde(default)]
    libraries: HashMap<String, DevelopSourceFile>,
}

/// The library's remembered strip, `None` when nothing is stored or the file can't be read.
pub fn read(path: &Path, library_id: &str) -> Option<DevelopSourceFile> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<AllLibraries>(&text).ok()?.libraries.remove(library_id)
}

/// Store the library's strip, keeping every other library's; written whole then renamed, so a
/// crash never leaves half a file.
pub fn write(path: &Path, library_id: &str, file: &DevelopSourceFile) -> std::io::Result<()> {
    let mut all = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<AllLibraries>(&t).ok()).unwrap_or_default();
    all.libraries.insert(library_id.to_string(), file.clone());
    let text = serde_json::to_string_pretty(&all).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// Drop the photos that are no longer in the library (deleted for good, or in the trash) from
/// the strip and from `last`.
pub fn prune(conn: &Connection, file: &mut DevelopSourceFile) -> Result<()> {
    let mut asked: Vec<i64> = file.source.as_ref().and_then(|s| s.only.clone()).unwrap_or_default();
    asked.extend(file.last);
    if asked.is_empty() {
        return Ok(());
    }
    let list = asked.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!("SELECT id FROM assets WHERE state = 0 AND id IN ({list})"))?;
    let alive: std::collections::HashSet<i64> = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    if let Some(only) = file.source.as_mut().and_then(|s| s.only.as_mut()) {
        only.retain(|i| alive.contains(i));
    }
    file.last = file.last.filter(|i| alive.contains(i));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(only: Option<Vec<i64>>) -> RememberedStrip {
        RememberedStrip { scope: LayoutScope::library(), only, label: "Selection".into(), sort: StripSort::Rating }
    }

    #[test]
    fn a_strip_survives_a_write_and_a_read_beside_another_librarys() {
        let dir = std::env::temp_dir().join(format!("lumen-dsrc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("develop-source.json");
        let _ = std::fs::remove_file(&path);
        assert_eq!(read(&path, "a"), None, "no file yet");

        let mine = DevelopSourceFile { source: Some(strip(Some(vec![3, 1, 2]))), last: Some(1) };
        let other = DevelopSourceFile { source: Some(strip(None)), last: None };
        write(&path, "a", &mine).unwrap();
        write(&path, "b", &other).unwrap();
        assert_eq!(read(&path, "a"), Some(mine));
        assert_eq!(read(&path, "b"), Some(other));
        assert_eq!(read(&path, "c"), None);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(read(&path, "a"), None, "a damaged file is nothing remembered, not an error");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn photos_gone_from_the_library_are_dropped_in_order() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, state)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 1, 0),
                          (2, 'h2', 'p', 'b.jpg', 'jpg', 0, 1, 1, 1, 0),
                          (3, 'h3', 'p', 'c.jpg', 'jpg', 0, 1, 1, 1, 1);",
            )?;
            // 3 is in the trash, 4 never existed (or was deleted for good).
            let mut file = DevelopSourceFile { source: Some(strip(Some(vec![2, 3, 4, 1]))), last: Some(4) };
            prune(c, &mut file)?;
            assert_eq!(file.source.unwrap().only, Some(vec![2, 1]));
            assert_eq!(file.last, None);

            let mut scoped = DevelopSourceFile { source: Some(strip(None)), last: Some(2) };
            prune(c, &mut scoped)?;
            assert_eq!(scoped.source.unwrap().only, None, "a scope has no ids to drop");
            assert_eq!(scoped.last, Some(2));
            Ok(())
        })
        .unwrap();
    }
}
