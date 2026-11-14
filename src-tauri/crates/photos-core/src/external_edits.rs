//! The rows behind *Edit in* copies (Round 22 lane C, migration 0038): `external_edits` and
//! `assets.edited_in`. The binary's `external_edits` decides and writes the files; this is its SQL.

use rusqlite::{params, Connection, OptionalExtension};

use crate::assets::NewAsset;
use crate::Result;

/// The source asset of a copy: where it is, what it is called, and in `inherit` what a rendered
/// copy takes from it (date, place, camera, metadata).
#[derive(Debug, Clone)]
pub struct SourceRow {
    pub relative_path: String,
    pub filename: String,
    pub extension: String,
    pub content_hash: String,
    /// Set when this asset is itself a copy made for an app.
    pub edited_in: Option<String>,
    pub inherit: NewAsset,
}

pub fn source_row(conn: &Connection, id: i64) -> Result<Option<SourceRow>> {
    Ok(conn
        .query_row(
            "SELECT relative_path, filename, extension, content_hash, edited_in, captured_at, captured_at_utc,
                    tz_offset_minutes, capture_source, device_id, gps_lat, gps_lon, gps_altitude, metadata_json
               FROM assets WHERE id = ?1",
            [id],
            |r| {
                Ok(SourceRow {
                    relative_path: r.get(0)?,
                    filename: r.get(1)?,
                    extension: r.get(2)?,
                    content_hash: r.get(3)?,
                    edited_in: r.get(4)?,
                    inherit: NewAsset {
                        captured_at: r.get(5)?,
                        captured_at_utc: r.get(6)?,
                        tz_offset_minutes: r.get(7)?,
                        capture_source: r.get(8)?,
                        device_id: r.get(9)?,
                        gps_lat: r.get(10)?,
                        gps_lon: r.get(11)?,
                        gps_altitude: r.get(12)?,
                        metadata_json: r.get(13)?,
                        ..Default::default()
                    },
                })
            },
        )
        .optional()?)
}

/// A copy as it was last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyRow {
    pub relative_path: String,
    pub asset_id: Option<i64>,
    pub seen_size: i64,
    pub seen_mtime: i64,
}

/// Copies made for `(source, app)` with this `made_key` that nobody has saved over (their asset,
/// if any, still has `made_hash`), newest first. The caller still checks the file's size and time.
pub fn reusable(conn: &Connection, source: i64, app_id: &str, made_key: &str) -> Result<Vec<CopyRow>> {
    let mut stmt = conn.prepare(
        "SELECT e.relative_path, e.asset_id, e.seen_size, e.seen_mtime FROM external_edits e
          WHERE e.source_asset_id = ?1 AND e.app_id = ?2 AND e.made_key = ?3
            AND COALESCE((SELECT content_hash FROM assets WHERE id = e.asset_id), e.made_hash) = e.made_hash
          ORDER BY e.id DESC",
    )?;
    let rows = stmt.query_map(params![source, app_id, made_key], |r| {
        Ok(CopyRow { relative_path: r.get(0)?, asset_id: r.get(1)?, seen_size: r.get(2)?, seen_mtime: r.get(3)? })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// Whether a copy already has this path under `originals/`.
pub fn path_taken(conn: &Connection, relative_path: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM external_edits WHERE relative_path = ?1", [relative_path], |_| Ok(())).optional()?.is_some())
}

pub fn set_edited_in(conn: &Connection, asset_id: i64, app_id: &str) -> Result<()> {
    conn.execute("UPDATE assets SET edited_in = ?2 WHERE id = ?1", params![asset_id, app_id])?;
    Ok(())
}

/// A new copy's row.
#[derive(Debug, Clone)]
pub struct NewCopy<'a> {
    pub source_asset_id: i64,
    pub asset_id: Option<i64>,
    pub app_id: &'a str,
    pub relative_path: &'a str,
    pub made_key: &'a str,
    pub made_hash: &'a str,
    pub seen_size: i64,
    pub seen_mtime: i64,
    pub created_at: i64,
}

/// A copy's row with its asset's current hash, for *coming back* (C4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyFull {
    pub id: i64,
    pub source_asset_id: i64,
    pub asset_id: Option<i64>,
    pub app_id: String,
    pub relative_path: String,
    pub made_hash: String,
    pub seen_size: i64,
    pub seen_mtime: i64,
    /// The catalogued copy's `content_hash` now; `None` while the copy waits.
    pub asset_hash: Option<String>,
}

/// Every copy, oldest first.
pub fn all_copies(conn: &Connection) -> Result<Vec<CopyFull>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.source_asset_id, e.asset_id, e.app_id, e.relative_path, e.made_hash, e.seen_size, e.seen_mtime, a.content_hash
           FROM external_edits e LEFT JOIN assets a ON a.id = e.asset_id ORDER BY e.id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(CopyFull {
            id: r.get(0)?,
            source_asset_id: r.get(1)?,
            asset_id: r.get(2)?,
            app_id: r.get(3)?,
            relative_path: r.get(4)?,
            made_hash: r.get(5)?,
            seen_size: r.get(6)?,
            seen_mtime: r.get(7)?,
            asset_hash: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// What the person's editor left of a copy's file, as read now.
#[derive(Debug, Clone, Default)]
pub struct FileChange {
    pub content_hash: String,
    pub byte_size: i64,
    /// Seconds.
    pub file_modified_at: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub orientation: i64,
}

/// A catalogued copy's file was saved over: the row follows the bytes (hash, size, shape) and is
/// readable again; its derivatives are made again by the caller. Dates, albums, ratings stay.
pub fn file_changed(conn: &Connection, asset_id: i64, c: &FileChange) -> Result<()> {
    conn.execute(
        "UPDATE assets SET content_hash = ?2, byte_size = ?3, file_modified_at = ?4, width = ?5, height = ?6,
                orientation = ?7, unreadable = 0 WHERE id = ?1",
        params![asset_id, c.content_hash, c.byte_size, c.file_modified_at, c.width, c.height, c.orientation.max(1)],
    )?;
    Ok(())
}

/// What a copy's file looked like when Lumen last looked.
pub fn set_seen(conn: &Connection, id: i64, size: i64, mtime: i64) -> Result<()> {
    conn.execute("UPDATE external_edits SET seen_size = ?2, seen_mtime = ?3 WHERE id = ?1", params![id, size, mtime])?;
    Ok(())
}

/// A waiting copy became a photo.
pub fn link_asset(conn: &Connection, id: i64, asset_id: i64) -> Result<()> {
    conn.execute("UPDATE external_edits SET asset_id = ?2 WHERE id = ?1", params![id, asset_id])?;
    Ok(())
}

pub fn delete_copy(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM external_edits WHERE id = ?1", [id])?;
    Ok(())
}

/// The asset whose file is at this path under `originals/`.
pub fn asset_at_path(conn: &Connection, relative_path: &str) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT id FROM assets WHERE relative_path = ?1", [relative_path], |r| r.get(0)).optional()?)
}

pub fn insert_copy(conn: &Connection, c: &NewCopy<'_>) -> Result<i64> {
    conn.execute(
        "INSERT INTO external_edits (source_asset_id, asset_id, app_id, relative_path, made_key, made_hash, seen_size, seen_mtime, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![c.source_asset_id, c.asset_id, c.app_id, c.relative_path, c.made_key, c.made_hash, c.seen_size, c.seen_mtime, c.created_at],
    )?;
    Ok(conn.last_insert_rowid())
}
