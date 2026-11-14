use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CoreError, Result};

/// Sort key shared by every timeline query. Assets without a capture time fall back
/// to their import time, never to the end of the list.
pub const TIMELINE_ORDER: &str = "COALESCE(captured_at, imported_at) DESC, id DESC";

/// Extensions that only render through FFmpeg (mirrors `photos_media::kind`).
const NEEDS_FFMPEG: &[&str] = &[
    "heic", "heif", "hif", "avif", "mp4", "m4v", "mov", "avi", "mkv", "mts", "m2ts", "webm", "3gp", "mpg", "mpeg", "wmv",
];

/// Whether a derivative can be made *now*: HEIC and video need FFmpeg.
pub fn is_renderable_extension(ext: &str, ffmpeg_available: bool) -> bool {
    ffmpeg_available || !NEEDS_FFMPEG.contains(&ext.to_ascii_lowercase().as_str())
}

#[derive(Debug, Clone, Default)]
pub struct NewAsset {
    pub content_hash: String,
    pub relative_path: String,
    pub filename: String,
    pub extension: String,
    pub media_type: i64,
    pub byte_size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub orientation: i64,
    pub captured_at: Option<i64>,
    pub captured_at_utc: Option<i64>,
    pub tz_offset_minutes: Option<i64>,
    /// `photos_media::CaptureSource::as_str`; `None` only when there is no date at all.
    pub capture_source: Option<String>,
    pub imported_at: i64,
    pub file_modified_at: Option<i64>,
    /// When the source file was created, if its system said (Round 8A).
    pub file_created_at: Option<i64>,
    pub device_id: Option<i64>,
    pub import_batch_id: Option<i64>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub gps_altitude: Option<f64>,
    pub metadata_json: Option<String>,
    pub duration_ms: Option<i64>,
    /// `photos_media::hdr::HdrKind::code`
    pub hdr: i64,
}

/// Inserts every row in one transaction; returns the new ids in order.
pub fn insert_assets(conn: &Connection, rows: &[NewAsset]) -> Result<Vec<i64>> {
    let tx = conn.unchecked_transaction()?;
    let mut ids = Vec::with_capacity(rows.len());
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, width, height,
               orientation, captured_at, captured_at_utc, tz_offset_minutes, imported_at, file_modified_at, device_id,
               import_batch_id, gps_lat, gps_lon, gps_altitude, metadata_json, duration_ms, hdr, capture_source, file_created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)",
        )?;
        for a in rows {
            stmt.execute(params![
                a.content_hash, a.relative_path, a.filename, a.extension, a.media_type, a.byte_size, a.width, a.height,
                a.orientation.max(1), a.captured_at, a.captured_at_utc, a.tz_offset_minutes, a.imported_at,
                a.file_modified_at, a.device_id, a.import_batch_id, a.gps_lat, a.gps_lon, a.gps_altitude, a.metadata_json,
                a.duration_ms,
                a.hdr,
                a.capture_source,
                a.file_created_at,
            ])?;
            ids.push(tx.last_insert_rowid());
        }
    }
    crate::fts::index_ids(&tx, &ids)?;
    tx.commit()?;
    Ok(ids)
}

pub fn find_by_hash(conn: &Connection, hash: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row("SELECT id FROM assets WHERE content_hash = ?1", [hash], |r| r.get(0))
        .optional()?)
}

/// Store what the derivative pass learned: placeholder hash, mean colour and (when EXIF
/// had none) the decoded dimensions.
/// Curation facts from the 256 px image: sharpness and the 64-bit perceptual hash, stored
/// as the `i64` with the same bits.
pub fn set_quality(conn: &Connection, id: i64, sharpness: f32, phash: u64) -> Result<()> {
    conn.execute("UPDATE assets SET sharpness = ?2, phash = ?3 WHERE id = ?1", params![id, sharpness as f64, phash as i64])?;
    Ok(())
}

/// The date and where it came from, after a re-read.
pub fn set_capture_time(
    conn: &Connection,
    id: i64,
    captured_at: i64,
    captured_at_utc: Option<i64>,
    tz_offset_minutes: Option<i64>,
    source: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE assets SET captured_at = ?2, captured_at_utc = ?3, tz_offset_minutes = ?4, capture_source = ?5 WHERE id = ?1",
        params![id, captured_at, captured_at_utc, tz_offset_minutes, source],
    )?;
    Ok(())
}

/// Assets whose date has no recorded source: imported before Lumen read every date tag.
/// Returns id, path relative to `originals/`, and the file's modified time.
pub fn list_unsourced_dates(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, Option<i64>)>> {
    let mut stmt = conn.prepare(
        "SELECT id, relative_path, file_modified_at FROM assets WHERE capture_source IS NULL AND state = 0 ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Photos whose metadata was read by an older reading than `version` (`metadata_json.exif`):
/// id and path relative to `originals/`, oldest id first (Round 7).
pub fn list_metadata_before(conn: &Connection, version: u8, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, relative_path FROM assets
         WHERE COALESCE(json_extract(metadata_json, '$.exif'), 0) < ?1 ORDER BY id LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![version, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// How many photos [`list_metadata_before`] still has to hand out: the activity ring's total.
pub fn count_metadata_before(conn: &Connection, version: u8) -> Result<u64> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM assets WHERE COALESCE(json_extract(metadata_json, '$.exif'), 0) < ?1",
        params![version],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}

/// Add `patch`'s keys to a photo's metadata (JSON merge patch), keeping every other key.
pub fn merge_metadata(conn: &Connection, id: i64, patch: &serde_json::Value) -> Result<()> {
    conn.execute(
        "UPDATE assets SET metadata_json = json_patch(COALESCE(metadata_json, '{}'), ?2) WHERE id = ?1",
        params![id, patch.to_string()],
    )?;
    Ok(())
}

/// Give a photo the position a newer reading of its original found, only when it has none
/// (a location set by hand is never replaced). Clearing the place names hands it to the
/// maintenance geocoder. Returns whether the row changed.
pub fn fill_missing_gps(conn: &Connection, id: i64, lat: f64, lon: f64, altitude: Option<f64>) -> Result<bool> {
    let n = conn.execute(
        "UPDATE assets SET gps_lat = ?2, gps_lon = ?3, gps_altitude = COALESCE(gps_altitude, ?4), place_name = NULL, locality = NULL, place_source = 0
         WHERE id = ?1 AND (gps_lat IS NULL OR gps_lon IS NULL)",
        params![id, lat, lon, altitude],
    )?;
    Ok(n > 0)
}

pub fn captured_at_of(conn: &Connection, id: i64) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT captured_at FROM assets WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten())
}

/// Assets that have a thumbnail but no curation facts yet (imported before Phase 6).
pub fn list_missing_quality(conn: &Connection, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_hash FROM assets WHERE state = 0 AND sharpness IS NULL AND thumbhash IS NOT NULL ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_derived(
    conn: &Connection,
    id: i64,
    thumbhash: &[u8],
    dominant_color: &str,
    width: Option<i64>,
    height: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE assets SET thumbhash = ?2, dominant_color = ?3,
           width = COALESCE(width, ?4), height = COALESCE(height, ?5) WHERE id = ?1",
        params![id, thumbhash, dominant_color, width, height],
    )?;
    Ok(())
}

/// Record what the decoder actually produced: the orientation still owed to the pixels
/// and their true size. EXIF can disagree with both — a HEIC comes out of FFmpeg already
/// upright, and an iPhone PNG export keeps a rotation tag its pixels no longer need — so
/// the decoded truth replaces the recorded claim rather than coalescing with it.
pub fn set_decoded_geometry(conn: &Connection, id: i64, orientation: i64, width: i64, height: i64) -> Result<()> {
    conn.execute(
        "UPDATE assets SET orientation = ?2, width = ?3, height = ?4 WHERE id = ?1",
        params![id, orientation, width, height],
    )?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AssetDetail {
    #[ts(type = "number")]
    pub id: i64,
    pub filename: String,
    pub extension: String,
    #[ts(type = "number")]
    pub media_type: i64,
    #[ts(type = "number")]
    pub byte_size: i64,
    #[ts(type = "number | null")]
    pub width: Option<i64>,
    #[ts(type = "number | null")]
    pub height: Option<i64>,
    #[ts(type = "number")]
    pub orientation: i64,
    #[ts(type = "number | null")]
    pub captured_at: Option<i64>,
    #[ts(type = "number")]
    pub imported_at: i64,
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    /// 0 ordinary, 1 gain map, 2 PQ, 3 HLG.
    #[ts(type = "number")]
    pub hdr: i64,
    pub device_name: Option<String>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    /// The village or town within 5 km ("Idrija, Slovenia"), once maintenance has named it.
    pub locality: Option<String>,
    /// The town ("Idrija, Slovenia", or just the country), once a place has been named.
    pub place_name: Option<String>,
    /// 0 the built-in atlas (or nothing yet), 1 OpenStreetMap's exact names (migration 0035).
    #[ts(type = "number")]
    pub place_source: i64,
    pub is_favorite: bool,
    #[ts(type = "number")]
    pub rating: i64,
    pub relative_path: String,
    pub content_hash: String,
    #[ts(type = "Record<string, unknown> | null")]
    pub metadata: Option<serde_json::Value>,
    pub renderable: bool,
    /// A camera RAW original, for the badge and the info panel.
    pub raw: bool,
    /// The movie half of a Live Photo, played by the viewer on demand.
    #[ts(type = "number | null")]
    pub live_video_id: Option<i64>,
    #[ts(type = "number")]
    pub color_label: i64,
    #[ts(type = "number")]
    pub flag: i64,
    /// The photo this one was exported from, when an export was added to the library.
    pub derived_from: Option<DerivedFrom>,
    /// Kept from a damaged file (Round 22 §4): the share of it that was recovered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<i32>")]
    pub recovered_pct: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DerivedFrom {
    #[ts(type = "number")]
    pub id: i64,
    pub filename: String,
}

/// Mark the asset with `content_hash` as exported from `source`. An asset that already has
/// a source keeps it, and nothing is its own source. Returns whether a row changed.
pub fn link_derived(conn: &Connection, content_hash: &str, source: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE assets SET derived_from = ?2 WHERE content_hash = ?1 AND id != ?2 AND derived_from IS NULL",
        params![content_hash, source],
    )? > 0)
}

/// The photos in the library (not trashed) holding these bytes, in the order asked; a hash
/// the library does not hold is skipped. How an export into an album finds what it made,
/// new or already there (Round 5).
pub fn ids_by_hash(conn: &Connection, hashes: &[String]) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM assets WHERE content_hash = ?1 AND state = 0 ORDER BY id LIMIT 1")?;
    let mut out = Vec::with_capacity(hashes.len());
    for h in hashes {
        if let Some(id) = stmt.query_row([h], |r| r.get::<_, i64>(0)).optional()? {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    Ok(out)
}

/// A video probed after import (FFmpeg arrived later): store the probe and its facts.
pub fn set_video_facts(
    conn: &Connection,
    id: i64,
    metadata_json: &str,
    duration_ms: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE assets SET metadata_json = ?2, duration_ms = COALESCE(?3, duration_ms),
           width = COALESCE(?4, width), height = COALESCE(?5, height) WHERE id = ?1",
        params![id, metadata_json, duration_ms, width, height],
    )?;
    Ok(())
}

/// `assets.media_type` for the still half of a Live Photo. It is neither a plain photo
/// nor a video: the grid shows one tile that can play its MOV.
pub const MEDIA_TYPE_LIVE: i64 = 2;

/// The movie half of a Live Photo, for a query that names the table `a`. It is reached only
/// through its still, so it is never a tile, never counted and never listed on its own.
pub const MOVIE_HALF: &str = "(a.media_type = 1 AND a.live_photo_group IS NOT NULL)";

/// Apple's movie half runs about three seconds. Anything much longer that shares a stem
/// with a still is a video in its own right.
const LIVE_MOVIE_MAX_MS: i64 = 6_000;

/// `ids` plus the movie half of every Live Photo among them, so a Live Photo is trashed,
/// restored, deleted and copied as the one item the user sees.
pub fn with_movie_halves(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let list = in_list(ids);
    let mut stmt = conn.prepare(&format!(
        "SELECT mov.id FROM assets still
         JOIN assets mov ON mov.live_photo_group = still.live_photo_group AND mov.media_type = 1
         WHERE still.id IN ({list}) AND still.media_type = {MEDIA_TYPE_LIVE}"
    ))?;
    let mut out = ids.to_vec();
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))? {
        let id = id?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Pairs Live Photos: a still and a `.mov` that share a filename stem and the folder they came
/// from become one item. Both rows get `live_photo_group = <stem>#<still id>`; the still
/// becomes `MEDIA_TYPE_LIVE` and the MOV half is hidden from every grid by `LayoutScope`.
///
/// The partner is looked for in this batch and, when the other half came earlier, in older
/// ones: same `source_dir` (set at import, so old rows never pair) and a capture time within
/// [`LIVE_PAIR_SECONDS`]. Returns how many pairs were made.
pub fn pair_live_photos(conn: &Connection, batch_id: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let pairs: Vec<(i64, i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT still.id, mov.id, LOWER(rtrim(still.filename, replace(still.filename, '.', '')))
             FROM assets still
             JOIN assets mov
               ON mov.media_type = 1
              AND mov.live_photo_group IS NULL
              AND COALESCE(mov.duration_ms, 0) <= ?2
              AND LOWER(mov.filename) = LOWER(
                    rtrim(still.filename, replace(still.filename, '.', '')) || 'mov')
              AND mov.source_dir IS still.source_dir
              AND (mov.import_batch_id IS still.import_batch_id
                   OR (still.source_dir IS NOT NULL
                       AND ABS(COALESCE(mov.captured_at, 0) - COALESCE(still.captured_at, 0)) <= ?3))
             WHERE ?1 IN (still.import_batch_id, mov.import_batch_id)
               AND still.media_type = 0
               AND still.live_photo_group IS NULL
             ORDER BY still.id, mov.id",
        )?;
        let rows = stmt
            .query_map(params![batch_id, LIVE_MOVIE_MAX_MS, LIVE_PAIR_SECONDS], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    let (mut stills, mut movs) = (std::collections::HashSet::new(), std::collections::HashSet::new());
    let mut made = 0;
    for (still, mov, stem) in &pairs {
        if !stills.insert(*still) || !movs.insert(*mov) {
            continue;
        }
        // `stem` still carries the trailing dot from rtrim; the id keeps two pairs that share a
        // name (two cards, two folders) from joining on the group.
        let group = format!("{}#{still}", stem.trim_end_matches('.'));
        tx.execute("UPDATE assets SET live_photo_group = ?2, media_type = ?3 WHERE id = ?1", params![still, group, MEDIA_TYPE_LIVE])?;
        tx.execute("UPDATE assets SET live_photo_group = ?2 WHERE id = ?1", params![mov, group])?;
        made += 1;
    }
    tx.commit()?;
    Ok(made)
}

/// How far apart a still and its movie may be when they arrive in different batches.
const LIVE_PAIR_SECONDS: i64 = 3;

/// Records the folder `id` came from on its source (`''` at the root), for Live Photo pairing.
pub fn set_source_dir(conn: &Connection, id: i64, dir: &str) -> Result<()> {
    conn.execute("UPDATE assets SET source_dir = ?2 WHERE id = ?1", params![id, dir])?;
    Ok(())
}

/// The MOV half of a Live Photo, when there is one.
pub fn live_photo_mov(conn: &Connection, still_id: i64) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT mov.id FROM assets still
             JOIN assets mov ON mov.live_photo_group = still.live_photo_group AND mov.media_type = 1
             WHERE still.id = ?1 AND still.live_photo_group IS NOT NULL AND mov.state = 0",
            [still_id],
            |r| r.get(0),
        )
        .optional()?)
}

/// Geotagged assets whose place has not been looked up yet, oldest first.
pub fn list_needing_place(conn: &Connection, limit: usize) -> Result<Vec<(i64, f64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, gps_lat, gps_lon FROM assets
         WHERE state = 0 AND gps_lat IS NOT NULL AND gps_lon IS NOT NULL AND place_name IS NULL
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Store the place a photo was taken and make it searchable. An empty string marks
/// "looked up, nothing to say", so the backfill does not try the same photo forever.
pub fn set_places(conn: &Connection, places: &[(i64, String)]) -> Result<usize> {
    if places.is_empty() {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached("UPDATE assets SET place_name = ?2 WHERE id = ?1")?;
        for (id, place) in places {
            stmt.execute(params![id, place])?;
        }
    }
    crate::fts::index_ids(&tx, &places.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    tx.commit()?;
    Ok(places.len())
}

/// Geotagged assets not yet named online (`place_source = 0`: the atlas or nothing), oldest
/// first, with their lookup key (`places_online::key`).
pub fn list_needing_online_place(conn: &Connection, limit: usize) -> Result<Vec<(i64, String, f64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, gps_lat, gps_lon FROM assets
         WHERE place_source = 0 AND state = 0 AND gps_lat IS NOT NULL AND gps_lon IS NOT NULL
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| {
            let (lat, lon): (f64, f64) = (r.get(1)?, r.get(2)?);
            Ok((r.get(0)?, crate::places_online::key(lat, lon), lat, lon))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// How many geotagged assets still wait for an online name (the activity ring's total).
pub fn count_needing_online_place(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM assets WHERE place_source = 0 AND state = 0 AND gps_lat IS NOT NULL AND gps_lon IS NOT NULL",
        [],
        |r| r.get(0),
    )?)
}

/// The names an earlier lookup of exactly this key gave, if any.
pub fn lookup_cached(conn: &Connection, key: &str) -> Result<Option<crate::places_online::Names>> {
    Ok(conn
        .query_row("SELECT place_name, locality, country_code FROM place_lookups WHERE key = ?1", [key], |r| {
            Ok(crate::places_online::Names { place_name: r.get(0)?, locality: r.get(1)?, country_code: r.get(2)? })
        })
        .optional()?)
}

/// Keep a lookup's answer for every later photo at the same key. Never leaves the machine.
pub fn cache_lookup(conn: &Connection, key: &str, names: &crate::places_online::Names, address_json: &str, now: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO place_lookups (key, place_name, locality, country_code, address_json, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(key) DO UPDATE SET place_name = ?2, locality = ?3, country_code = ?4, address_json = ?5, fetched_at = ?6",
        params![key, names.place_name, names.locality, names.country_code, address_json, now],
    )?;
    Ok(())
}

/// Replace both names with the online ones, mark them `place_source = 1` and make them
/// searchable, in one transaction.
pub fn set_online_places(conn: &Connection, places: &[(i64, crate::places_online::Names)]) -> Result<usize> {
    if places.is_empty() {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached("UPDATE assets SET place_name = ?2, locality = ?3, place_source = 1 WHERE id = ?1")?;
        for (id, n) in places {
            stmt.execute(params![id, n.place_name, n.locality])?;
        }
    }
    crate::fts::index_ids(&tx, &places.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    tx.commit()?;
    Ok(places.len())
}

/// Geotagged assets whose locality has not been looked up yet (Round 4K), oldest first.
pub fn list_needing_locality(conn: &Connection, limit: usize) -> Result<Vec<(i64, f64, f64)>> {
    let mut stmt = conn.prepare(
        "SELECT id, gps_lat, gps_lon FROM assets
         WHERE state = 0 AND gps_lat IS NOT NULL AND gps_lon IS NOT NULL AND locality IS NULL
         ORDER BY id LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Store the locality a photo was taken in and make it searchable. `''` marks "looked up,
/// nothing near", so the backfill does not try the same photo forever.
pub fn set_localities(conn: &Connection, localities: &[(i64, String)]) -> Result<usize> {
    if localities.is_empty() {
        return Ok(0);
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached("UPDATE assets SET locality = ?2 WHERE id = ?1")?;
        for (id, locality) in localities {
            stmt.execute(params![id, locality])?;
        }
    }
    crate::fts::index_ids(&tx, &localities.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    tx.commit()?;
    Ok(localities.len())
}

/// Assets (state 0) that never got a thumbhash: catalogued before FFmpeg was available,
/// or whose ingest job was lost to a crash. The maintenance pass re-queues them.
pub fn list_underived(conn: &Connection, limit: usize) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM assets WHERE state = 0 AND thumbhash IS NULL ORDER BY id LIMIT ?1")?;
    let rows = stmt.query_map([limit as i64], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
    Ok(rows)
}

/// `(id, content_hash, width, height, orientation)` of a derived still.
pub type DerivedShape = (i64, String, i64, i64, i64);

/// Every derived still with its catalogue geometry, for checking that its thumbnail has the
/// shape the catalogue says.
pub fn derived_shapes(conn: &Connection) -> Result<Vec<DerivedShape>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_hash, width, height, orientation FROM assets
         WHERE state = 0 AND media_type != 1 AND thumbhash IS NOT NULL AND width > 0 AND height > 0 ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Sends these assets back through ingest (their derivatives are redone and their
/// orientation re-decided from the pixels). Returns how many were marked.
pub fn mark_underived(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(conn.execute(&format!("UPDATE assets SET thumbhash = NULL WHERE id IN ({})", in_list(ids)), [])?)
}

/// `ffmpeg_available` decides whether HEIC and video count as renderable.
/// Trashed photos too: the Trash shows them until they are deleted for good.
pub fn get_asset(conn: &Connection, id: i64, ffmpeg_available: bool) -> Result<AssetDetail> {
    conn.query_row(
        "SELECT a.id, a.filename, a.extension, a.media_type, a.byte_size, a.width, a.height, a.orientation,
                a.captured_at, a.imported_at, d.display_name, a.gps_lat, a.gps_lon, a.is_favorite, a.relative_path,
                a.content_hash, a.metadata_json, a.duration_ms, a.rating, a.hdr, a.color_label, a.flag,
                src.id, src.filename, NULLIF(a.locality, ''), NULLIF(a.place_name, ''), a.place_source, a.recovered_pct
         FROM assets a LEFT JOIN devices d ON d.id = a.device_id
         LEFT JOIN assets src ON src.id = a.derived_from AND src.state = 0
         WHERE a.id = ?1",
        [id],
        |r| {
            let ext: String = r.get(2)?;
            let meta: Option<String> = r.get(16)?;
            Ok(AssetDetail {
                id: r.get(0)?,
                filename: r.get(1)?,
                renderable: is_renderable_extension(&ext, ffmpeg_available),
                extension: ext,
                media_type: r.get(3)?,
                byte_size: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                orientation: r.get(7)?,
                captured_at: r.get(8)?,
                imported_at: r.get(9)?,
                device_name: r.get(10)?,
                gps_lat: r.get(11)?,
                gps_lon: r.get(12)?,
                locality: r.get(24)?,
                place_name: r.get(25)?,
                place_source: r.get(26)?,
                is_favorite: r.get::<_, i64>(13)? != 0,
                relative_path: r.get(14)?,
                content_hash: r.get(15)?,
                metadata: meta.and_then(|m| serde_json::from_str(&m).ok()),
                duration_ms: r.get(17)?,
                rating: r.get(18)?,
                hdr: r.get(19)?,
                raw: false,
                live_video_id: None,
                color_label: r.get(20)?,
                flag: r.get(21)?,
                derived_from: match (r.get::<_, Option<i64>>(22)?, r.get::<_, Option<String>>(23)?) {
                    (Some(id), Some(filename)) => Some(DerivedFrom { id, filename }),
                    _ => None,
                },
                recovered_pct: r.get(27)?,
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(format!("asset {id}")),
        e => CoreError::from(e),
    })
    .and_then(|mut d| {
        d.raw = crate::formats::is_raw(&d.extension);
        if d.media_type == MEDIA_TYPE_LIVE {
            d.live_video_id = live_photo_mov(conn, id)?;
        }
        Ok(d)
    })
}

/// What the protocol handler needs to serve or render an asset.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderInfo {
    pub content_hash: String,
    pub extension: String,
    pub hdr: i64,
    pub media_type: i64,
    pub orientation: i64,
    pub relative_path: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// The saved edit and its version, when the photo has one.
    pub edit: Option<(crate::edits::Recipe, i64)>,
    /// As imported, for naming exports.
    pub filename: String,
    /// The AI rasters `edit`'s masks name (Phase 8f). Empty for almost every photo.
    pub ai_rasters: crate::ai_masks::AiRasters,
}

/// `info.edit`, unless it doesn't change a pixel here, and the rasters it names.
fn set_edit(conn: &Connection, id: i64, info: &mut RenderInfo, edit: Option<(crate::edits::Recipe, i64)>) -> Result<()> {
    let edit = edit.filter(|(r, _)| r.changes_pixels());
    let rasters = edit.as_ref().map(|(r, _)| crate::ai_masks::rasters_for(conn, id, r)).transpose()?.unwrap_or_default();
    info.edit = edit.filter(|(r, _)| r.changes_pixels_with(&|h| rasters.contains_key(h)));
    info.ai_rasters = rasters;
    Ok(())
}

/// Trashed photos too, so the Trash has thumbnails and previews.
/// The newest videos in the library, newest first (Round 8A: their proxies are made ahead).
pub fn newest_video_ids(conn: &Connection, limit: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM assets WHERE media_type = 1 AND state = 0 ORDER BY imported_at DESC, id DESC LIMIT ?1")?;
    let rows = stmt.query_map([limit], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A video as the background video prep sees it (Round 9A).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoPrepRow {
    pub id: i64,
    pub content_hash: String,
    pub relative_path: String,
    pub metadata_json: Option<String>,
    pub byte_size: i64,
    pub captured_at: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// The library's videos not in the trash, a page at a time by id, highest first: pass the
/// last page's last id as `before`. The caller orders them (newest first) in Rust.
pub fn video_prep_rows(conn: &Connection, before: Option<i64>, limit: i64) -> Result<Vec<VideoPrepRow>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, content_hash, relative_path, metadata_json, byte_size, captured_at, duration_ms FROM assets
         WHERE media_type = 1 AND state = 0 AND id < ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![before.unwrap_or(i64::MAX), limit], |r| {
        Ok(VideoPrepRow {
            id: r.get(0)?,
            content_hash: r.get(1)?,
            relative_path: r.get(2)?,
            metadata_json: r.get(3)?,
            byte_size: r.get(4)?,
            captured_at: r.get(5)?,
            duration_ms: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `metadata_json` of a video, `None` for anything else (Round 8A).
pub fn video_metadata(conn: &Connection, id: i64) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT metadata_json FROM assets WHERE id = ?1 AND media_type = 1", [id], |r| r.get::<_, Option<String>>(0))
        .optional()?
        .flatten())
}

pub fn render_info(conn: &Connection, id: i64) -> Result<RenderInfo> {
    let mut info = conn.query_row(
        "SELECT content_hash, extension, media_type, orientation, relative_path, hdr, width, height, filename FROM assets WHERE id = ?1",
        [id],
        |r| {
            Ok(RenderInfo {
                content_hash: r.get(0)?,
                extension: r.get(1)?,
                media_type: r.get(2)?,
                orientation: r.get(3)?,
                relative_path: r.get(4)?,
                hdr: r.get(5)?,
                width: r.get(6)?,
                height: r.get(7)?,
                edit: None,
                filename: r.get(8)?,
                ai_rasters: Default::default(),
            })
        },
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(format!("asset {id}")),
        e => CoreError::from(e),
    })?;
    // A trim alone changes no pixel: the video's poster and thumbnails stay the original's.
    let edit = crate::edits::get_recipe(conn, id)?;
    set_edit(conn, id, &mut info, edit)?;
    Ok(info)
}

/// `render_info` for an export (Round 6): Develop's draft when the photo has one, which the
/// library never shows.
pub fn export_render_info(conn: &Connection, id: i64) -> Result<RenderInfo> {
    let mut info = render_info(conn, id)?;
    let edit = crate::edits::export_recipe(conn, id)?;
    set_edit(conn, id, &mut info, edit)?;
    Ok(info)
}

fn in_list(ids: &[i64]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

pub fn set_favorite(conn: &Connection, ids: &[i64], value: bool) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(conn.execute(
        &format!("UPDATE assets SET is_favorite = ?1 WHERE id IN ({})", in_list(ids)),
        [value as i64],
    )?)
}

/// A filename in `dir` that is free: `IMG_1.jpg`, then `IMG_1 (2).jpg`, and so on.
/// Export never overwrites, because the folder is the user's, not ours.
pub fn unique_export_name(dir: &std::path::Path, filename: &str) -> std::path::PathBuf {
    let path = std::path::Path::new(filename);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let mut candidate = dir.join(filename);
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

/// `(relative_path, filename)` for the given ids, skipping trashed rows.
pub fn export_paths(conn: &Connection, ids: &[i64]) -> Result<Vec<(String, String)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids = with_movie_halves(conn, ids)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT relative_path, filename FROM assets WHERE state = 0 AND id IN ({})",
        in_list(&ids)
    ))?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Stars, 0 (none) to 5. Values outside that range are clamped rather than rejected:
/// the UI can only produce 0–5, and a bad value is not worth failing a batch over.
pub fn set_rating(conn: &Connection, ids: &[i64], rating: i64) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(conn.execute(&format!("UPDATE assets SET rating = ?1 WHERE id IN ({})", in_list(ids)), [rating.clamp(0, 5)])?)
}

/// Newest favorites first, for the Home strip.
pub fn favorite_ids(conn: &Connection, limit: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT id FROM assets WHERE state = 0 AND concealed = 0 AND is_favorite = 1 ORDER BY {TIMELINE_ORDER} LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

pub const STATE_ACTIVE: i64 = 0;
pub const STATE_TRASHED: i64 = 1;

/// What a file is, when its name said otherwise (Round 8A: a HEIF saved as `.JPG`). The
/// original and its name stay as they are; only how the catalogue reads it changes.
pub fn set_extension(conn: &Connection, id: i64, extension: &str) -> Result<()> {
    conn.execute("UPDATE assets SET extension = ?2 WHERE id = ?1", params![id, extension])?;
    Ok(())
}

/// [`set_extension`] for a file whose container is a movie or a still whatever its name said
/// (a QuickTime movie saved as `.JPG`, a HEIF saved as `.MOV`): the extension, and `media_type`
/// 1 for a movie. A still that was filed as a video goes back to 0; a Live Photo's still
/// (2) and its movie half are not touched by a still-to-still correction.
pub fn set_container(conn: &Connection, id: i64, extension: &str, is_video: bool) -> Result<()> {
    conn.execute(
        "UPDATE assets SET extension = ?2,
           media_type = CASE WHEN ?3 THEN 1 WHEN media_type = 1 THEN 0 ELSE media_type END WHERE id = ?1",
        params![id, extension, is_video],
    )?;
    Ok(())
}

pub fn trash_assets(conn: &Connection, ids: &[i64], now: i64) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let ids = with_movie_halves(conn, ids)?;
    Ok(conn.execute(
        &format!("UPDATE assets SET state = 1, trashed_at = ?1 WHERE state = 0 AND id IN ({})", in_list(&ids)),
        [now],
    )?)
}

pub fn restore_assets(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let ids = with_movie_halves(conn, ids)?;
    Ok(conn.execute(
        &format!("UPDATE assets SET state = 0, trashed_at = NULL WHERE state = 1 AND id IN ({})", in_list(&ids)),
        [],
    )?)
}

/// Trashed rows older than `cutoff` (unix seconds): `(id, content_hash, relative_path)`.
pub fn list_expired(conn: &Connection, cutoff: i64) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, content_hash, relative_path FROM assets WHERE state = 1 AND trashed_at IS NOT NULL AND trashed_at <= ?1",
    )?;
    let rows = stmt.query_map([cutoff], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

pub fn list_trashed(conn: &Connection, ids: Option<&[i64]>) -> Result<Vec<(i64, String, String)>> {
    let filter = match ids {
        Some(ids) if !ids.is_empty() => format!(" AND id IN ({})", in_list(&with_movie_halves(conn, ids)?)),
        Some(_) => return Ok(Vec::new()),
        None => String::new(),
    };
    let mut stmt = conn.prepare(&format!("SELECT id, content_hash, relative_path FROM assets WHERE state = 1{filter}"))?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

pub fn delete_rows(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    crate::fts::remove_ids(conn, ids)?;
    Ok(conn.execute(&format!("DELETE FROM assets WHERE id IN ({})", in_list(ids)), [])?)
}

/// Permanently removes trashed rows (all of them, or `only`): original file, derivatives
/// (through `delete_derivatives(hash)`), then the row. A missing original is not an error.
pub fn delete_permanently(
    lib: &crate::Library,
    only: Option<&[i64]>,
    delete_derivatives: &dyn Fn(&str),
) -> Result<usize> {
    let rows = lib.db.with_conn(|c| list_trashed(c, only))?;
    let mut done = Vec::with_capacity(rows.len());
    for (id, hash, rel) in rows {
        match std::fs::remove_file(lib.originals_dir().join(&rel)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        delete_derivatives(&hash);
        done.push(id);
    }
    lib.db.with_conn(|c| delete_rows(c, &done))
}

/// The 30-day purge: everything trashed at or before `cutoff`.
pub fn purge_expired(lib: &crate::Library, cutoff: i64, delete_derivatives: &dyn Fn(&str)) -> Result<usize> {
    let ids: Vec<i64> = lib.db.with_conn(|c| list_expired(c, cutoff))?.into_iter().map(|(id, _, _)| id).collect();
    if ids.is_empty() {
        return Ok(0);
    }
    delete_permanently(lib, Some(&ids), delete_derivatives)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Neighbors {
    #[ts(type = "number | null")]
    pub prev: Option<i64>,
    #[ts(type = "number | null")]
    pub next: Option<i64>,
}

/// Neighbours in timeline order. `prev` is the newer item (earlier in the grid).
pub fn neighbors(conn: &Connection, id: i64) -> Result<Neighbors> {
    let (ts, _): (i64, i64) = conn
        .query_row(
            "SELECT COALESCE(captured_at, imported_at), id FROM assets WHERE id = ?1 AND state = 0",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => CoreError::NotFound(format!("asset {id}")),
            e => e.into(),
        })?;
    // A Live Photo's MOV half is part of its still, never a stop of its own.
    let prev: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT id FROM assets a WHERE a.state = 0 AND a.concealed = 0 AND NOT {MOVIE_HALF} AND (COALESCE(a.captured_at, a.imported_at), a.id) > (?1, ?2)
             ORDER BY COALESCE(a.captured_at, a.imported_at) ASC, a.id ASC LIMIT 1"
            ),
            params![ts, id],
            |r| r.get(0),
        )
        .optional()?;
    let next: Option<i64> = conn
        .query_row(
            &format!(
                "SELECT id FROM assets a WHERE a.state = 0 AND a.concealed = 0 AND NOT {MOVIE_HALF} AND (COALESCE(a.captured_at, a.imported_at), a.id) < (?1, ?2)
             ORDER BY COALESCE(a.captured_at, a.imported_at) DESC, a.id DESC LIMIT 1"
            ),
            params![ts, id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(Neighbors { prev, next })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LibraryStats {
    #[ts(type = "number")]
    pub items: i64,
    #[ts(type = "number")]
    pub videos: i64,
    #[ts(type = "number")]
    pub bytes: i64,
}

pub fn stats(conn: &Connection) -> Result<LibraryStats> {
    Ok(conn.query_row(
        &format!(
            // Hidden photos are not counted (they appear nowhere), but their bytes are still
            // the library's.
            "SELECT COALESCE(SUM(a.concealed = 0 AND NOT {MOVIE_HALF}), 0), COALESCE(SUM(a.concealed = 0 AND a.media_type = 1 AND NOT {MOVIE_HALF}), 0),
                    COALESCE(SUM(a.byte_size), 0)
             FROM assets a WHERE a.state = 0"
        ),
        [],
        |r| Ok(LibraryStats { items: r.get(0)?, videos: r.get(1)?, bytes: r.get(2)? }),
    )?)
}

/// Every asset (trashed ones too) whose stored orientation still turns its pixels, with its
/// extension: the orientation backfill's candidates. Which extensions' decoders already
/// turn the pixels is `photos-media`'s knowledge, so the caller filters.
pub fn oriented_rows(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, extension FROM assets WHERE orientation != 1 AND state IN (0, 1) ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// Marks `ids` upright and queues them to be derived again (no thumbhash), dropping what was
/// learned from the wrongly turned pixels: the image embedding and the face scan with its
/// faces. Returns how many rows changed; a row already upright is left alone.
pub fn reset_orientation(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let list = in_list(ids);
    let tx = conn.unchecked_transaction()?;
    let n = tx.execute(&format!("UPDATE assets SET orientation = 1, thumbhash = NULL WHERE orientation != 1 AND id IN ({list})"), [])?;
    if n > 0 {
        tx.execute(&format!("DELETE FROM asset_embeddings WHERE asset_id IN ({list})"), [])?;
        tx.execute(&format!("DELETE FROM faces WHERE asset_id IN ({list})"), [])?;
        tx.execute(&format!("DELETE FROM face_scan WHERE asset_id IN ({list})"), [])?;
    }
    tx.commit()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn asset(hash: &str, name: &str, ext: &str, captured_at: Option<i64>, media_type: i64, bytes: i64) -> NewAsset {
        NewAsset {
            content_hash: hash.into(),
            relative_path: format!("2026/09/17/{hash}_{name}"),
            filename: name.into(),
            extension: ext.into(),
            media_type,
            byte_size: bytes,
            orientation: 1,
            captured_at,
            imported_at: 1_000,
            ..Default::default()
        }
    }

    /// Exact place names: rows named by the atlas wait for an online name; the online name
    /// replaces both, is searchable, and a new position hands the row back.
    #[test]
    fn online_place_names_replace_the_atlas_and_are_searchable() {
        use crate::places_online::Names;
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("p1", "IMG_1.HEIC", "heic", Some(1), 0, 1), asset("p2", "IMG_2.HEIC", "heic", Some(2), 0, 1), asset("p3", "IMG_3.HEIC", "heic", Some(3), 0, 1)])?;
            c.execute("UPDATE assets SET gps_lat = 46.0006, gps_lon = 14.0264 WHERE id IN (?1, ?2)", [ids[0], ids[1]])?;
            set_places(c, &[(ids[0], "Gorizia, Italy".into()), (ids[1], "Gorizia, Italy".into())])?;
            let todo = list_needing_online_place(c, 10)?;
            assert_eq!(todo.iter().map(|t| (t.0, t.1.as_str())).collect::<Vec<_>>(), vec![(ids[0], "46.00060,14.02640"), (ids[1], "46.00060,14.02640")]);
            assert_eq!(count_needing_online_place(c)?, 2, "the photo without a position waits for nothing");

            let names = Names { place_name: "Idrija, Slovenia".into(), locality: "Idrija, Slovenia".into(), country_code: Some("si".into()) };
            assert_eq!(lookup_cached(c, "46.00060,14.02640")?, None);
            cache_lookup(c, "46.00060,14.02640", &names, r#"{"town":"Idrija"}"#, 5)?;
            cache_lookup(c, "46.00060,14.02640", &names, r#"{"town":"Idrija"}"#, 6)?;
            assert_eq!(lookup_cached(c, "46.00060,14.02640")?, Some(names.clone()));
            assert_eq!(lookup_cached(c, "46.00061,14.02640")?, None, "no nearby reuse");

            assert_eq!(set_online_places(c, &[(ids[0], names.clone()), (ids[1], names.clone())])?, 2);
            assert!(list_needing_online_place(c, 10)?.is_empty());
            let row: (String, String, i64) =
                c.query_row("SELECT place_name, locality, place_source FROM assets WHERE id = ?1", [ids[0]], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            assert_eq!(row, ("Idrija, Slovenia".into(), "Idrija, Slovenia".into(), 1));
            // The info panel reads both names and where they came from.
            let d = get_asset(c, ids[0], false)?;
            assert_eq!((d.place_name.as_deref(), d.locality.as_deref(), d.place_source), (Some("Idrija, Slovenia"), Some("Idrija, Slovenia"), 1));
            let none = get_asset(c, ids[2], false)?;
            assert_eq!((none.place_name, none.locality, none.place_source), (None, None, 0));
            let hits = |q: &str| -> Result<i64> {
                Ok(c.query_row("SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1", [q], |r| r.get(0))?)
            };
            assert_eq!(hits("Idrija")?, 2);
            assert_eq!(hits("Gorizia")?, 0, "searching Gorizia never returns Idrija");

            // A position found later hands the row back to the geocoders.
            assert!(fill_missing_gps(c, ids[2], 45.9557, 13.6379, None)?);
            c.execute("UPDATE assets SET gps_lat = NULL WHERE id = ?1", [ids[1]])?;
            assert!(fill_missing_gps(c, ids[1], 45.9557, 13.6379, None)?);
            let sources: Vec<i64> = c
                .prepare("SELECT place_source FROM assets ORDER BY id")?
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(sources, vec![1, 0, 0]);
            Ok(())
        })
        .unwrap();
    }

    /// The orientation backfill (round 3): photos whose decoder already turns them were
    /// stored with the file's tag, which was then applied a second time.
    #[test]
    fn the_orientation_backfill_resets_owed_turns_once() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let mut upside = asset("h1", "IMG_1.HEIC", "heic", Some(1), 0, 1);
            upside.orientation = 3;
            let mut jpeg = asset("j1", "IMG_2.JPG", "jpg", Some(2), 0, 1);
            jpeg.orientation = 3;
            let ids = insert_assets(c, &[upside, jpeg, asset("h2", "IMG_3.HEIC", "heic", Some(3), 0, 1)])?;
            c.execute("UPDATE assets SET thumbhash = 'x'", [])?;
            c.execute("INSERT INTO face_scan (asset_id, model) VALUES (?1, 'm')", [ids[0]])?;
            // The caller decides which extensions owe nothing; the candidates are every row
            // with a turn stored.
            let rows = oriented_rows(c)?;
            assert_eq!(rows, vec![(ids[0], "heic".to_string()), (ids[1], "jpg".to_string())]);
            assert_eq!(reset_orientation(c, &[ids[0]])?, 1);
            let (o, th): (i64, Option<String>) =
                c.query_row("SELECT orientation, thumbhash FROM assets WHERE id = ?1", [ids[0]], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((o, th), (1, None), "upright, and queued to be derived again");
            let scans: i64 = c.query_row("SELECT COUNT(*) FROM face_scan", [], |r| r.get(0))?;
            assert_eq!(scans, 0, "faces are looked for again in the upright pixels");
            // The JPEG was not reset; running it again finds only the JPEG.
            assert_eq!(oriented_rows(c)?, vec![(ids[1], "jpg".to_string())]);
            assert_eq!(reset_orientation(c, &[ids[0]])?, 0, "idempotent");
            Ok(())
        })
        .unwrap();
    }

    /// A file whose container says otherwise (a movie saved as `.JPG`, a HEIF saved as `.MOV`)
    /// is filed by what it is; a Live Photo's still keeps its type.
    #[test]
    fn a_container_that_disagrees_with_the_name_refiles_the_row() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(
                c,
                &[asset("m1", "IMG_1730.JPG", "jpg", Some(1), 0, 1), asset("h1", "IMG_2028.MOV", "mov", Some(2), 1, 1), asset("l1", "IMG_3.HEIC", "heic", Some(3), 2, 1)],
            )?;
            let row = |id: i64| -> Result<(String, i64)> { Ok(c.query_row("SELECT extension, media_type FROM assets WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?) };
            set_container(c, ids[0], "mp4", true)?;
            assert_eq!(row(ids[0])?, ("mp4".to_string(), 1), "a movie saved as .JPG becomes a video");
            set_container(c, ids[1], "heic", false)?;
            assert_eq!(row(ids[1])?, ("heic".to_string(), 0), "a HEIF saved as .MOV goes back to a photo");
            set_container(c, ids[2], "heic", false)?;
            assert_eq!(row(ids[2])?, ("heic".to_string(), 2), "a Live Photo's still stays one");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn video_prep_pages_through_live_videos_only() {
        // Round 9A: the background video prep walks every video in the library, a page at a
        // time by id, skipping photos and trashed videos.
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(
                c,
                &[
                    asset("v1", "A.MOV", "mov", Some(10), 1, 2_000),
                    asset("p1", "B.JPG", "jpg", Some(20), 0, 10),
                    asset("v2", "C.MKV", "mkv", None, 1, 5_000_000_000),
                    asset("v3", "D.MOV", "mov", Some(30), 1, 7),
                ],
            )?;
            trash_assets(c, &ids[3..], 1)?;
            c.execute("UPDATE assets SET metadata_json = '{\"probe\":{}}' WHERE id = ?1", [ids[0]])?;
            let first = video_prep_rows(c, None, 1)?;
            assert_eq!(first.len(), 1);
            assert_eq!((first[0].id, first[0].byte_size, first[0].captured_at), (ids[2], 5_000_000_000, None));
            let rest = video_prep_rows(c, Some(first[0].id), 10)?;
            assert_eq!(rest.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[0]]);
            assert_eq!(rest[0].content_hash, "v1");
            assert_eq!(rest[0].relative_path, "2026/09/17/v1_A.MOV");
            assert_eq!(rest[0].captured_at, Some(10));
            assert!(rest[0].metadata_json.as_deref().unwrap().contains("probe"));
            assert!(video_prep_rows(c, Some(ids[0]), 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn finds_the_photos_holding_exported_bytes() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("a", "A.jpg", "jpg", Some(500), 0, 10), asset("b", "B.jpg", "jpg", Some(500), 0, 20), asset("t", "T.jpg", "jpg", Some(500), 0, 30)])?;
            trash_assets(c, &ids[2..], 1)?;
            let hashes: Vec<String> = ["b", "zzz", "a", "b", "t"].iter().map(|s| s.to_string()).collect();
            assert_eq!(ids_by_hash(c, &hashes)?, vec![ids[1], ids[0]], "unknown and trashed skipped, each once");
            Ok(())
        })
        .unwrap();
    }

    /// Round 7: photos read before the software and exposure details are found, read again,
    /// and only those keys are merged in; everything else in the blob stays.
    #[test]
    fn older_metadata_is_found_and_merged_into_not_replaced() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("a", "A.jpg", "jpg", Some(500), 0, 10), asset("b", "B.jpg", "jpg", Some(500), 0, 20)])?;
            c.execute("UPDATE assets SET metadata_json = '{\"model\":\"iPhone 12\",\"iso\":32}' WHERE id = ?1", [ids[0]])?;
            c.execute("UPDATE assets SET metadata_json = '{\"model\":\"X\",\"exif\":2}' WHERE id = ?1", [ids[1]])?;
            let todo = list_metadata_before(c, 2, 10)?;
            assert_eq!(todo.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![ids[0]]);
            merge_metadata(c, ids[0], &serde_json::json!({ "software": "17.4.1", "exif": 2 }))?;
            let json: String = c.query_row("SELECT metadata_json FROM assets WHERE id = ?1", [ids[0]], |r| r.get(0))?;
            let v: serde_json::Value = serde_json::from_str(&json)?;
            assert_eq!((v["model"].as_str(), v["iso"].as_i64(), v["software"].as_str()), (Some("iPhone 12"), Some(32), Some("17.4.1")));
            assert!(list_metadata_before(c, 2, 10)?.is_empty(), "done once");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_photo_in_the_trash_still_shows_its_picture_and_details() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("t", "T.jpg", "jpg", Some(500), 0, 10)])?;
            trash_assets(c, &ids, 1)?;
            assert_eq!(get_asset(c, ids[0], false)?.filename, "T.jpg");
            assert_eq!(render_info(c, ids[0])?.content_hash, "t");
            Ok(())
        })
        .unwrap();
    }

    /// Round 6: thumbnails show the library's edit; an export renders Develop's draft.
    #[test]
    fn an_export_renders_the_develop_draft_and_thumbnails_do_not() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("d", "D.jpg", "jpg", Some(500), 0, 10)])?;
            let mut develop = crate::edits::Develop::default();
            develop.basic.exposure = 0.5;
            let draft = crate::edits::Recipe { develop: Some(develop), ..crate::edits::Recipe::identity() };
            crate::edits::save_draft(c, ids[0], &draft, 1)?;
            assert!(render_info(c, ids[0])?.edit.is_none(), "the library shows the original");
            assert_eq!(export_render_info(c, ids[0])?.edit.map(|(r, _)| r.basic().exposure), Some(0.5));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_export_added_to_the_library_remembers_its_source() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(c, &[asset("src1", "DSC_1.NEF", "nef", Some(500), 0, 10), asset("exp1", "DSC_1-edited.jpg", "jpg", Some(500), 0, 20)])?;
            assert_eq!(get_asset(c, ids[1], false)?.derived_from, None);
            assert!(link_derived(c, "exp1", ids[0])?);
            assert_eq!(get_asset(c, ids[1], false)?.derived_from, Some(DerivedFrom { id: ids[0], filename: "DSC_1.NEF".into() }));
            // A second export of the same bytes does not move the link, and nothing is its own source.
            assert!(!link_derived(c, "exp1", 999)?);
            assert!(!link_derived(c, "src1", ids[0])?);
            // A trashed source is not offered; a deleted one is forgotten.
            trash_assets(c, &ids[..1], 1)?;
            assert_eq!(get_asset(c, ids[1], false)?.derived_from, None);
            c.execute("DELETE FROM assets WHERE id = ?1", [ids[0]])?;
            let left: Option<i64> = c.query_row("SELECT derived_from FROM assets WHERE id = ?1", [ids[1]], |r| r.get(0))?;
            assert_eq!(left, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn insert_find_get_and_stats() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(
                c,
                &[
                    asset("aaaa", "a.jpg", "jpg", Some(500), 0, 10),
                    asset("bbbb", "b.heic", "heic", None, 0, 20),
                    asset("cccc", "c.mov", "mov", Some(900), 1, 30),
                ],
            )?;
            assert_eq!(ids.len(), 3);
            assert_eq!(find_by_hash(c, "bbbb")?, Some(ids[1]));
            assert_eq!(find_by_hash(c, "zzzz")?, None);
            let d = get_asset(c, ids[1], false)?;
            assert_eq!(d.filename, "b.heic");
            assert!(!d.renderable);
            assert!(get_asset(c, ids[0], false)?.renderable);
            assert!(get_asset(c, ids[1], true)?.renderable, "video renders once FFmpeg is present");
            assert!(matches!(get_asset(c, 999, false), Err(CoreError::NotFound(_))));
            assert_eq!(list_underived(c, 10)?, ids, "nothing derived yet");
            assert_eq!(set_rating(c, &ids[..2], 4)?, 2);
            assert_eq!(set_rating(c, &ids[..1], 99)?, 1, "clamped, not rejected");
            assert_eq!(get_asset(c, ids[0], false)?.rating, 5);
            assert_eq!(get_asset(c, ids[1], false)?.rating, 4);
            assert_eq!(set_rating(c, &[], 3)?, 0);
            let dir = tempfile::tempdir().unwrap();
            let first = unique_export_name(dir.path(), "IMG_1.jpg");
            assert_eq!(first.file_name().unwrap(), "IMG_1.jpg");
            std::fs::write(&first, b"x").unwrap();
            assert_eq!(unique_export_name(dir.path(), "IMG_1.jpg").file_name().unwrap(), "IMG_1 (2).jpg");
            assert_eq!(unique_export_name(dir.path(), "noext").file_name().unwrap(), "noext");
            assert_eq!(export_paths(c, &[])?.len(), 0);
            assert_eq!(export_paths(c, &ids)?.len(), 3);
            trash_assets(c, &ids[..1], 1)?;
            assert_eq!(export_paths(c, &ids)?.len(), 2, "trashed rows are skipped");
            restore_assets(c, &ids[..1])?;
            let s = stats(c)?;
            assert_eq!((s.items, s.videos, s.bytes), (3, 1, 60));
            set_derived(c, ids[0], &[1, 2, 3], "#112233", Some(640), Some(480))?;
            let (w, th): (i64, Vec<u8>) =
                c.query_row("SELECT width, thumbhash FROM assets WHERE id = ?1", [ids[0]], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((w, th), (640, vec![1, 2, 3]));
            let info = render_info(c, ids[0])?;
            assert_eq!((info.content_hash.as_str(), info.extension.as_str(), info.media_type, info.orientation), ("aaaa", "jpg", 0, 1));
            assert!(info.relative_path.ends_with("a.jpg"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn favorites_trash_restore_and_purge() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = crate::Library::create(tmp.path()).unwrap();
        let ids = lib
            .db
            .with_conn(|c| {
                insert_assets(
                    c,
                    &[
                        asset("aaaa", "a.jpg", "jpg", Some(500), 0, 10),
                        asset("bbbb", "b.jpg", "jpg", Some(600), 0, 10),
                        asset("cccc", "c.jpg", "jpg", Some(700), 0, 10),
                    ],
                )
            })
            .unwrap();
        for (h, name) in [("aaaa", "a.jpg"), ("bbbb", "b.jpg"), ("cccc", "c.jpg")] {
            let p = lib.originals_dir().join(format!("2026/09/17/{h}_{name}"));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        }
        lib.db
            .with_conn(|c| {
                assert_eq!(set_favorite(c, &ids[..2], true)?, 2);
                assert_eq!(favorite_ids(c, 10)?, vec![ids[1], ids[0]]);
                assert_eq!(trash_assets(c, &[ids[0], ids[1]], 1_000)?, 2);
                assert_eq!(stats(c)?.items, 1, "trashed items leave the library count");
                assert_eq!(get_asset(c, ids[0], false)?.id, ids[0], "the Trash still shows it");
                assert_eq!(restore_assets(c, &[ids[1]])?, 1);
                assert_eq!(list_expired(c, 999)?.len(), 0);
                assert_eq!(list_expired(c, 1_000)?.len(), 1);
                Ok(())
            })
            .unwrap();
        let deleted_hashes = std::cell::RefCell::new(Vec::new());
        let n = purge_expired(&lib, 1_000, &|h| deleted_hashes.borrow_mut().push(h.to_string())).unwrap();
        assert_eq!(n, 1);
        assert_eq!(*deleted_hashes.borrow(), vec!["aaaa".to_string()]);
        assert!(!lib.originals_dir().join("2026/09/17/aaaa_a.jpg").exists());
        let remaining: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
        assert_eq!(remaining, 2);
        // Delete now for a specific trashed id, and empty trash for the rest.
        lib.db.with_conn(|c| trash_assets(c, &[ids[1], ids[2]], 2_000)).unwrap();
        assert_eq!(delete_permanently(&lib, Some(&[ids[1]]), &|_| {}).unwrap(), 1);
        assert_eq!(delete_permanently(&lib, None, &|_| {}).unwrap(), 1);
        let remaining: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn neighbors_follow_the_timeline_with_null_capture_times() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Timeline (newest first): c (900), b (NULL → imported 1000 → newest!), a (500)
            // so order is: b (1000), c (900), a (500)
            let ids = insert_assets(
                c,
                &[
                    asset("aaaa", "a.jpg", "jpg", Some(500), 0, 1),
                    asset("bbbb", "b.jpg", "jpg", None, 0, 1),
                    asset("cccc", "c.jpg", "jpg", Some(900), 0, 1),
                ],
            )?;
            let (a, b, cc) = (ids[0], ids[1], ids[2]);
            assert_eq!(neighbors(c, b)?, Neighbors { prev: None, next: Some(cc) });
            assert_eq!(neighbors(c, cc)?, Neighbors { prev: Some(b), next: Some(a) });
            assert_eq!(neighbors(c, a)?, Neighbors { prev: Some(cc), next: None });
            Ok(())
        })
        .unwrap();
    }

    /// Paging the viewer past a Live Photo landed on its `.MOV` half, shown as a video.
    #[test]
    fn neighbors_skip_a_live_photos_movie_half() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = insert_assets(
                c,
                &[
                    asset("aaaa", "a.jpg", "jpg", Some(500), 0, 1),
                    asset("bbbb", "IMG_1.heic", "heic", Some(700), 0, 1),
                    asset("cccc", "IMG_1.mov", "mov", Some(700), 1, 1),
                    asset("dddd", "d.jpg", "jpg", Some(900), 0, 1),
                ],
            )?;
            c.execute("UPDATE assets SET live_photo_group = 'img_1' WHERE id IN (?1, ?2)", [ids[1], ids[2]])?;
            assert_eq!(neighbors(c, ids[1])?, Neighbors { prev: Some(ids[3]), next: Some(ids[0]) });
            assert_eq!(neighbors(c, ids[3])?.next, Some(ids[1]));
            assert_eq!(neighbors(c, ids[0])?.prev, Some(ids[1]));
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod live_photo_tests {
    use super::*;
    use crate::Db;

    fn insert(c: &Connection, name: &str, media_type: i64, batch: i64) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, import_batch_id)
             VALUES (?1, ?1, ?2, ?3, ?4, 1, 1, 1, ?5)",
            params![format!("h-{name}"), name, name.rsplit('.').next().unwrap(), media_type, batch],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    #[test]
    fn a_date_carries_its_source_and_the_unsourced_are_listed() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, file_modified_at)
                 VALUES (1, 'h1', '2026/09/19/h1_a.jpg', 'a.jpg', 'jpg', 0, 1, 1, 100, 100, 90), (2, 'h2', 'b.jpg', 'b.jpg', 'jpg', 0, 1, 1, 100, 100, NULL)",
                [],
            )?;
            c.execute("UPDATE assets SET capture_source = 'original' WHERE id = 2", [])?;

            let todo = list_unsourced_dates(c, 10)?;
            assert_eq!(todo, vec![(1, "2026/09/19/h1_a.jpg".to_string(), Some(90))]);

            set_capture_time(c, 1, 1_700_000_000, Some(1_699_992_800), Some(120), "digitized")?;
            let row: (i64, Option<i64>, Option<i64>, String) = c.query_row(
                "SELECT captured_at, captured_at_utc, tz_offset_minutes, capture_source FROM assets WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            assert_eq!(row, (1_700_000_000, Some(1_699_992_800), Some(120), "digitized".to_string()));
            assert!(list_unsourced_dates(c, 10)?.is_empty(), "a sourced row is done");
            assert_eq!(captured_at_of(c, 1)?, Some(1_700_000_000));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn same_stem_in_two_folders_pairs_within_each_folder_and_across_batches() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for id in [1, 2] {
                c.execute("INSERT INTO import_batches (id, source_kind, source_label, started_at, state) VALUES (?1, 0, 'test', 1, 1)", [id])?;
            }
            let at = |c: &Connection, id: i64, dir: &str, t: i64| {
                set_source_dir(c, id, dir).unwrap();
                c.execute("UPDATE assets SET captured_at = ?2 WHERE id = ?1", [id, t]).unwrap();
            };
            // Two folders, the same stems: each still pairs with the movie of its own folder.
            let (a1, a2) = (insert(c, "A_IMG_1.HEIC", 0, 1), insert(c, "A_IMG_1.MOV", 1, 1));
            let (b1, b2) = (insert(c, "B_IMG_1.HEIC", 0, 1), insert(c, "B_IMG_1.MOV", 1, 1));
            c.execute("UPDATE assets SET filename = 'IMG_1.HEIC' WHERE id IN (?1, ?2)", [a1, b1])?;
            c.execute("UPDATE assets SET filename = 'IMG_1.MOV' WHERE id IN (?1, ?2)", [a2, b2])?;
            at(c, a1, "Trip/A", 1000);
            at(c, a2, "Trip/A", 1000);
            at(c, b1, "Trip/B", 5000);
            at(c, b2, "Trip/B", 5000);
            // The movie came in an earlier batch than its still.
            let late = insert(c, "IMG_9.HEIC", 0, 2);
            let early = insert(c, "E_IMG_9.MOV", 1, 1);
            c.execute("UPDATE assets SET filename = 'IMG_9.MOV' WHERE id = ?1", [early])?;
            at(c, late, "DCIM/100", 9000);
            at(c, early, "DCIM/100", 9001);
            // The same name in a different folder and minutes away is not its partner.
            let stranger = insert(c, "S_IMG_9.MOV", 1, 1);
            c.execute("UPDATE assets SET filename = 'IMG_9.MOV' WHERE id = ?1", [stranger])?;
            at(c, stranger, "DCIM/101", 9001);

            // Batch 2 brought the still after the movie of batch 1 was catalogued: it finds it.
            assert_eq!(pair_live_photos(c, 2)?, 1);
            assert_eq!(pair_live_photos(c, 1)?, 2, "A and B pair, each within its folder");
            let group = |id: i64| -> Option<String> { c.query_row("SELECT live_photo_group FROM assets WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
            assert_eq!(live_photo_mov(c, a1)?, Some(a2));
            assert_eq!(live_photo_mov(c, b1)?, Some(b2), "no cross-pairing between the folders");
            assert_ne!(group(a1), group(b1), "the two pairs are two groups");
            assert_eq!(live_photo_mov(c, late)?, Some(early));
            assert_eq!(group(stranger), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn pairs_stills_with_their_movs_inside_one_batch() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for id in [7, 8, 9] {
                c.execute(
                    "INSERT INTO import_batches (id, source_kind, source_label, started_at, state) VALUES (?1, 0, 'test', 1, 1)",
                    [id],
                )?;
            }
            let still = insert(c, "IMG_0001.HEIC", 0, 7);
            let mov = insert(c, "IMG_0001.MOV", 1, 7);
            let lonely = insert(c, "IMG_0002.HEIC", 0, 7);
            let clip = insert(c, "IMG_0003.MOV", 1, 7);
            let other_batch = insert(c, "IMG_0004.HEIC", 0, 8);
            insert(c, "IMG_0004.MOV", 1, 9);

            assert_eq!(pair_live_photos(c, 7)?, 1);
            let group = |id: i64| -> Option<String> {
                c.query_row("SELECT live_photo_group FROM assets WHERE id = ?1", [id], |r| r.get(0)).unwrap()
            };
            let kind = |id: i64| -> i64 { c.query_row("SELECT media_type FROM assets WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
            assert_eq!(group(still).as_deref(), Some(format!("img_0001#{still}").as_str()));
            assert_eq!(group(mov), group(still));
            assert_eq!(kind(still), MEDIA_TYPE_LIVE);
            assert_eq!(kind(mov), 1, "the MOV half stays a video row");
            assert_eq!(group(lonely), None);
            assert_eq!(group(clip), None);
            assert_eq!(group(other_batch), None, "pairs never cross batches");
            assert_eq!(live_photo_mov(c, still)?, Some(mov));
            assert_eq!(live_photo_mov(c, lonely)?, None);
            assert_eq!(pair_live_photos(c, 7)?, 0, "idempotent");
            let detail = get_asset(c, still, true)?;
            assert_eq!(detail.live_video_id, Some(mov), "the viewer finds the movie through the still");
            assert!(!detail.raw);
            Ok(())
        })
        .unwrap();
    }

    fn batch(c: &Connection, id: i64) {
        c.execute("INSERT INTO import_batches (id, source_kind, source_label, started_at, state) VALUES (?1, 0, 'test', 1, 1)", [id])
            .unwrap();
    }

    fn state(c: &Connection, id: i64) -> i64 {
        c.query_row("SELECT state FROM assets WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    /// A Live Photo's movie is only ever reached through its still. Trashing the still and
    /// leaving the movie active kept the phone's device section alive with nothing in it.
    #[test]
    fn the_movie_half_follows_its_still_through_trash_restore_delete_and_copy() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            batch(c, 1);
            let still = insert(c, "IMG_0001.HEIC", 0, 1);
            let mov = insert(c, "IMG_0001.MOV", 1, 1);
            let plain = insert(c, "IMG_0002.JPG", 0, 1);
            pair_live_photos(c, 1)?;

            assert_eq!(export_paths(c, &[still])?.len(), 2, "copying a Live Photo copies its movie too");

            trash_assets(c, &[still, plain], 10)?;
            assert_eq!(state(c, mov), 1);
            let listed: Vec<i64> = list_trashed(c, Some(&[still]))?.into_iter().map(|(id, _, _)| id).collect();
            assert!(listed.contains(&mov), "delete now removes both files: {listed:?}");

            restore_assets(c, &[still])?;
            assert_eq!(state(c, mov), 0);
            assert_eq!(state(c, plain), 1, "only what was asked for comes back");
            Ok(())
        })
        .unwrap();
    }

    /// Apple's movie half is about three seconds. A long clip that happens to share a
    /// stem (IMG_5992.MOV next to an unrelated IMG_5992.HEIC) is a video in its own right.
    #[test]
    fn a_long_video_is_never_taken_for_a_live_photo_movie() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            batch(c, 1);
            insert(c, "IMG_5992.HEIC", 0, 1);
            let clip = insert(c, "IMG_5992.MOV", 1, 1);
            c.execute("UPDATE assets SET duration_ms = 19800 WHERE id = ?1", [clip])?;
            assert_eq!(pair_live_photos(c, 1)?, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn stats_count_a_live_photo_once() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            batch(c, 1);
            insert(c, "IMG_0001.HEIC", 0, 1);
            insert(c, "IMG_0001.MOV", 1, 1);
            pair_live_photos(c, 1)?;
            let s = stats(c)?;
            assert_eq!((s.items, s.videos), (1, 0));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn quality_is_stored_with_every_bit_of_the_hash() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 2, 3)?;
            set_quality(c, 1, 12.5, u64::MAX - 3)?;
            let (s, h): (f64, i64) = c.query_row("SELECT sharpness, phash FROM assets WHERE id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((s, h as u64), (12.5, u64::MAX - 3));
            c.execute("UPDATE assets SET thumbhash = x'00'", [])?;
            let missing: Vec<i64> = list_missing_quality(c, 10)?.into_iter().map(|(id, _)| id).collect();
            assert_eq!(missing, vec![2]);
            Ok(())
        })
        .unwrap();
    }

}
