use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Result;

pub const BATCH_RUNNING: i64 = 0;
pub const BATCH_COMPLETE: i64 = 1;
pub const BATCH_FAILED: i64 = 2;
pub const BATCH_CANCELLED: i64 = 3;
/// Stopped part-way — the device was unplugged, the disk was nearly full, or the app closed —
/// with everything brought in so far kept. Importing again picks up the rest (round 3).
pub const BATCH_INTERRUPTED: i64 = 4;

pub const SOURCE_FOLDER: i64 = 0;
pub const SOURCE_VOLUME: i64 = 1;
pub const SOURCE_IPHONE: i64 = 2;
pub const SOURCE_MACDISK: i64 = 3;
pub const SOURCE_PHOTOS_LIBRARY: i64 = 4;
/// A Lightroom Classic catalog (Round 11).
pub const SOURCE_LIGHTROOM: i64 = 5;
/// An importer plugin (Round 23); the batch label is the importer's `label`.
pub const SOURCE_PLUGIN: i64 = 6;

pub fn start_batch(conn: &Connection, source_kind: i64, label: &str, source_path: Option<&str>, now: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO import_batches (source_kind, source_label, source_path, started_at, state) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![source_kind, label, source_path, now, BATCH_RUNNING],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn finish_batch(conn: &Connection, id: i64, state: i64, item_count: i64, skipped_count: i64, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE import_batches SET state = ?2, item_count = ?3, skipped_count = ?4, finished_at = ?5 WHERE id = ?1",
        params![id, state, item_count, skipped_count, now],
    )?;
    Ok(())
}

pub fn record_failure(conn: &Connection, batch_id: i64, path: &str, reason: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO import_failures (batch_id, path, reason) VALUES (?1, ?2, ?3)",
        params![batch_id, path, reason],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportBatchSummary {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number")]
    pub source_kind: i64,
    pub source_label: String,
    pub source_path: Option<String>,
    #[ts(type = "number")]
    pub started_at: i64,
    #[ts(type = "number | null")]
    pub finished_at: Option<i64>,
    #[ts(type = "number")]
    pub item_count: i64,
    #[ts(type = "number")]
    pub skipped_count: i64,
    #[ts(type = "number")]
    pub failed_count: i64,
    #[ts(type = "number")]
    pub state: i64,
    /// Files listed as not imported (`import_skips`): duplicates, foreign types, unreadable.
    #[ts(type = "number")]
    pub skip_count: i64,
}

pub fn list_batches(conn: &Connection, limit: i64) -> Result<Vec<ImportBatchSummary>> {
    let mut stmt = conn.prepare(
        "SELECT b.id, b.source_kind, b.source_label, b.source_path, b.started_at, b.finished_at, b.item_count,
                b.skipped_count, (SELECT COUNT(*) FROM import_failures f WHERE f.batch_id = b.id), b.state,
                (SELECT COUNT(*) FROM import_skips s WHERE s.batch_id = b.id)
         FROM import_batches b
         -- An import stays in the history while any photo it brought is in the library, and
         -- returns when one is restored. Imports that brought nothing new, and ones still
         -- running, always show: they are about the attempt, not the photos.
         WHERE b.state = 0 OR b.item_count = 0
            OR EXISTS (SELECT 1 FROM assets a WHERE a.import_batch_id = b.id AND a.state = 0 AND a.concealed = 0)
         ORDER BY b.started_at DESC, b.id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok(ImportBatchSummary {
            id: r.get(0)?,
            source_kind: r.get(1)?,
            source_label: r.get(2)?,
            source_path: r.get(3)?,
            started_at: r.get(4)?,
            finished_at: r.get(5)?,
            item_count: r.get(6)?,
            skipped_count: r.get(7)?,
            failed_count: r.get(8)?,
            state: r.get(9)?,
            skip_count: r.get(10)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportFailure {
    pub path: String,
    pub reason: String,
}

pub fn list_failures(conn: &Connection, batch_id: i64) -> Result<Vec<ImportFailure>> {
    let mut stmt = conn.prepare("SELECT path, reason FROM import_failures WHERE batch_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([batch_id], |r| Ok(ImportFailure { path: r.get(0)?, reason: r.get(1)? }))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Newest assets of a batch, for the Home strip.
pub fn recent_asset_ids(conn: &Connection, batch_id: i64, limit: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM assets WHERE import_batch_id = ?1 AND state = 0 AND concealed = 0 ORDER BY {} LIMIT ?2",
        crate::assets::TIMELINE_ORDER
    ))?;
    let rows = stmt.query_map(params![batch_id, limit], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    #[test]
    fn source_plugin_is_six_and_unique() {
        let all = [SOURCE_FOLDER, SOURCE_VOLUME, SOURCE_IPHONE, SOURCE_MACDISK, SOURCE_PHOTOS_LIBRARY, SOURCE_LIGHTROOM, SOURCE_PLUGIN];
        let mut s = all.to_vec();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), all.len());
        assert_eq!(SOURCE_PLUGIN, 6);
    }

    use super::*;
    use crate::Db;

    fn add_asset(c: &Connection, batch: i64, name: &str) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, import_batch_id)
             VALUES (?1, ?1, ?1, 'jpg', 0, 1, 1, 1, ?2)",
            params![name, batch],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    /// Import history is about photos in the library. A folder whose every photo has gone
    /// to the Trash (or been deleted) leaves the history, and comes back when any of them is
    /// restored. Imports that brought nothing new, and imports still running, always show.
    #[test]
    fn history_lists_only_imports_with_something_left_in_the_library() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let montenegro = start_batch(c, SOURCE_FOLDER, "Montenegro", Some("D:/Montenegro"), 10)?;
            let a = add_asset(c, montenegro, "a");
            let b = add_asset(c, montenegro, "b");
            finish_batch(c, montenegro, BATCH_COMPLETE, 2, 0, 11)?;
            let dupes = start_batch(c, SOURCE_FOLDER, "Again", Some("D:/Montenegro"), 20)?;
            finish_batch(c, dupes, BATCH_COMPLETE, 0, 2, 21)?;
            let running = start_batch(c, SOURCE_VOLUME, "EOS_DIGITAL", None, 30)?;
            let ids = |c: &Connection| list_batches(c, 10).unwrap().into_iter().map(|b| b.id).collect::<Vec<_>>();

            assert_eq!(ids(c), vec![running, dupes, montenegro]);
            crate::assets::trash_assets(c, &[a], 40)?;
            assert!(ids(c).contains(&montenegro), "one photo is still in the library");
            crate::assets::trash_assets(c, &[b], 41)?;
            assert_eq!(ids(c), vec![running, dupes], "every photo is in the Trash");
            crate::assets::restore_assets(c, &[b])?;
            assert!(ids(c).contains(&montenegro), "restoring one brings the import back");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn batch_lifecycle_with_failures() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = start_batch(c, SOURCE_FOLDER, "Screenshots", Some("C:/Pics"), 10)?;
            for n in 0..5 {
                add_asset(c, id, &format!("shot{n}"));
            }
            record_failure(c, id, "C:/Pics/bad.jpg", "not a file")?;
            record_failure(c, id, "C:/Pics/worse.jpg", "decode failed")?;
            finish_batch(c, id, BATCH_COMPLETE, 5, 2, 20)?;
            let list = list_batches(c, 10)?;
            assert_eq!(list.len(), 1);
            let b = &list[0];
            assert_eq!((b.item_count, b.skipped_count, b.failed_count, b.state), (5, 2, 2, BATCH_COMPLETE));
            assert_eq!(b.source_path.as_deref(), Some("C:/Pics"));
            assert_eq!(b.finished_at, Some(20));
            let f = list_failures(c, id)?;
            assert_eq!(f.len(), 2);
            assert_eq!(f[1].reason, "decode failed");
            assert_eq!(recent_asset_ids(c, id, 5)?.len(), 5);
            Ok(())
        })
        .unwrap();
    }
}
