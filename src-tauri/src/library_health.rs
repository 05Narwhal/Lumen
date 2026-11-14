//! What the library knows about its own files without a source (Round 22 §5): the health card's
//! numbers, the causes of the files that do not open, and the small file that keeps the last
//! *Check the library* beside `library.db`. Reads only; nothing here changes a photo.

use std::path::{Path, PathBuf};

use photos_core::unreadable::UnreadableFile;
use photos_core::Library;
use photos_media::failure::{cause, UnreadableCause};
use photos_media::DerivativeStore;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const HEALTH_FILE: &str = "health.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CauseCount {
    pub cause: UnreadableCause,
    #[ts(type = "number")]
    pub n: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryHealth {
    /// Photos and videos not in the Trash.
    #[ts(type = "number")]
    pub total: i64,
    /// Flagged as not openable, not in the Trash.
    #[ts(type = "number")]
    pub unreadable: i64,
    /// Files the last *Check the library* found missing or changed.
    #[ts(type = "number")]
    pub missing: i64,
    /// Photos whose damaged file was replaced by what could be read of it.
    #[ts(type = "number")]
    pub recovered: i64,
    /// When the last *Check the library* ran to the end (unix seconds).
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
    pub causes: Vec<CauseCount>,
}

/// One row of the list of files that do not open: the file and why.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnreadableRow {
    #[serde(flatten)]
    #[ts(flatten)]
    pub file: UnreadableFile,
    pub cause: UnreadableCause,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnreadableRowsPage {
    pub items: Vec<UnreadableRow>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

/// The last full check, as kept on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthFile {
    pub checked_at: i64,
    pub missing: i64,
}

fn health_path(root: &Path) -> PathBuf {
    root.join(HEALTH_FILE)
}

/// Keeps the result of a finished (not cancelled) library check beside `library.db`.
pub fn write_health_file(root: &Path, file: HealthFile) {
    let path = health_path(root);
    let Ok(text) = serde_json::to_string(&file) else { return };
    let tmp = path.with_extension("json.part");
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// `None`: never checked, or a file that cannot be read.
pub fn read_health_file(root: &Path) -> Option<HealthFile> {
    serde_json::from_str(&std::fs::read_to_string(health_path(root)).ok()?).ok()
}

/// The cause the reader recorded for a file with this hash; `Unknown` when it recorded nothing.
pub fn cause_of(store: &DerivativeStore, hash: &str) -> UnreadableCause {
    store.failure_text(hash).map_or(UnreadableCause::Unknown, |t| cause(&t))
}

pub fn health(lib: &Library) -> Result<LibraryHealth, photos_core::CoreError> {
    let (total, unreadable, recovered, hashes) = lib.db.with_conn(|c| {
        let n = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0));
        let total = n("SELECT COUNT(*) FROM assets WHERE state = 0")?;
        let unreadable = n("SELECT COUNT(*) FROM assets WHERE unreadable = 1 AND state = 0")?;
        let recovered = n("SELECT COUNT(*) FROM assets WHERE recovered_pct IS NOT NULL AND state = 0")?;
        let mut stmt = c.prepare("SELECT content_hash FROM assets WHERE unreadable = 1 AND state = 0")?;
        let hashes = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok((total, unreadable, recovered, hashes))
    })?;
    let store = DerivativeStore::new(&lib.root);
    let mut counts = [0i64; 4];
    for h in &hashes {
        counts[match cause_of(&store, h) {
            UnreadableCause::Incomplete => 0,
            UnreadableCause::NotAnImage => 1,
            UnreadableCause::TooLarge => 2,
            UnreadableCause::Unknown => 3,
        }] += 1;
    }
    let causes = [UnreadableCause::Incomplete, UnreadableCause::NotAnImage, UnreadableCause::TooLarge, UnreadableCause::Unknown]
        .into_iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(cause, n)| CauseCount { cause, n })
        .collect();
    let checked = read_health_file(&lib.root);
    Ok(LibraryHealth { total, unreadable, missing: checked.map_or(0, |c| c.missing), recovered, checked_at: checked.map(|c| c.checked_at), causes })
}

/// A page of `unreadable::list_in`, each row with its cause.
pub fn unreadable_page(lib: &Library, trashed: bool, after: Option<i64>, limit: usize) -> Result<UnreadableRowsPage, photos_core::CoreError> {
    let page = lib.db.with_conn(|c| photos_core::unreadable::list_in(c, trashed, after, limit))?;
    let store = DerivativeStore::new(&lib.root);
    let items = page
        .items
        .into_iter()
        .map(|file| {
            let cause = cause_of(&store, &file.content_hash);
            UnreadableRow { file, cause }
        })
        .collect();
    Ok(UnreadableRowsPage { items, next_cursor: page.next_cursor })
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::assets::{insert_assets, NewAsset};

    fn photo(hash: &str) -> NewAsset {
        NewAsset {
            content_hash: hash.into(),
            relative_path: format!("x/{hash}.jpg"),
            filename: format!("{hash}.jpg"),
            extension: "jpg".into(),
            media_type: 0,
            byte_size: 1,
            captured_at: Some(100),
            imported_at: 1,
            ..Default::default()
        }
    }

    /// A library with one ordinary photo and one flagged photo per message.
    fn library(messages: &[Option<&str>]) -> (tempfile::TempDir, Library) {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::create(&tmp.path().join("lib")).unwrap();
        let store = DerivativeStore::new(&lib.root);
        let mut all = vec![photo("aa00")];
        for i in 0..messages.len() {
            all.push(photo(&format!("bb{i:02}")));
        }
        let ids = lib.db.with_conn(|c| insert_assets(c, &all)).unwrap();
        for (i, m) in messages.iter().enumerate() {
            lib.db.with_conn(|c| photos_core::unreadable::set_unreadable(c, ids[i + 1], true)).unwrap();
            if let Some(m) = m {
                store.record_failure(&format!("bb{i:02}"), "26.1.0-alpha", m);
            }
        }
        (tmp, lib)
    }

    #[test]
    fn health_counts_one_flagged_asset_per_cause() {
        let (_tmp, lib) = library(&[Some("moov atom not found"), Some("Invalid data found when processing input"), Some("Memory limit exceeded"), None]);
        let h = health(&lib).unwrap();
        assert_eq!((h.total, h.unreadable, h.recovered, h.missing, h.checked_at), (5, 4, 0, 0, None));
        let by = |c: UnreadableCause| h.causes.iter().find(|x| x.cause == c).map(|x| x.n);
        assert_eq!(by(UnreadableCause::Incomplete), Some(1));
        assert_eq!(by(UnreadableCause::NotAnImage), Some(1));
        assert_eq!(by(UnreadableCause::TooLarge), Some(1));
        assert_eq!(by(UnreadableCause::Unknown), Some(1), "no recorded reason is unknown");
    }

    #[test]
    fn a_trashed_flagged_file_is_not_counted_and_a_recovered_one_is() {
        let (_tmp, lib) = library(&[Some("partial file"), Some("partial file")]);
        lib.db
            .with_conn(|c| {
                c.execute("UPDATE assets SET state = 1 WHERE content_hash = 'bb00'", [])?;
                c.execute("UPDATE assets SET recovered_pct = 62 WHERE content_hash = 'bb01'", [])?;
                Ok(())
            })
            .unwrap();
        let h = health(&lib).unwrap();
        assert_eq!((h.total, h.unreadable, h.recovered), (2, 1, 1));
    }

    #[test]
    fn health_json_round_trips_and_a_missing_file_reads_as_never_checked() {
        let (_tmp, lib) = library(&[]);
        assert_eq!(read_health_file(&lib.root), None);
        assert_eq!(health(&lib).unwrap().checked_at, None);
        write_health_file(&lib.root, HealthFile { checked_at: 1_700_000_000, missing: 3 });
        assert_eq!(read_health_file(&lib.root), Some(HealthFile { checked_at: 1_700_000_000, missing: 3 }));
        let h = health(&lib).unwrap();
        assert_eq!((h.checked_at, h.missing), (Some(1_700_000_000), 3));
        std::fs::write(lib.root.join("health.json"), "not json").unwrap();
        assert_eq!(read_health_file(&lib.root), None, "a broken file reads as never checked");
    }

    #[test]
    fn the_list_rows_carry_their_cause() {
        let (_tmp, lib) = library(&[Some("unexpected end of file"), None]);
        let page = unreadable_page(&lib, false, None, 10).unwrap();
        assert_eq!(page.items.iter().map(|r| r.cause).collect::<Vec<_>>(), vec![UnreadableCause::Incomplete, UnreadableCause::Unknown]);
        let json = serde_json::to_value(&page.items[0]).unwrap();
        assert!(json.get("filename").is_some() && json["cause"] == "incomplete", "flattened: {json}");
    }
}
