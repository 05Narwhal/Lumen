//! What travels with a photo (Round 19 C3): `.xmp`, `.aae` and `.thm` files copied next to the
//! original and recorded here, and the list of everything an import did not bring in, with the
//! reason. Nothing is dropped without a row (spec 13, items 3 and 4).

use rusqlite::{params, Connection};
use serde::Serialize;
use ts_rs::TS;

use crate::Result;

pub const KIND_XMP: &str = "xmp";
pub const KIND_AAE: &str = "aae";
pub const KIND_THM: &str = "thm";

/// The sidecar kind an extension names (any case), or `None` for everything else.
pub fn kind_for_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "xmp" => Some(KIND_XMP),
        "aae" => Some(KIND_AAE),
        "thm" => Some(KIND_THM),
        _ => None,
    }
}

pub const SKIP_UNSUPPORTED: &str = "unsupported";
pub const SKIP_DUPLICATE: &str = "duplicate";
pub const SKIP_UNREADABLE: &str = "unreadable";
pub const SKIP_BURST_FRAME: &str = "burst_frame";
pub const SKIP_NOT_DOWNLOADED: &str = "not_downloaded";
pub const SKIP_FAILED_VERIFY: &str = "failed_verify";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSidecar {
    pub kind: &'static str,
    /// Under `originals/`, beside the asset's own file.
    pub relative_path: String,
    /// The name it had on the source (`IMG_0001.xmp`).
    pub source_name: String,
    pub byte_size: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Sidecar {
    pub kind: String,
    pub relative_path: String,
    pub source_name: String,
    #[ts(type = "number")]
    pub byte_size: i64,
}

/// Records `s` for `asset_id`; a second one of the same kind replaces the first.
pub fn record_sidecar(conn: &Connection, asset_id: i64, s: &NewSidecar, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO asset_sidecars (asset_id, kind, relative_path, source_name, byte_size, content_hash, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(asset_id, kind) DO UPDATE SET relative_path = excluded.relative_path, source_name = excluded.source_name,
           byte_size = excluded.byte_size, content_hash = excluded.content_hash",
        params![asset_id, s.kind, s.relative_path, s.source_name, s.byte_size, s.content_hash, now],
    )?;
    Ok(())
}

pub fn sidecars_of(conn: &Connection, asset_id: i64) -> Result<Vec<Sidecar>> {
    let mut stmt = conn.prepare("SELECT kind, relative_path, source_name, byte_size FROM asset_sidecars WHERE asset_id = ?1 ORDER BY kind")?;
    let rows = stmt
        .query_map([asset_id], |r| Ok(Sidecar { kind: r.get(0)?, relative_path: r.get(1)?, source_name: r.get(2)?, byte_size: r.get(3)? }))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImportSkip {
    /// The file's path on its source (`rel_path` for a folder or card).
    pub path: String,
    pub reason: String,
    /// The library copy, for a duplicate.
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
}

pub fn record_skip(conn: &Connection, batch_id: i64, path: &str, reason: &str, asset_id: Option<i64>, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO import_skips (batch_id, path, reason, asset_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![batch_id, path, reason, asset_id, now],
    )?;
    Ok(())
}

/// One batch's skips, in the order they were recorded.
pub fn skips_of(conn: &Connection, batch_id: i64) -> Result<Vec<ImportSkip>> {
    let mut stmt = conn.prepare("SELECT path, reason, asset_id FROM import_skips WHERE batch_id = ?1 ORDER BY id")?;
    let rows = stmt
        .query_map([batch_id], |r| Ok(ImportSkip { path: r.get(0)?, reason: r.get(1)?, asset_id: r.get(2)? }))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn asset(c: &Connection, hash: &str) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
             VALUES (?1, ?1, 'a.jpg', 'jpg', 0, 1, 1, 1)",
            [hash],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn extensions_name_their_sidecar_kind_in_any_case() {
        assert_eq!(kind_for_extension("XMP"), Some(KIND_XMP));
        assert_eq!(kind_for_extension("aae"), Some(KIND_AAE));
        assert_eq!(kind_for_extension("Thm"), Some(KIND_THM));
        assert_eq!(kind_for_extension("jpg"), None);
    }

    #[test]
    fn a_sidecar_is_recorded_once_per_kind_and_goes_with_its_asset() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = asset(c, "h1");
            let s = |size| NewSidecar { kind: KIND_XMP, relative_path: "2026/01/01/h1_a.xmp".into(), source_name: "a.xmp".into(), byte_size: size, content_hash: "x".into() };
            record_sidecar(c, id, &s(10), 1)?;
            record_sidecar(c, id, &s(12), 2)?;
            record_sidecar(c, id, &NewSidecar { kind: KIND_AAE, ..s(5) }, 2)?;
            let got = sidecars_of(c, id)?;
            assert_eq!(got.iter().map(|s| (s.kind.as_str(), s.byte_size)).collect::<Vec<_>>(), [("aae", 5), ("xmp", 12)]);
            c.execute("DELETE FROM assets WHERE id = ?1", [id])?;
            assert!(sidecars_of(c, id)?.is_empty(), "a deleted asset keeps no sidecar rows");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn skips_are_listed_per_batch_with_their_reason() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for id in [1, 2] {
                c.execute("INSERT INTO import_batches (id, source_kind, source_label, started_at, state) VALUES (?1, 0, 't', 1, 1)", [id])?;
            }
            let id = asset(c, "h2");
            record_skip(c, 1, "sub/notes.txt", SKIP_UNSUPPORTED, None, 1)?;
            record_skip(c, 1, "sub/copy.jpg", SKIP_DUPLICATE, Some(id), 1)?;
            record_skip(c, 2, "other.bin", SKIP_UNSUPPORTED, None, 1)?;
            let got = skips_of(c, 1)?;
            assert_eq!(got.len(), 2);
            assert_eq!((got[1].reason.as_str(), got[1].asset_id), (SKIP_DUPLICATE, Some(id)));
            assert_eq!(skips_of(c, 2)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
