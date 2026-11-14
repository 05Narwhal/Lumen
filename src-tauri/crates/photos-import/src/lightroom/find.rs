//! Finding Lightroom Classic catalogs on this computer, for the Import screen's card.

use std::path::{Path, PathBuf};

use serde::Serialize;
use ts_rs::TS;

/// A catalog found by itself: the Import screen offers Lightroom only when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogRef {
    pub path: String,
    /// The file's name without `.lrcat`.
    pub name: String,
    /// Photos and videos in it (virtual copies left out); `None` when it could not be read.
    pub photo_count: Option<u32>,
    /// When it last changed, unix seconds.
    #[ts(type = "number | null")]
    pub modified: Option<i64>,
}

/// The `.lrcat` files directly in `roots` and one folder below them, plus `recent` (from
/// Lightroom's preferences) that exist.
pub fn find_lightroom_catalogs(roots: &[PathBuf], recent: &[PathBuf]) -> Vec<CatalogRef> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for root in roots {
        for dir in std::iter::once(root.clone()).chain(subfolders(root)) {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            paths.extend(entries.flatten().map(|e| e.path()).filter(|p| is_catalog(p)));
        }
    }
    paths.extend(recent.iter().filter(|p| is_catalog(p)).cloned());
    let mut seen = std::collections::HashSet::new();
    let mut found: Vec<CatalogRef> = paths
        .into_iter()
        .filter(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())))
        .map(|p| {
            let meta = std::fs::metadata(&p).ok();
            let modified = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64);
            CatalogRef {
                name: p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
                photo_count: photo_count(&p, modified, meta.map_or(0, |m| m.len())),
                path: p.to_string_lossy().into_owned(),
                modified,
            }
        })
        .collect();
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.path.cmp(&b.path)));
    found
}

fn is_catalog(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrcat"))
}

/// The folders directly in `root`, but not Lightroom's previews and helper folders.
fn subfolders(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .filter(|p| !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrdata")))
        .collect()
}

/// How many photos the catalog holds, read from a temp copy once per version of the file.
fn photo_count(path: &Path, modified: Option<i64>, len: u64) -> Option<u32> {
    use std::collections::HashMap;
    use std::sync::Mutex;
    type Key = (PathBuf, Option<i64>, u64);
    static COUNTS: Mutex<Option<HashMap<Key, Option<u32>>>> = Mutex::new(None);
    let key = (path.to_path_buf(), modified, len);
    if let Some(n) = COUNTS.lock().ok().and_then(|c| c.as_ref().and_then(|c| c.get(&key).copied())) {
        return n;
    }
    let count = (|| {
        let tmp = tempfile::tempdir().ok()?;
        let copy = super::catalog::copy_catalog(path, tmp.path()).ok()?;
        let conn = rusqlite::Connection::open(&copy).ok()?;
        conn.query_row("SELECT COUNT(*) FROM Adobe_images WHERE masterImage IS NULL", [], |r| r.get::<_, i64>(0)).ok().map(|n| n as u32)
    })();
    if let Ok(mut c) = COUNTS.lock() {
        c.get_or_insert_with(HashMap::new).insert(key, count);
    }
    count
}

/// The catalogs Lightroom's preferences name, best effort: every path ending `.lrcat`, in
/// the order first named. Windows' `.agprefs` is Lua text with escaped strings; macOS' plist
/// holds the paths as plain bytes.
pub fn recent_in_preferences(text: &str) -> Vec<PathBuf> {
    // Undo the escaping (`\\\\` inside a quoted Lua string inside a quoted string).
    let mut t = text.to_string();
    for _ in 0..3 {
        t = t.replace("\\\\", "\\").replace("\\\"", "\"");
    }
    let mut out: Vec<PathBuf> = Vec::new();
    let mut rest = t.as_str();
    while let Some(end) = rest.find(".lrcat") {
        let before = &rest[..end];
        // Back to the start of the path: a quote, a control character or a separator.
        let start = before.rfind(|c: char| c == '"' || c == '\'' || c.is_control()).map_or(0, |i| i + 1);
        let candidate = before[start..].trim();
        // A path starts at a drive (`C:`) or the root (`/`).
        let from = candidate.char_indices().find(|&(i, c)| c == '/' || (c.is_ascii_alphabetic() && candidate[i + c.len_utf8()..].starts_with(":\\"))).map(|(i, _)| i);
        if let Some(i) = from {
            let p = PathBuf::from(format!("{}.lrcat", &candidate[i..]));
            if !out.contains(&p) {
                out.push(p);
            }
        }
        rest = &rest[end + ".lrcat".len()..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_catalogs_one_level_down_and_skips_lightrooms_own_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let lr = root.join("Lightroom");
        std::fs::create_dir_all(lr.join("Lightroom Catalog Previews.lrdata")).unwrap();
        std::fs::create_dir_all(lr.join("Lightroom Catalog Helper.lrdata")).unwrap();
        std::fs::create_dir_all(root.join("deep").join("deeper")).unwrap();
        crate::lightroom::catalog::tests::sample(&lr);
        std::fs::write(lr.join("Lightroom Catalog Previews.lrdata").join("x.lrcat"), b"").unwrap();
        std::fs::write(root.join("deep").join("deeper").join("too deep.lrcat"), b"").unwrap();
        std::fs::write(root.join("Old.LRCAT"), b"not a database").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let recent = elsewhere.path().join("Travel.lrcat");
        crate::lightroom::catalog::tests::create_catalog(&recent);

        let found = find_lightroom_catalogs(&[root.to_path_buf(), lr.clone()], &[recent.clone(), elsewhere.path().join("gone.lrcat")]);
        let names: Vec<(&str, Option<u32>)> = found.iter().map(|c| (c.name.as_str(), c.photo_count)).collect();
        assert_eq!(names, vec![("Lightroom Catalog", Some(2)), ("Old", None), ("Travel", Some(0))], "{found:?}");
        assert!(found[0].modified.is_some());
    }

    #[test]
    fn reads_recent_catalogs_from_lightrooms_preferences() {
        let agprefs = r#"recentLibraries20 = "{\n\t\"C:\\\\Users\\\\Ana\\\\Pictures\\\\Lightroom\\\\Lightroom Catalog.lrcat\",\n\t\"D:\\\\Work\\\\Weddings.lrcat\",\n}\n",
libraryToLoad20 = "C:\\Users\\Ana\\Pictures\\Lightroom\\Lightroom Catalog.lrcat","#;
        let found = recent_in_preferences(agprefs);
        assert_eq!(found, vec![PathBuf::from(r"C:\Users\Ana\Pictures\Lightroom\Lightroom Catalog.lrcat"), PathBuf::from(r"D:\Work\Weddings.lrcat")]);
        let plist = "\u{0}\u{8}_\u{10}/Users/ana/Pictures/Lightroom/Lightroom Catalog.lrcat\u{0}";
        assert_eq!(recent_in_preferences(plist), vec![PathBuf::from("/Users/ana/Pictures/Lightroom/Lightroom Catalog.lrcat")]);
        assert!(recent_in_preferences("nothing here").is_empty());
    }
}
