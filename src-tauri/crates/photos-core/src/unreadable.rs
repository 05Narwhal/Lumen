//! Files Lumen can't open (Round 18 C1). An original every reader has been tried on and could
//! not decode is `assets.unreadable = 1` (migration 0033), which also sets `concealed`, so
//! every grid, count, summary, search and memory leaves it out. Nothing is deleted: the file
//! stays in `originals/`, the row stays, and Settings lists it with *Try again*.
//!
//! The flag is derived data. The marks under `derivatives/unreadable/` are the evidence, the
//! flag is their reflection in SQL, and either can be rebuilt from the other.

use rusqlite::{params, Connection};
use serde::Serialize;
use ts_rs::TS;

use crate::Result;

/// One row of *Files Lumen can't open*.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnreadableFile {
    #[ts(type = "number")]
    pub id: i64,
    pub filename: String,
    /// Path inside the library's `originals/`, for *Show in folder*.
    pub relative_path: String,
    pub extension: String,
    #[ts(type = "number")]
    pub byte_size: i64,
    pub content_hash: String,
}

/// A folder some flagged files were imported from, with how many of them it holds.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceFolder {
    pub path: String,
    #[ts(type = "number")]
    pub files: i64,
}

/// A page of the list, by keyset on the asset id.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UnreadablePage {
    pub items: Vec<UnreadableFile>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

/// Sets or clears the flag; true when the row changed. The triggers recompute `concealed`.
pub fn set_unreadable(conn: &Connection, asset_id: i64, unreadable: bool) -> Result<bool> {
    let n = conn.execute("UPDATE assets SET unreadable = ?2 WHERE id = ?1 AND unreadable != ?2", params![asset_id, unreadable as i64])?;
    Ok(n > 0)
}

/// How many files are flagged (the Settings badge).
pub fn count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM assets WHERE unreadable = 1 AND state = 0", [], |r| r.get(0))?)
}

/// The flagged files after `after` (exclusive), `limit` at a time, in id order.
pub fn list(conn: &Connection, after: Option<i64>, limit: usize) -> Result<UnreadablePage> {
    list_in(conn, false, after, limit)
}

/// Ids of the flagged files that are in the Trash (Round 21 B6: *n in Trash · Put back*).
pub fn trashed_ids(conn: &Connection) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM assets WHERE unreadable = 1 AND state = 1 ORDER BY id")?;
    let ids = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

/// The folders these files were imported from (their batch's `source_path`) with how many of the
/// files each holds, the most first (*Restore…*'s *Look in <path>*). `ids` None: every flagged
/// file. A batch with no folder (a phone, a re-import) is left out; whether a folder still
/// exists is the caller's question.
pub fn source_folders(conn: &Connection, ids: Option<&[i64]>) -> Result<Vec<SourceFolder>> {
    let only = match ids {
        Some(ids) => format!("a.id IN ({})", ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")),
        None => "a.unreadable = 1 AND a.state = 0".to_string(),
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT b.source_path, COUNT(*) AS n FROM assets a JOIN import_batches b ON b.id = a.import_batch_id
         WHERE {only} AND b.source_path IS NOT NULL GROUP BY b.source_path ORDER BY n DESC, b.source_path"
    ))?;
    let rows = stmt.query_map([], |r| Ok(SourceFolder { path: r.get(0)?, files: r.get(1)? }))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// [`list`] for the flagged files in the Trash (`trashed`) or out of it.
pub fn list_in(conn: &Connection, trashed: bool, after: Option<i64>, limit: usize) -> Result<UnreadablePage> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, filename, relative_path, extension, byte_size, content_hash FROM assets
         WHERE unreadable = 1 AND state = ?3 AND id > ?1 ORDER BY id LIMIT ?2",
    )?;
    let mut items: Vec<UnreadableFile> = stmt
        .query_map(params![after.unwrap_or(0), limit as i64 + 1, trashed as i64], |r| {
            Ok(UnreadableFile { id: r.get(0)?, filename: r.get(1)?, relative_path: r.get(2)?, extension: r.get(3)?, byte_size: r.get(4)?, content_hash: r.get(5)? })
        })?
        .collect::<std::result::Result<_, _>>()?;
    let next_cursor = (items.len() > limit).then(|| {
        items.truncate(limit);
        items.last().map(|i| i.id)
    });
    Ok(UnreadablePage { items, next_cursor: next_cursor.flatten() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::albums::{add_to_album, create_album};
    use crate::assets::{insert_assets, NewAsset};
    use crate::concealed::set_album_hides;
    use crate::layout::count_scoped;
    use crate::scope::LayoutScope;
    use crate::Db;

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

    #[test]
    fn an_unreadable_file_is_in_no_grid_count_or_album_but_the_list_and_comes_back_when_cleared() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[photo("a1"), photo("a2"), photo("a3")])?;
            let (good, bad, plain) = (ids[0], ids[1], ids[2]);
            let normal = create_album(c, "Trip", None, 1)?;
            let hiding = create_album(c, "School", None, 1)?;
            add_to_album(c, normal, &[good, plain], 2)?;
            add_to_album(c, hiding, &[bad], 2)?;
            set_album_hides(c, hiding, true, 3)?;
            let count = |s: LayoutScope| count_scoped(c, &s).unwrap();
            let in_album = |id| crate::albums::get_album(c, id).unwrap().item_count;
            let hiding_scope = LayoutScope { album_id: Some(hiding), ..Default::default() };
            assert_eq!(count(LayoutScope::library()), 2, "`bad` is already hidden by its album");
            assert_eq!(count(hiding_scope.clone()), 1, "its own album shows it");

            assert!(set_unreadable(c, bad, true)?);
            assert!(!set_unreadable(c, bad, true)?, "idempotent");
            assert_eq!(count(LayoutScope::library()), 2);
            assert_eq!(count(hiding_scope.clone()), 0, "a hiding album does not show an unreadable file");
            assert_eq!(in_album(hiding), 0);
            assert_eq!(in_album(normal), 2);
            assert_eq!(super::count(c)?, 1);
            let page = list(c, None, 10)?;
            assert_eq!(page.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![bad]);
            assert_eq!(page.next_cursor, None);

            // Un-hiding the album, or taking the file out of it, keeps it hidden.
            set_album_hides(c, hiding, false, 4)?;
            assert_eq!(count(LayoutScope::library()), 2, "still hidden: unreadable");
            assert_eq!(in_album(hiding), 0);
            remove_everywhere(c, bad)?;
            assert_eq!(count(LayoutScope::library()), 2);

            // A later successful decode or Try again clears it.
            assert!(set_unreadable(c, bad, false)?);
            assert_eq!(count(LayoutScope::library()), 3);
            assert_eq!(super::count(c)?, 0);

            // An ordinary photo goes the same way, from the library and from a normal album.
            set_unreadable(c, plain, true)?;
            assert_eq!(count(LayoutScope::library()), 2);
            assert_eq!(count(LayoutScope { album_id: Some(normal), ..Default::default() }), 1);
            assert_eq!(in_album(normal), 1);
            Ok(())
        })
        .unwrap();
    }

    fn remove_everywhere(c: &Connection, id: i64) -> crate::Result<()> {
        c.execute("DELETE FROM album_items WHERE asset_id = ?1", [id])?;
        Ok(())
    }

    #[test]
    fn a_trashed_unreadable_file_leaves_the_list_and_is_counted_in_the_trash_line_until_put_back() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[photo("t1"), photo("t2"), photo("t3")])?;
            for id in &ids[..2] {
                set_unreadable(c, *id, true)?;
            }
            // `t3` is trashed but readable: it is not this list's business.
            crate::assets::trash_assets(c, &[ids[0], ids[2]], 5)?;

            let open = list(c, None, 10)?;
            assert_eq!(open.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![ids[1]]);
            assert_eq!(count(c)?, 1, "the badge counts only what is not in the Trash");
            let trashed = list_in(c, true, None, 10)?;
            assert_eq!(trashed.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![ids[0]]);
            assert_eq!(trashed_ids(c)?, vec![ids[0]]);

            crate::assets::restore_assets(c, &[ids[0]])?;
            assert_eq!(list(c, None, 10)?.items.len(), 2);
            assert!(trashed_ids(c)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn restore_offers_the_folders_the_files_were_imported_from_most_files_first() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[photo("s1"), photo("s2"), photo("s3"), photo("s4")])?;
            let card = crate::batches::start_batch(c, 0, "Card", Some("E:/DCIM"), 1)?;
            let other = crate::batches::start_batch(c, 0, "Old", Some("D:/Old"), 1)?;
            let none = crate::batches::start_batch(c, 0, "Phone", None, 1)?;
            for (id, batch) in [(ids[0], card), (ids[1], card), (ids[2], other), (ids[3], none)] {
                c.execute("UPDATE assets SET import_batch_id = ?2 WHERE id = ?1", params![id, batch])?;
                set_unreadable(c, id, true)?;
            }
            let all = source_folders(c, None)?;
            assert_eq!(all, vec![SourceFolder { path: "E:/DCIM".into(), files: 2 }, SourceFolder { path: "D:/Old".into(), files: 1 }], "a batch without a folder is left out");
            let some = source_folders(c, Some(&[ids[2]]))?;
            assert_eq!(some, vec![SourceFolder { path: "D:/Old".into(), files: 1 }]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_list_pages_by_id() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[photo("b1"), photo("b2"), photo("b3")])?;
            for id in &ids {
                set_unreadable(c, *id, true)?;
            }
            let first = list(c, None, 2)?;
            assert_eq!(first.items.len(), 2);
            let second = list(c, first.next_cursor, 2)?;
            assert_eq!(second.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![ids[2]]);
            assert_eq!(second.next_cursor, None);
            Ok(())
        })
        .unwrap();
    }
}
