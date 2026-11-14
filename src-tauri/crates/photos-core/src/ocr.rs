//! The text read in photos (Round 13): which photos have been read and by which models
//! (`ocr_scan`), and the lines kept (`asset_text`). Every save reindexes the photos' search
//! rows, so a word on a sign is searchable as soon as its batch lands.

use rusqlite::{params, Connection};

use crate::query::AssetKind;
use crate::Result;

/// Lines read with less confidence than this are noise and are not stored (spec §1; tuned
/// on the eval set later).
pub const MIN_CONF: f32 = 0.6;

/// One line as the reader gave it, before the noise filter.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTextLine {
    pub text: String,
    pub conf: f32,
    /// Four corners × (x, y), clockwise from top-left, 0..1 of the unedited photo.
    pub quad: [f32; 8],
}

// Stills only, live and not hidden away: the same set the pet scan reads.
const UNSCANNED: &str = "FROM assets a LEFT JOIN ocr_scan s ON s.asset_id = a.id
     WHERE a.state = 0 AND a.media_type != 1 AND (s.asset_id IS NULL OR s.model != ?1)";

/// Photos not yet read with `key` (`"<det id>+<rec id>"`), newest first, each with whether it
/// is a screenshot (read at a larger size).
pub fn unscanned(conn: &Connection, key: &str, limit: usize) -> Result<Vec<(i64, bool)>> {
    let sql = format!(
        "SELECT a.id, {} {UNSCANNED} AND NOT {} ORDER BY a.id DESC LIMIT ?2",
        AssetKind::Screenshot.predicate(),
        crate::assets::MOVIE_HALF
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![key, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn unscanned_count(conn: &Connection, key: &str) -> Result<u64> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) {UNSCANNED} AND NOT {}", crate::assets::MOVIE_HALF), [key], |r| r.get(0))?;
    Ok(n as u64)
}

/// A line as stored: its place in reading order, its words, how sure the reader was and its
/// corners in 0..1 of the unedited photo.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredLine {
    pub ord: i64,
    pub text: String,
    pub conf: f32,
    pub quad: [f32; 8],
}

/// A photo's lines in reading order (none when it has no text or was never read).
pub fn lines(conn: &Connection, asset: i64) -> Result<Vec<StoredLine>> {
    let mut stmt = conn.prepare_cached("SELECT ord, text, conf, quad FROM asset_text WHERE asset_id = ?1 ORDER BY ord")?;
    let rows = stmt.query_map([asset], |r| {
        let blob: Vec<u8> = r.get(3)?;
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, blob))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (ord, text, conf, blob) = row?;
        // A quad of the wrong size is not drawable; the line is still searchable.
        if let Some(quad) = quad_of(&blob) {
            out.push(StoredLine { ord, text, conf: conf as f32, quad });
        }
    }
    Ok(out)
}

/// Whether the photo has been read with `key` (its lines, possibly none, are current).
pub fn scanned_with(conn: &Connection, asset: i64, key: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT EXISTS (SELECT 1 FROM ocr_scan WHERE asset_id = ?1 AND model = ?2)", params![asset, key], |r| r.get(0))?)
}

fn kept(line: &NewTextLine) -> bool {
    line.conf >= MIN_CONF && line.text.trim().chars().count() >= 2
}

pub fn quad_blob(quad: &[f32; 8]) -> Vec<u8> {
    quad.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// The quad stored by [`save_scans`]; `None` for a blob of the wrong size.
pub fn quad_of(blob: &[u8]) -> Option<[f32; 8]> {
    if blob.len() != 32 {
        return None;
    }
    let mut q = [0f32; 8];
    for (v, b) in q.iter_mut().zip(blob.as_chunks::<4>().0) {
        *v = f32::from_le_bytes(*b);
    }
    Some(q)
}

/// Records each photo as read with `key` and replaces its lines with the ones worth keeping;
/// an empty list (no text, or a photo that would not decode) still marks it read. One
/// transaction, then one reindex for the batch.
pub fn save_scans(conn: &Connection, key: &str, scans: &[(i64, Vec<NewTextLine>)]) -> Result<()> {
    if scans.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut del = tx.prepare_cached("DELETE FROM asset_text WHERE asset_id = ?1")?;
        let mut ins = tx.prepare_cached("INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (?1, ?2, ?3, ?4, ?5, 'Latn')")?;
        let mut scan = tx.prepare_cached("INSERT INTO ocr_scan (asset_id, model) VALUES (?1, ?2) ON CONFLICT(asset_id) DO UPDATE SET model = excluded.model")?;
        for (id, lines) in scans {
            del.execute([id])?;
            for (ord, line) in lines.iter().filter(|l| kept(l)).enumerate() {
                ins.execute(params![id, ord as i64, line.text.trim(), line.conf as f64, quad_blob(&line.quad)])?;
            }
            scan.execute(params![id, key])?;
        }
    }
    let ids: Vec<i64> = scans.iter().map(|(id, _)| *id).collect();
    crate::fts::index_ids(&tx, &ids)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn asset(c: &Connection, id: i64, filename: &str, media_type: i64) {
        c.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
             VALUES (?1, ?2, ?3, ?3, 'jpg', ?4, 1, 1, 1)",
            params![id, format!("h{id}"), filename, media_type],
        )
        .unwrap();
    }

    fn line(text: &str, conf: f32) -> NewTextLine {
        NewTextLine { text: text.into(), conf, quad: [0.1, 0.2, 0.3, 0.2, 0.3, 0.4, 0.1, 0.4] }
    }

    fn texts(c: &Connection, id: i64) -> Vec<(i64, String)> {
        c.prepare("SELECT ord, text FROM asset_text WHERE asset_id = ?1 ORDER BY ord")
            .unwrap()
            .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn ids(v: Vec<(i64, bool)>) -> Vec<i64> {
        v.into_iter().map(|(id, _)| id).collect()
    }

    #[test]
    fn lines_read_back_in_order() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            let mut second = line("Gostilna", 0.9);
            second.quad = [0.5, 0.5, 0.9, 0.5, 0.9, 0.6, 0.5, 0.6];
            save_scans(c, "A", &[(1, vec![line("Kosilo", 0.8), second.clone()])])?;
            let got = lines(c, 1)?;
            assert_eq!(got.iter().map(|l| (l.ord, l.text.as_str())).collect::<Vec<_>>(), vec![(0, "Kosilo"), (1, "Gostilna")]);
            assert_eq!(got[1].quad, second.quad);
            assert!((got[1].conf - 0.9).abs() < 1e-6);
            assert!(lines(c, 2)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn scanned_with_names_the_key() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            asset(c, 2, "b.jpg", 0);
            save_scans(c, "A", &[(1, vec![])])?;
            assert!(scanned_with(c, 1, "A")?);
            assert!(!scanned_with(c, 1, "B")?, "another key reads it again");
            assert!(!scanned_with(c, 2, "A")?, "never read");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn noise_lines_are_not_stored() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            save_scans(c, "A", &[(1, vec![line("faint", 0.5), line("x", 0.95), line("Gostilna", 0.9)])])?;
            assert_eq!(texts(c, 1), vec![(0, "Gostilna".to_string())]);
            Ok(())
        })
        .unwrap();
    }

    /// Review focus 3: a photo with no text, or one that would not decode, never loops.
    #[test]
    fn undecodable_and_textless_assets_are_marked() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            asset(c, 2, "b.jpg", 0);
            assert_eq!(unscanned_count(c, "A")?, 2);
            save_scans(c, "A", &[(1, vec![])])?;
            assert_eq!(ids(unscanned(c, "A", 16)?), vec![2]);
            assert_eq!(unscanned_count(c, "A")?, 1);
            assert!(texts(c, 1).is_empty());
            Ok(())
        })
        .unwrap();
    }

    /// Review focus 4: a new model rescans, and its lines replace the old ones.
    #[test]
    fn a_new_key_replaces_lines() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            save_scans(c, "A", &[(1, vec![line("Gostilna", 0.9), line("MENU", 0.9)])])?;
            assert_eq!(ids(unscanned(c, "B", 16)?), vec![1], "a new key rescans");
            save_scans(c, "B", &[(1, vec![line("Kavarna", 0.9)])])?;
            assert_eq!(texts(c, 1), vec![(0, "Kavarna".to_string())]);
            assert!(unscanned(c, "B", 16)?.is_empty());
            assert_eq!(ids(unscanned(c, "A", 16)?), vec![1]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn videos_are_not_scanned() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            asset(c, 2, "clip.mov", 1);
            asset(c, 3, "Screenshot 2026-10-31.png", 0);
            assert_eq!(unscanned(c, "A", 16)?, vec![(3, true), (1, false)], "newest first, screenshots marked");
            assert_eq!(unscanned_count(c, "A")?, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn saved_text_is_searchable() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            crate::fts::index_ids(c, &[1])?;
            save_scans(c, "A", &[(1, vec![line("Gostilna pri Lojzetu", 0.9)])])?;
            let hits: Vec<i64> = c
                .prepare("SELECT rowid FROM assets_fts WHERE assets_fts MATCH '\"gostilna\"*'")?
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(hits, vec![1]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn quad_round_trips() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            asset(c, 1, "a.jpg", 0);
            let l = line("Gostilna", 0.9);
            save_scans(c, "A", &[(1, vec![l.clone()])])?;
            let blob: Vec<u8> = c.query_row("SELECT quad FROM asset_text WHERE asset_id = 1", [], |r| r.get(0))?;
            assert_eq!(blob.len(), 32);
            assert_eq!(quad_of(&blob), Some(l.quad));
            assert_eq!(quad_of(&blob[..31]), None);
            Ok(())
        })
        .unwrap();
    }
}
