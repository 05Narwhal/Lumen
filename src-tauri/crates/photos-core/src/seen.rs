//! Incremental import memory: which files on a card or phone were already imported,
//! keyed by the source and the file's relative path, size and mtime. A file that
//! changed size or mtime counts as new again.

use rusqlite::{params, Connection, OptionalExtension};

use crate::Result;

/// What the memory needs to know about a candidate file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenItem<'a> {
    pub rel_path: &'a str,
    pub size: u64,
    pub mtime: Option<i64>,
}

/// Record `items` as imported from `source_key`. One transaction; re-recording updates.
pub fn mark_seen(conn: &Connection, source_key: &str, items: &[SeenItem<'_>], now: i64) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO import_seen (source_key, rel_path, size, mtime, seen_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for it in items {
            stmt.execute(params![source_key, it.rel_path, it.size as i64, it.mtime, now])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Whether one file is unseen: no row, or a different size/mtime.
fn is_unseen(conn: &Connection, source_key: &str, it: &SeenItem<'_>) -> Result<bool> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT size, mtime FROM import_seen WHERE source_key = ?1 AND rel_path = ?2",
            params![source_key, it.rel_path],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(match row {
        None => true,
        Some((size, mtime)) => size != it.size as i64 || mtime != it.mtime,
    })
}

/// Indexes into `items` of the files not yet imported from `source_key`.
pub fn unseen_indexes(conn: &Connection, source_key: &str, items: &[SeenItem<'_>]) -> Result<Vec<usize>> {
    let mut out = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if is_unseen(conn, source_key, it)? {
            out.push(i);
        }
    }
    Ok(out)
}

pub fn count_unseen(conn: &Connection, source_key: &str, items: &[SeenItem<'_>]) -> Result<usize> {
    Ok(unseen_indexes(conn, source_key, items)?.len())
}

/// Forget everything about a source (Settings → "Forget this card").
pub fn forget_source(conn: &Connection, source_key: &str) -> Result<usize> {
    Ok(conn.execute("DELETE FROM import_seen WHERE source_key = ?1", [source_key])?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    #[test]
    fn remembers_by_path_size_and_mtime() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let key = "vol:SDCARD:32000000000";
            let first = [
                SeenItem { rel_path: "100CANON/IMG_0001.CR3", size: 10, mtime: Some(100) },
                SeenItem { rel_path: "100CANON/IMG_0002.CR3", size: 20, mtime: Some(200) },
                SeenItem { rel_path: "100CANON/IMG_0003.CR3", size: 30, mtime: None },
            ];
            assert_eq!(count_unseen(c, key, &first)?, 3);
            mark_seen(c, key, &first, 1_000)?;
            assert_eq!(count_unseen(c, key, &first)?, 0);

            let later = [
                first[0].clone(),
                SeenItem { rel_path: "100CANON/IMG_0002.CR3", size: 21, mtime: Some(200) }, // size changed
                first[2].clone(),
                SeenItem { rel_path: "100CANON/IMG_0004.CR3", size: 40, mtime: Some(400) }, // new
            ];
            assert_eq!(unseen_indexes(c, key, &later)?, vec![1, 3]);
            assert_eq!(count_unseen(c, "vol:OTHER:1", &first)?, 3, "keys are independent");
            assert_eq!(forget_source(c, key)?, 3);
            assert_eq!(count_unseen(c, key, &first)?, 3);
            Ok(())
        })
        .unwrap();
    }
}
