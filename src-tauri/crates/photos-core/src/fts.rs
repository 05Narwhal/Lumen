//! The full-text index behind free-text search. `assets_fts` keeps its own copy of the
//! text (migration 0004 explains why), so a row is replaced by deleting its rowid and
//! inserting again — no shadow copy of the previous values to keep straight.

use std::borrow::Cow;

use rusqlite::{params, CachedStatement, Connection};

use crate::Result;

/// The searchable text of one asset.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FtsRow {
    pub id: i64,
    pub filename: String,
    pub device_name: Option<String>,
    /// Filled by Phase 4's reverse geocoding.
    pub place_name: Option<String>,
    pub keywords: Option<String>,
    /// The lines read in the photo (Round 13), in reading order.
    pub photo_text: Option<String>,
}

// The keywords column carries every piece of text a source attached to the asset (title,
// caption, keywords; migration 0010), the scene tags the model saw (Phase 7b) and the names
// of the people and pets in it (Round 4C), so "Anna", "Milo" and "beach" are plain searches.
// photo_text is the text read in the photo (Round 13); search leaves that column out while
// *Read text in photos* is off (`AssetQuery::photo_text`).
const SELECT_ROWS: &str = "SELECT a.id, a.filename, d.display_name, NULLIF(TRIM(COALESCE(a.place_name, '') || ' ' || COALESCE(a.locality, '')), ''),
                                  NULLIF(TRIM(COALESCE(a.title, '') || ' ' || COALESCE(a.caption, '') || ' ' || COALESCE(a.keywords, '')
                                    || ' ' || COALESCE(a.scene_tags, '')
                                    || ' ' || COALESCE((SELECT group_concat(DISTINCT p.name) FROM faces f JOIN people p ON p.id = f.person_id
                                                        WHERE f.asset_id = a.id AND p.name IS NOT NULL AND p.status = 'listed'), '')
                                    || ' ' || COALESCE((SELECT group_concat(DISTINCT pt.name) FROM pet_detections pd JOIN pets pt ON pt.id = pd.pet_id
                                                        WHERE pd.asset_id = a.id AND pd.rejected IS NULL AND pt.name IS NOT NULL AND pt.status = 'listed'), '')), ''),
                                  (SELECT group_concat(t.text || ' ' || COALESCE(t.alt, ''), ' ')
                                     FROM (SELECT text, alt FROM asset_text WHERE asset_id = a.id ORDER BY ord) t)
                           FROM assets a LEFT JOIN devices d ON d.id = a.device_id";

fn read_rows(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<FtsRow>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt
        .query_map(params, |r| {
            Ok(FtsRow { id: r.get(0)?, filename: r.get(1)?, device_name: r.get(2)?, place_name: r.get(3)?, keywords: r.get(4)?, photo_text: r.get(5)? })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// The indexable text of `ids` as the catalogue currently has it.
pub fn rows_for(conn: &Connection, ids: &[i64]) -> Result<Vec<FtsRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let holes = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
    let sql = format!("{SELECT_ROWS} WHERE a.id IN ({holes})");
    let refs: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|i| i as &dyn rusqlite::ToSql).collect();
    read_rows(conn, &sql, &refs)
}

/// Runs `f` inside the caller's transaction, or in one of its own when there is none. Row by
/// row in autocommit, each FTS5 delete and insert was a commit of its own: naming someone in
/// 1,141 photos was 2,282 commits, each queueing for the write lock behind the face scan's
/// (Round 5B, real library: 0.3 s idle, 2.7–4.8 s with the scan running).
fn in_transaction(conn: &Connection, f: impl FnOnce(&Connection) -> Result<()>) -> Result<()> {
    if !conn.is_autocommit() {
        return f(conn);
    }
    let tx = conn.unchecked_transaction()?;
    f(&tx)?;
    tx.commit()?;
    Ok(())
}

/// Replace the index entries for `rows`. Safe for rows that were never indexed. Joins the
/// caller's transaction (`insert_assets` owns one) or opens its own.
pub fn index_rows(conn: &Connection, rows: &[FtsRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    in_transaction(conn, |conn| write_rows(conn, rows))
}

/// `đ`/`Đ` as `d`/`D`. The tokenizer's `remove_diacritics` folds č, ć, š and ž, but đ is a
/// letter of its own with no combining mark, so *Dakovo* would never find *Đakovo*. Applied
/// to every indexed column and to the query ([`crate::query::AssetQuery::match_expression`]).
pub(crate) fn fold_dj(s: &str) -> Cow<'_, str> {
    if s.contains(['đ', 'Đ']) {
        Cow::Owned(s.replace('đ', "d").replace('Đ', "D"))
    } else {
        Cow::Borrowed(s)
    }
}

fn insert_row(ins: &mut CachedStatement<'_>, r: &FtsRow) -> Result<()> {
    let fold = |v: &Option<String>| v.as_deref().map(|s| fold_dj(s).into_owned());
    ins.execute(params![r.id, fold_dj(&r.filename), fold(&r.device_name), fold(&r.place_name), fold(&r.keywords), fold(&r.photo_text)])?;
    Ok(())
}

fn write_rows(conn: &Connection, rows: &[FtsRow]) -> Result<()> {
    let mut del = conn.prepare_cached("DELETE FROM assets_fts WHERE rowid = ?1")?;
    let mut ins = conn.prepare_cached(
        "INSERT INTO assets_fts(rowid, filename, device_name, place_name, keywords, photo_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )?;
    for r in rows {
        del.execute([r.id])?;
        insert_row(&mut ins, r)?;
    }
    Ok(())
}

/// Index `ids` from whatever the catalogue says about them right now, read and written in
/// one transaction.
pub fn index_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    in_transaction(conn, |conn| write_rows(conn, &rows_for(conn, ids)?))
}

/// Drop `ids` from the index.
pub fn remove_ids(conn: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    in_transaction(conn, |conn| {
        let mut del = conn.prepare_cached("DELETE FROM assets_fts WHERE rowid = ?1")?;
        for id in ids {
            del.execute([id])?;
        }
        Ok(())
    })
}

/// Re-index every asset of one device, after a rename.
pub fn reindex_device(conn: &Connection, device_id: i64) -> Result<usize> {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM assets WHERE device_id = ?1")?;
        let rows = stmt.query_map([device_id], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
        rows
    };
    index_ids(conn, &ids)?;
    Ok(ids.len())
}

/// Throw the index away and build it again from the catalogue. Returns the row count.
pub fn rebuild(conn: &Connection) -> Result<usize> {
    let rows = read_rows(conn, SELECT_ROWS, &[])?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM assets_fts", [])?;
    {
        let mut ins = tx.prepare_cached(
            "INSERT INTO assets_fts(rowid, filename, device_name, place_name, keywords, photo_text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for r in &rows {
            insert_row(&mut ins, r)?;
        }
    }
    tx.commit()?;
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn matches(c: &Connection, expr: &str) -> Vec<i64> {
        let mut stmt = c.prepare("SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?1 ORDER BY rowid").unwrap();
        stmt.query_map([expr], |r| r.get(0)).unwrap().collect::<std::result::Result<Vec<i64>, _>>().unwrap()
    }

    fn insert(c: &Connection, name: &str, device: Option<i64>) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, device_id)
             VALUES (?1, ?1, ?2, 'jpg', 0, 1, 1, 1, ?3)",
            params![format!("h-{name}"), name, device],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    fn add_line(c: &Connection, asset: i64, ord: i64, text: &str) {
        c.execute(
            "INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (?1, ?2, ?3, 0.9, x'00', 'Latn')",
            params![asset, ord, text],
        )
        .unwrap();
    }

    #[test]
    fn photo_text_is_indexed_in_reading_order() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let a = insert(c, "IMG_0001.jpg", None);
            add_line(c, a, 1, "MENU");
            add_line(c, a, 0, "Gostilna");
            let rows = rows_for(c, &[a])?;
            let words: Vec<&str> = rows[0].photo_text.as_deref().unwrap_or("").split_whitespace().collect();
            assert_eq!(words, ["Gostilna", "MENU"]);
            index_ids(c, &[a])?;
            assert_eq!(matches(c, "\"menu\"*"), vec![a]);
            assert_eq!(matches(c, "photo_text : \"gostilna\"*"), vec![a]);
            Ok(())
        })
        .unwrap();
    }

    /// Review focus 1: accents fold both ways, đ/Đ included (it has no combining mark).
    #[test]
    fn photo_text_folds_diacritics_and_dj() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let accented = insert(c, "a.jpg", None);
            add_line(c, accented, 0, "Ćevapčići Đakovo");
            let plain = insert(c, "b.jpg", None);
            add_line(c, plain, 0, "cevapcici dakovo");
            index_ids(c, &[accented, plain])?;
            let both = vec![accented, plain];
            for word in ["cevapcici", "dakovo", "ćevapčići", "đakovo", "ĐAKOVO"] {
                let expr = crate::query::AssetQuery::match_expression(word);
                assert_eq!(matches(c, &expr), both, "{word}");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn indexes_renames_removes_and_rebuilds() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let sony = crate::devices::upsert_device(c, Some("SONY"), Some("ILCE-7M4"), None, 1)?.unwrap();
            let a = insert(c, "IMG_0001.jpg", Some(sony));
            let b = insert(c, "IMG_0002.jpg", Some(sony));
            let cc = insert(c, "Ünïcode Bëach.jpg", None);
            index_ids(c, &[a, b, cc])?;

            assert_eq!(matches(c, "\"img\"*"), vec![a, b]);
            assert_eq!(matches(c, "\"sony\"*"), vec![a, b], "device name is searchable");
            assert_eq!(matches(c, "\"beach\"*"), vec![cc], "diacritics are folded");
            assert_eq!(matches(c, "\"0002\"*"), vec![b]);

            // rename_device re-indexes on its own; calling it again must be harmless.
            crate::devices::rename_device(c, sony, "Studio A7")?;
            assert_eq!(reindex_device(c, sony)?, 2);
            assert_eq!(matches(c, "\"studio\"*"), vec![a, b]);
            assert!(matches(c, "\"sony\"*").is_empty(), "the old name is gone");

            remove_ids(c, &[b])?;
            assert_eq!(matches(c, "\"img\"*"), vec![a]);

            assert_eq!(rebuild(c)?, 3);
            assert_eq!(matches(c, "\"img\"*"), vec![a, b], "rebuild restores what was dropped");

            // Indexing twice must not double-count.
            index_ids(c, &[a])?;
            assert_eq!(matches(c, "\"0001\"*"), vec![a]);
            Ok(())
        })
        .unwrap();
    }
}
