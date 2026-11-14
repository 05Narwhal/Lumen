//! Manual albums (spec §11). Membership is an ordered list with fractional positions,
//! so a reorder is one UPDATE. Removing from an album never touches files.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::AssetQuery;
use crate::scope::LayoutScope;
use crate::{CoreError, Result};

pub const KIND_MANUAL: i64 = 0;
pub const KIND_SMART: i64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AlbumSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub kind: i64,
    #[ts(type = "number | null")]
    pub parent_id: Option<i64>,
    /// Explicit cover, else the first item by position.
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
    #[ts(type = "number")]
    pub item_count: i64,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    /// Smart albums only: the saved query that defines their contents.
    #[serde(default)]
    pub query: Option<AssetQuery>,
    /// Its photos are hidden from the rest of the library (round 3).
    #[serde(default)]
    pub hides_items: bool,
    /// The collection it is in (Round 5A, `album_collections`).
    #[serde(default)]
    #[ts(type = "number | null")]
    pub collection_id: Option<i64>,
    /// A shape with an icon shown instead of its photos (migration 0034); `None` shows photos.
    #[serde(default)]
    pub cover_style: Option<CoverStyle>,
    /// The cover first, then up to two more of its photos, for the fanned stack of prints.
    #[serde(default)]
    #[ts(type = "Array<number>")]
    pub preview_ids: Vec<i64>,
}

/// The big shape an icon cover is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum CoverShape {
    Folder,
    Book,
    Box,
    Binder,
}

/// How an album or collection looks when it does not show its photos (Julian's 2026-11-08 list,
/// item 3). Stored as JSON in `albums.cover_style` / `album_collections.cover_style`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[ts(export)]
pub enum CoverStyle {
    /// `icon` is a Tabler icon name (`"plane"`); `color` is `"swatch-1"`…`"swatch-10"` (a
    /// token in `globals.scss`) or a custom `"#rrggbb"`.
    Icon { shape: CoverShape, icon: String, color: String },
}

impl CoverStyle {
    pub fn validate(&self) -> Result<()> {
        let CoverStyle::Icon { icon, color, .. } = self;
        let icon_ok = !icon.is_empty() && icon.len() <= 40 && icon.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        let swatch = color.strip_prefix("swatch-").and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=10).contains(&n));
        let hex = color.strip_prefix('#').is_some_and(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()));
        if !icon_ok {
            return Err(CoreError::Invalid(format!("unknown cover icon {icon:?}")));
        }
        if !swatch && !hex {
            return Err(CoreError::Invalid(format!("unknown cover colour {color:?}")));
        }
        Ok(())
    }

    pub(crate) fn to_json(style: Option<&CoverStyle>) -> Result<Option<String>> {
        match style {
            Some(s) => {
                s.validate()?;
                Ok(Some(serde_json::to_string(s)?))
            }
            None => Ok(None),
        }
    }

    /// An unreadable value shows photos rather than failing the whole list.
    pub(crate) fn from_json(json: Option<String>) -> Option<CoverStyle> {
        json.and_then(|j| serde_json::from_str(&j).ok())
    }
}

/// What counts as one of an album's photos everywhere it is shown: in the library, not in the
/// vault, not hidden by another album (unless this one hides them), not the video half of a Live
/// Photo. Needs the album as `a` and the asset as `x`.
pub(crate) const SHOWN: &str = "x.state = 0 AND x.in_vault = 0 AND ((a.hides_items = 1 AND x.unreadable = 0) OR x.concealed = 0)
                     AND NOT (x.media_type = 1 AND x.live_photo_group IS NOT NULL)";

pub fn create_album(conn: &Connection, name: &str, parent_id: Option<i64>, now: i64) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidPath("album name is empty".into()));
    }
    conn.execute(
        "INSERT INTO albums (name, kind, parent_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![name, KIND_MANUAL, parent_id, now],
    )?;
    Ok(conn.last_insert_rowid())
}

/// A smart album is a saved [`AssetQuery`]; it owns no `album_items` rows and its
/// contents change as the library does.
pub fn create_smart_album(conn: &Connection, name: &str, query: &AssetQuery, now: i64) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidPath("album name is empty".into()));
    }
    conn.execute(
        "INSERT INTO albums (name, kind, query_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        params![name, KIND_SMART, serde_json::to_string(query)?, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_smart_album(conn: &Connection, id: i64, name: &str, query: &AssetQuery, now: i64) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::InvalidPath("album name is empty".into()));
    }
    let n = conn.execute(
        "UPDATE albums SET name = ?2, query_json = ?3, updated_at = ?4 WHERE id = ?1 AND kind = ?5",
        params![id, name, serde_json::to_string(query)?, now, KIND_SMART],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("smart album {id}")));
    }
    Ok(())
}

/// The saved query, or `None` for a manual album (or an unreadable one).
pub fn smart_query(conn: &Connection, id: i64) -> Result<Option<AssetQuery>> {
    let json: Option<String> = conn
        .query_row("SELECT query_json FROM albums WHERE id = ?1 AND kind = ?2", params![id, KIND_SMART], |r| r.get(0))
        .optional()?
        .flatten();
    Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
}

/// The scope that renders a smart album's contents.
pub fn smart_scope(query: AssetQuery) -> LayoutScope {
    LayoutScope { query: Some(query), ..Default::default() }
}

pub fn rename_album(conn: &Connection, id: i64, name: &str, now: i64) -> Result<()> {
    let n = conn.execute("UPDATE albums SET name = ?2, updated_at = ?3 WHERE id = ?1", params![id, name.trim(), now])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("album {id}")));
    }
    Ok(())
}

pub fn delete_album(conn: &Connection, id: i64) -> Result<()> {
    let n = conn.execute("DELETE FROM albums WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("album {id}")));
    }
    Ok(())
}

/// Every album with its count and up to three prints (`album_covers`): picked once and cached
/// until its photos change, so a list with nothing changed only reads.
pub fn list_albums(conn: &Connection) -> Result<Vec<AlbumSummary>> {
    type Extra = (bool, i64, Option<(String, String)>);
    let mut stmt = conn.prepare(
        "SELECT a.id, a.name, a.kind, a.parent_id, a.cover_asset_id, a.created_at, a.updated_at, a.query_json,
                a.hides_items, a.collection_id, a.cover_style, a.cover_picked, a.cover_seed, a.preview_json, a.preview_key
         FROM albums a ORDER BY a.updated_at DESC, a.id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        let json: Option<String> = r.get(7)?;
        let cached = match (r.get::<_, Option<String>>(13)?, r.get::<_, Option<String>>(14)?) {
            (Some(j), Some(k)) => Some((j, k)),
            _ => None,
        };
        let extra: Extra = (r.get::<_, i64>(11)? != 0, r.get(12)?, cached);
        Ok((
            AlbumSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                parent_id: r.get(3)?,
                cover_asset_id: r.get(4)?,
                item_count: 0,
                created_at: r.get(5)?,
                updated_at: r.get(6)?,
                query: json.and_then(|j| serde_json::from_str(&j).ok()),
                hides_items: r.get::<_, i64>(8)? != 0,
                collection_id: r.get(9)?,
                cover_style: CoverStyle::from_json(r.get(10)?),
                preview_ids: Vec::new(),
            },
            extra,
        ))
    })?;
    let rows = rows.collect::<std::result::Result<Vec<(AlbumSummary, Extra)>, _>>()?;
    let mut albums = Vec::with_capacity(rows.len());
    for (mut a, (picked, seed, cached)) in rows {
        crate::album_covers::refresh(conn, &mut a, picked, seed, cached)?;
        albums.push(a);
    }
    Ok(albums)
}

/// A new set of prints: a new seed, and a hand-picked cover forgotten (*Shuffle cover*).
pub fn shuffle_cover(conn: &Connection, id: i64) -> Result<()> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(1);
    let n = conn.execute(
        "UPDATE albums SET cover_seed = CASE WHEN cover_seed = ?2 THEN ?2 + 1 ELSE ?2 END, cover_picked = 0, preview_key = NULL WHERE id = ?1",
        params![id, nanos],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("album {id}")));
    }
    Ok(())
}

/// The newest `n` assets a scope selects: a smart album's cover and the prints behind it.
pub(crate) fn first_matches(conn: &Connection, scope: &LayoutScope, n: usize) -> Result<Vec<i64>> {
    let (from_where, order, params) = scope.sql();
    let mut stmt = conn.prepare(&format!("SELECT a.id {from_where} ORDER BY {order} LIMIT {n}"))?;
    let ids = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(ids)
}

pub fn get_album(conn: &Connection, id: i64) -> Result<AlbumSummary> {
    list_albums(conn)?.into_iter().find(|a| a.id == id).ok_or_else(|| CoreError::NotFound(format!("album {id}")))
}

/// Appends after the last position; ids already in the album are skipped. Returns how many were added.
pub fn add_to_album(conn: &Connection, album_id: i64, asset_ids: &[i64], now: i64) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut pos: f64 = tx
        .query_row("SELECT COALESCE(MAX(position), 0) FROM album_items WHERE album_id = ?1", [album_id], |r| r.get(0))
        .optional()?
        .unwrap_or(0.0);
    let mut added = 0;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO album_items (album_id, asset_id, position, added_at) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for &id in asset_ids {
            pos += 1.0;
            added += stmt.execute(params![album_id, id, pos, now])?;
        }
    }
    tx.execute("UPDATE albums SET updated_at = ?2 WHERE id = ?1", params![album_id, now])?;
    tx.commit()?;
    Ok(added)
}

pub fn remove_from_album(conn: &Connection, album_id: i64, asset_ids: &[i64]) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut removed = 0;
    {
        let mut stmt = tx.prepare_cached("DELETE FROM album_items WHERE album_id = ?1 AND asset_id = ?2")?;
        for &id in asset_ids {
            removed += stmt.execute(params![album_id, id])?;
        }
    }
    // A cover that leaves the album leaves its cover too; the next list picks automatically.
    let cover: Option<i64> = tx.query_row("SELECT cover_asset_id FROM albums WHERE id = ?1", [album_id], |r| r.get(0)).optional()?.flatten();
    if cover.is_some_and(|c| asset_ids.contains(&c)) {
        tx.execute("UPDATE albums SET cover_asset_id = NULL, cover_picked = 0 WHERE id = ?1", [album_id])?;
    }
    tx.commit()?;
    Ok(removed)
}

/// Move `asset_id` before `before` (or to the end when `None`). Positions are renumbered
/// 1..n when neighbours get too close to bisect.
pub fn move_in_album(conn: &Connection, album_id: i64, asset_id: i64, before: Option<i64>) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let new_pos: f64 = match before {
        None => {
            let max: f64 = tx.query_row(
                "SELECT COALESCE(MAX(position), 0) FROM album_items WHERE album_id = ?1 AND asset_id != ?2",
                params![album_id, asset_id],
                |r| r.get(0),
            )?;
            max + 1.0
        }
        Some(b) => {
            let bpos: f64 = tx
                .query_row("SELECT position FROM album_items WHERE album_id = ?1 AND asset_id = ?2", params![album_id, b], |r| r.get(0))
                .map_err(|_| CoreError::NotFound(format!("asset {b} in album {album_id}")))?;
            let prev: Option<f64> = tx
                .query_row(
                    "SELECT MAX(position) FROM album_items WHERE album_id = ?1 AND position < ?2 AND asset_id != ?3",
                    params![album_id, bpos, asset_id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            let lo = prev.unwrap_or(bpos - 2.0);
            let mid = (lo + bpos) / 2.0;
            if bpos - lo < 1e-6 {
                renumber(&tx, album_id)?;
                tx.commit()?;
                return move_in_album(conn, album_id, asset_id, before);
            }
            mid
        }
    };
    let n = tx.execute(
        "UPDATE album_items SET position = ?3 WHERE album_id = ?1 AND asset_id = ?2",
        params![album_id, asset_id, new_pos],
    )?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("asset {asset_id} in album {album_id}")));
    }
    tx.commit()?;
    Ok(())
}

fn renumber(conn: &Connection, album_id: i64) -> Result<()> {
    let mut stmt = conn.prepare("SELECT asset_id FROM album_items WHERE album_id = ?1 ORDER BY position, asset_id")?;
    let ids: Vec<i64> = stmt.query_map([album_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    let mut upd = conn.prepare("UPDATE album_items SET position = ?3 WHERE album_id = ?1 AND asset_id = ?2")?;
    for (i, id) in ids.iter().enumerate() {
        upd.execute(params![album_id, id, (i + 1) as f64])?;
    }
    Ok(())
}

/// A photo picked as the cover goes in front of the prints and also brings the photos back in
/// place of an icon cover; `None` goes back to an automatic pick.
pub fn set_cover(conn: &Connection, album_id: i64, asset_id: Option<i64>) -> Result<()> {
    conn.execute(
        "UPDATE albums SET cover_asset_id = ?2, cover_picked = (?2 IS NOT NULL),
                cover_style = CASE WHEN ?2 IS NULL THEN cover_style END WHERE id = ?1",
        params![album_id, asset_id],
    )?;
    Ok(())
}

/// An icon cover, or `None` for its photos again.
pub fn set_cover_style(conn: &Connection, album_id: i64, style: Option<&CoverStyle>) -> Result<()> {
    let json = CoverStyle::to_json(style)?;
    if conn.execute("UPDATE albums SET cover_style = ?2 WHERE id = ?1", params![album_id, json])? == 0 {
        return Err(CoreError::NotFound(format!("album {album_id}")));
    }
    Ok(())
}

/// A copy of album `id` (Round 4G's *Duplicate album*): the same photos in the same order,
/// the same cover and hiding, or for a smart album the same rules. Named "<name> copy", then
/// "<name> copy 2", … so it never shares a name with another album.
pub fn duplicate_album(conn: &Connection, id: i64, now: i64) -> Result<i64> {
    // A copy sits in the same collection as the album (Round 5A).
    type Row = (String, i64, Option<String>, Option<i64>, i64, Option<i64>, Option<String>, i64);
    let (name, kind, query_json, cover, hides, collection, style, picked): Row = conn.query_row(
        "SELECT name, kind, query_json, cover_asset_id, hides_items, collection_id, cover_style, cover_picked FROM albums WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
    )?;
    let taken = |n: &str| -> Result<bool> { Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM albums WHERE name = ?1)", [n], |r| r.get(0))?) };
    let mut copy = format!("{name} copy");
    let mut n = 2;
    while taken(&copy)? {
        copy = format!("{name} copy {n}");
        n += 1;
    }
    conn.execute(
        "INSERT INTO albums (name, kind, query_json, cover_asset_id, hides_items, collection_id, created_at, updated_at, cover_style, cover_picked)
         VALUES (?1, ?2, ?3, ?4, ?5, ?7, ?6, ?6, ?8, ?9)",
        params![copy, kind, query_json, cover, hides, now, collection, style, picked],
    )?;
    let new_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO album_items (album_id, asset_id, position, added_at) SELECT ?2, asset_id, position, ?3 FROM album_items WHERE album_id = ?1",
        params![id, new_id, now],
    )?;
    Ok(new_id)
}

pub fn album_item_ids(conn: &Connection, album_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT asset_id FROM album_items WHERE album_id = ?1 ORDER BY position, asset_id")?;
    let rows = stmt.query_map([album_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{insert_assets, NewAsset};
    use crate::Db;

    fn seed(conn: &Connection, n: usize) -> Vec<i64> {
        let rows: Vec<NewAsset> = (0..n)
            .map(|i| NewAsset {
                content_hash: format!("h{i}"),
                relative_path: format!("p/{i}.jpg"),
                filename: format!("{i}.jpg"),
                extension: "jpg".into(),
                byte_size: 1,
                orientation: 1,
                imported_at: 100 + i as i64,
                ..Default::default()
            })
            .collect();
        insert_assets(conn, &rows).unwrap()
    }

    #[test]
    fn crud_membership_and_order() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 5);
            let a = create_album(c, " Trip ", None, 10)?;
            assert!(create_album(c, "  ", None, 10).is_err());
            assert_eq!(add_to_album(c, a, &ids[..3], 11)?, 3);
            assert_eq!(add_to_album(c, a, &ids[..3], 12)?, 0, "duplicates are ignored");
            let list = list_albums(c)?;
            assert_eq!(list[0].name, "Trip");
            assert_eq!(list[0].item_count, 3);
            assert!(ids[..3].contains(&list[0].cover_asset_id.unwrap()), "one of its photos");
            rename_album(c, a, "Trip 2026", 13)?;
            assert_eq!(get_album(c, a)?.name, "Trip 2026");
            // Move the third before the first.
            move_in_album(c, a, ids[2], Some(ids[0]))?;
            assert_eq!(album_item_ids(c, a)?, vec![ids[2], ids[0], ids[1]]);
            move_in_album(c, a, ids[2], None)?;
            assert_eq!(album_item_ids(c, a)?, vec![ids[0], ids[1], ids[2]]);
            set_cover(c, a, Some(ids[1]))?;
            assert_eq!(get_album(c, a)?.cover_asset_id, Some(ids[1]));
            assert_eq!(remove_from_album(c, a, &[ids[1], 999])?, 1);
            let cover = get_album(c, a)?.cover_asset_id.unwrap();
            assert!(cover == ids[0] || cover == ids[2], "a cover that leaves is replaced by another of its photos");
            delete_album(c, a)?;
            assert!(get_album(c, a).is_err());
            let n: i64 = c.query_row("SELECT COUNT(*) FROM album_items", [], |r| r.get(0))?;
            assert_eq!(n, 0, "membership cascades");
            Ok(())
        })
        .unwrap();
    }

    /// Round 4G: *Duplicate album* copies the photos in their order, the cover and whether
    /// the album hides its photos, under a name that says it is a copy.
    #[test]
    fn duplicating_an_album_copies_its_photos_in_order() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 4);
            let a = create_album(c, "Lisbon", None, 1)?;
            add_to_album(c, a, &ids[..3], 1)?;
            move_in_album(c, a, ids[2], Some(ids[0]))?;
            set_cover(c, a, Some(ids[1]))?;
            let copy = duplicate_album(c, a, 5)?;
            assert_ne!(copy, a);
            let got = get_album(c, copy)?;
            assert_eq!(got.name, "Lisbon copy");
            assert_eq!(got.cover_asset_id, Some(ids[1]));
            assert_eq!(album_item_ids(c, copy)?, album_item_ids(c, a)?);
            // Changing the copy leaves the original alone.
            remove_from_album(c, copy, &[ids[0]])?;
            assert_eq!(album_item_ids(c, a)?.len(), 3);
            assert_eq!(get_album(c, duplicate_album(c, a, 6)?)?.name, "Lisbon copy 2");
            assert!(duplicate_album(c, 999, 7).is_err());
            Ok(())
        })
        .unwrap();
    }

    fn stored(c: &Connection, a: i64) -> (i64, Option<String>) {
        c.query_row("SELECT cover_picked, preview_key FROM albums WHERE id = ?1", [a], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    }

    /// The 2026-11-09 design: three prints as different as possible, kept until the photos change.
    #[test]
    fn list_picks_three_and_keeps_them() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 6);
            let a = create_album(c, "Six", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            let first = get_album(c, a)?;
            assert_eq!(first.preview_ids.len(), 3);
            assert_eq!(first.cover_asset_id, Some(first.preview_ids[0]));
            let key = stored(c, a).1;
            assert!(key.is_some());
            let second = get_album(c, a)?;
            assert_eq!(second.preview_ids, first.preview_ids);
            assert_eq!(stored(c, a).1, key, "a cache hit does not pick again");
            let empty = create_album(c, "E", None, 1)?;
            assert!(get_album(c, empty)?.preview_ids.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn adding_photos_repicks() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 6);
            let a = create_album(c, "A", None, 1)?;
            add_to_album(c, a, &ids[..5], 1)?;
            get_album(c, a)?;
            let key = stored(c, a).1;
            add_to_album(c, a, &ids[5..], 2)?;
            get_album(c, a)?;
            assert_ne!(stored(c, a).1, key);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn picked_cover_is_front() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 6);
            let a = create_album(c, "A", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            set_cover(c, a, Some(ids[4]))?;
            let got = get_album(c, a)?;
            assert_eq!(got.preview_ids[0], ids[4]);
            assert_eq!(got.cover_asset_id, Some(ids[4]));
            assert_eq!(stored(c, a).0, 1);
            set_cover(c, a, None)?;
            assert_eq!(stored(c, a).0, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn removed_picked_cover_goes_automatic() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 6);
            let a = create_album(c, "A", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            set_cover(c, a, Some(ids[2]))?;
            remove_from_album(c, a, &[ids[2]])?;
            let got = get_album(c, a)?;
            assert!(!got.preview_ids.contains(&ids[2]));
            assert_eq!(got.cover_asset_id, Some(got.preview_ids[0]));
            assert_eq!(stored(c, a).0, 0);
            // A picked cover that is trashed stays in the album but is never shown.
            set_cover(c, a, Some(ids[3]))?;
            c.execute("UPDATE assets SET state = 1 WHERE id = ?1", [ids[3]])?;
            let got = get_album(c, a)?;
            assert!(!got.preview_ids.contains(&ids[3]), "{:?}", got.preview_ids);
            assert_ne!(got.cover_asset_id, Some(ids[3]));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn shuffle_forgets_pick() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 6);
            let a = create_album(c, "A", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            set_cover(c, a, Some(ids[1]))?;
            get_album(c, a)?;
            let key = stored(c, a).1;
            shuffle_cover(c, a)?;
            assert_eq!(get_album(c, a)?.preview_ids.len(), 3);
            let (picked, new_key) = stored(c, a);
            assert_eq!(picked, 0);
            assert_ne!(new_key, key);
            assert!(shuffle_cover(c, 999).is_err());
            Ok(())
        })
        .unwrap();
    }

    /// Item 3: an icon cover in a swatch or a custom colour; a photo picked as the cover brings
    /// the photos back.
    #[test]
    fn an_icon_cover_is_kept_checked_and_copied() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 2);
            let a = create_album(c, "Trip", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            let style = CoverStyle::Icon { shape: CoverShape::Book, icon: "plane".into(), color: "swatch-3".into() };
            set_cover_style(c, a, Some(&style))?;
            assert_eq!(get_album(c, a)?.cover_style, Some(style.clone()));
            let custom = CoverStyle::Icon { shape: CoverShape::Box, icon: "beach".into(), color: "#1A2b3c".into() };
            set_cover_style(c, a, Some(&custom))?;
            assert_eq!(get_album(c, duplicate_album(c, a, 2)?)?.cover_style, Some(custom));
            for bad in [("plane", "swatch-11"), ("plane", "red"), ("plane", "#12345"), ("Plane!", "swatch-1"), ("", "swatch-1")] {
                let s = CoverStyle::Icon { shape: CoverShape::Folder, icon: bad.0.into(), color: bad.1.into() };
                assert!(set_cover_style(c, a, Some(&s)).is_err(), "{bad:?}");
            }
            assert!(set_cover_style(c, 999, Some(&style)).is_err());
            set_cover(c, a, Some(ids[1]))?;
            assert_eq!(get_album(c, a)?.cover_style, None, "a photo cover replaces the icon");
            set_cover_style(c, a, Some(&style))?;
            set_cover_style(c, a, None)?;
            assert_eq!(get_album(c, a)?.cover_style, None);
            // An unreadable stored value shows photos rather than failing the list.
            c.execute("UPDATE albums SET cover_style = 'nonsense' WHERE id = ?1", [a])?;
            assert_eq!(get_album(c, a)?.cover_style, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn renumbers_when_positions_get_too_close() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let ids = seed(c, 3);
            let a = create_album(c, "A", None, 1)?;
            add_to_album(c, a, &ids, 1)?;
            // Repeatedly move the last item before the first: bisections shrink until a renumber happens.
            for _ in 0..80 {
                let order = album_item_ids(c, a)?;
                move_in_album(c, a, *order.last().unwrap(), Some(order[0]))?;
            }
            let order = album_item_ids(c, a)?;
            assert_eq!(order.len(), 3);
            let positions: Vec<f64> = {
                let mut s = c.prepare("SELECT position FROM album_items WHERE album_id = ?1 ORDER BY position")?;
                let v: Vec<f64> = s.query_map([a], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                v
            };
            assert!(positions.windows(2).all(|w| w[1] - w[0] > 1e-6));
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod smart_tests {
    use super::*;
    use crate::Db;

    fn asset(c: &Connection, name: &str, mt: i64, rating: i64, ts: i64) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, rating)
             VALUES (?1, ?1, ?2, 'x', ?3, 1, 1, ?4, ?4, ?5)",
            params![format!("h-{name}"), name, mt, ts, rating],
        )
        .unwrap();
        let id = c.last_insert_rowid();
        crate::fts::index_ids(c, &[id]).unwrap();
        id
    }

    #[test]
    fn a_smart_album_counts_and_covers_from_its_query() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let old_clip = asset(c, "clip.mp4", 1, 0, 10);
            let new_clip = asset(c, "trip.mp4", 1, 5, 20);
            let _photo = asset(c, "photo.jpg", 0, 5, 30);

            let q = AssetQuery { media_type: Some(1), ..Default::default() };
            let id = create_smart_album(c, "  Videos  ", &q, 100)?;
            let album = get_album(c, id)?;
            assert_eq!(album.name, "Videos");
            assert_eq!(album.kind, KIND_SMART);
            assert_eq!(album.item_count, 2, "counted through the query, not album_items");
            assert_eq!(album.cover_asset_id, Some(new_clip), "newest match is the cover");
            assert!(!album.preview_ids.is_empty() && album.preview_ids.len() <= 3, "{:?}", album.preview_ids);
            assert_eq!(album.preview_ids[0], new_clip);
            assert_eq!(album.query, Some(q.clone()));
            assert_eq!(smart_query(c, id)?, Some(q));

            // Narrowing the query changes what the album holds, with no membership edits.
            let tighter = AssetQuery { media_type: Some(1), min_rating: Some(5), ..Default::default() };
            update_smart_album(c, id, "Best videos", &tighter, 200)?;
            let album = get_album(c, id)?;
            assert_eq!(album.name, "Best videos");
            assert_eq!(album.item_count, 1);
            assert_eq!(album.cover_asset_id, Some(new_clip));
            assert_ne!(album.cover_asset_id, Some(old_clip));

            // Manual albums are untouched by any of this.
            let manual = create_album(c, "Trip", None, 1)?;
            assert_eq!(smart_query(c, manual)?, None);
            assert!(update_smart_album(c, manual, "x", &AssetQuery::default(), 1).is_err());
            assert!(create_smart_album(c, "  ", &AssetQuery::default(), 1).is_err());
            Ok(())
        })
        .unwrap();
    }
}
