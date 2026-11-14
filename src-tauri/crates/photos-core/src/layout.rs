//! The justified grid is computed here, in Rust, from a compact in-memory index of
//! the whole timeline. The frontend positions tiles absolutely and never measures.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::sync::Arc;

use crate::assets::{is_renderable_extension, Neighbors};
use crate::scope::LayoutScope;
use crate::Result;

pub const HEADER_HEIGHT: u32 = 44;
const MIN_ROW_FACTOR: f32 = 0.7;
const MAX_ROW_FACTOR: f32 = 1.35;

/// How a timeline grid is cut into sections. A year holds too much for a justified run to
/// be worth scrolling, so it is laid out as a plain square grid instead — the shapes stop
/// mattering when the tiles are this small, and a regular grid is what the eye scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum GroupBy {
    Day,
    #[default]
    Month,
    Year,
}

impl GroupBy {
    /// Year is the square grid; the others are justified rows of their own shapes.
    pub fn square(self) -> bool {
        self == GroupBy::Year
    }
}

#[derive(Debug, Clone)]
pub struct LayoutItem {
    pub id: i64,
    pub aspect: f32,
    /// `COALESCE(captured_at, imported_at)`
    pub sort_ts: i64,
    /// `year * 12 + month0`, computed once at load with the local offset (see `month_key`).
    pub month: u32,
    /// Local days since the epoch, computed alongside `month` from the same offset.
    pub day: i32,
    pub media_type: i64,
    pub duration_ms: Option<i64>,
    /// base64, ready for the wire
    pub thumbhash: Option<String>,
    pub dominant_color: Option<String>,
    pub renderable: bool,
    pub extension: String,
    pub is_favorite: bool,
    pub trashed_at: Option<i64>,
    pub rating: i64,
    pub hdr: i64,
    pub color_label: i64,
    pub flag: i64,
    /// The app a copy was made for (Round 22 lane C): the tile shows its icon.
    pub edited_in: Option<String>,
    /// Filled by the offline atlas; used to name month sections.
    pub place: Option<String>,
    /// Set when the index is a collection's photos by album (Round 19 §10): the section this
    /// copy of the photo is in. One photo in two albums is two items.
    pub album: Option<Arc<AlbumSection>>,
}

/// One album's section in a collection's grid.
#[derive(Debug, PartialEq, Eq)]
pub struct AlbumSection {
    /// The section's position, in album order.
    pub ord: u32,
    pub album_id: i64,
    /// The album's name, with where it is below the opened collection when it is nested.
    pub title: String,
}

impl LayoutItem {
    pub fn with_ts(mut self, ts: i64, offset_secs: i32) -> Self {
        self.sort_ts = ts;
        self.month = month_key(ts, offset_secs);
        self.day = day_key(ts, offset_secs);
        self
    }
    pub fn placeholder() -> Self {
        Self {
            id: 0,
            aspect: 1.0,
            sort_ts: 0,
            month: 0,
            day: 0,
            media_type: 0,
            duration_ms: None,
            thumbhash: None,
            dominant_color: None,
            renderable: true,
            extension: "jpg".into(),
            is_favorite: false,
            trashed_at: None,
            rating: 0,
            hdr: 0,
            color_label: 0,
            flag: 0,
            edited_in: None,
            place: None,
            album: None,
        }
    }
}

/// The whole timeline, in order, ready to lay out at any width.
pub struct LayoutIndex {
    pub items: Vec<LayoutItem>,
}

/// The albums of collection `root` and of every collection inside it, with their section
/// titles, in the order the collection's page lists them: the collection's own albums
/// (most recently changed first, as the Albums page lists them), then each nested
/// collection by name. A nested album's title says where it is below `root`.
fn collection_albums(conn: &Connection, root: i64) -> Result<Vec<(i64, String)>> {
    fn walk(conn: &Connection, collection: i64, path: &mut Vec<String>, out: &mut Vec<(i64, String)>) -> Result<()> {
        // A refused move keeps the tree acyclic; the depth cap is only a guard.
        if path.len() > 32 {
            return Ok(());
        }
        let mut albums = conn.prepare("SELECT id, name FROM albums WHERE collection_id = ?1 ORDER BY updated_at DESC, id DESC")?;
        for row in albums.query_map([collection], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
            let (id, name) = row?;
            out.push((id, if path.is_empty() { name } else { format!("{name} · in {}", path.join(" › ")) }));
        }
        let mut inner = conn.prepare("SELECT id, name FROM album_collections WHERE parent_id = ?1 ORDER BY name COLLATE NOCASE, id")?;
        let inner = inner.query_map([collection], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, name) in inner {
            path.push(name);
            walk(conn, id, path, out)?;
            path.pop();
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(conn, root, &mut Vec::new(), &mut out)?;
    Ok(out)
}

/// How many assets a scope selects, without loading the index. Used by the search
/// header and the smart-album editor's live count.
pub fn count_scoped(conn: &Connection, scope: &LayoutScope) -> Result<i64> {
    let (from_where, _, params) = scope.sql();
    Ok(conn.query_row(&format!("SELECT COUNT(*) {from_where}"), rusqlite::params_from_iter(params.iter()), |r| r.get(0))?)
}

/// The ids a scope selects, in its order, at most `limit` (Round 4G: *Export…* and *Start
/// slideshow* on an album, a collection or Favorites from a menu, without opening the grid).
pub fn ids_scoped(conn: &Connection, scope: &LayoutScope, limit: i64) -> Result<Vec<i64>> {
    let (from_where, order, params) = scope.sql();
    let ranking = scope.ranking();
    // A ranked scope sorts after the query, so the limit applies after the sort.
    let sql_limit = if ranking.is_some() { String::new() } else { format!(" LIMIT {}", limit.max(0)) };
    let mut stmt = conn.prepare(&format!("SELECT a.id {from_where} ORDER BY {order}{sql_limit}"))?;
    let mut ids = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
    if let Some(rank) = ranking {
        ids.sort_by_key(|id| rank.get(id).copied().unwrap_or(usize::MAX));
        ids.truncate(limit.max(0) as usize);
    }
    Ok(ids)
}

impl LayoutIndex {
    /// The whole library in timeline order.
    pub fn load(conn: &Connection, ffmpeg_available: bool) -> Result<Self> {
        Self::load_scoped(conn, &LayoutScope::library(), ffmpeg_available)
    }

    /// Whatever `scope` selects, in the scope's order. `ffmpeg_available` decides whether
    /// HEIC and video tiles are renderable.
    pub fn load_scoped(conn: &Connection, scope: &LayoutScope, ffmpeg_available: bool) -> Result<Self> {
        if let Some(collection) = scope.collection_id {
            return Self::load_collection(conn, scope, collection, ffmpeg_available);
        }
        let offset = local_offset_secs();
        let (from_where, order, params) = scope.sql();
        // An edited photo takes the shape of its edit (a crop, a quarter turn).
        let from_where = from_where.replacen("FROM assets a", "FROM assets a LEFT JOIN edits ed ON ed.asset_id = a.id", 1);
        let mut stmt = conn.prepare(&format!(
            "SELECT a.id, a.width, a.height, a.orientation, COALESCE(a.captured_at, a.imported_at), a.media_type,
                    a.duration_ms, a.thumbhash, a.dominant_color, a.extension, a.is_favorite, a.trashed_at, a.rating, a.hdr, COALESCE(NULLIF(a.locality, ''), a.place_name),
                    ed.aspect, a.color_label, a.flag, a.captured_at IS NOT NULL, a.edited_in
             {from_where} ORDER BY {order}"
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            let w: Option<i64> = r.get(1)?;
            let h: Option<i64> = r.get(2)?;
            let orientation: i64 = r.get(3)?;
            let ext: String = r.get(9)?;
            let (w, h) = match (w, h) {
                (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
                _ => (4.0, 3.0),
            };
            let (w, h) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
            let sort_ts: i64 = r.get(4)?;
            // `captured_at` is the camera's wall clock stored as if it were UTC, so it is already
            // local; only an `imported_at` (true UTC) is shifted into local time.
            let shift = if r.get::<_, bool>(18)? { 0 } else { offset };
            let thumbhash: Option<Vec<u8>> = r.get(7)?;
            let aspect = match r.get::<_, Option<f64>>(15)? {
                Some(a) if a.is_finite() && a > 0.0 => a as f32,
                _ => w / h,
            };
            Ok(LayoutItem {
                id: r.get(0)?,
                aspect: aspect.clamp(0.2, 5.0),
                sort_ts,
                month: month_key(sort_ts, shift),
                day: day_key(sort_ts, shift),
                media_type: r.get(5)?,
                duration_ms: r.get(6)?,
                thumbhash: thumbhash.map(|b| b64(&b)),
                dominant_color: r.get(8)?,
                renderable: is_renderable_extension(&ext, ffmpeg_available),
                extension: ext,
                is_favorite: r.get::<_, i64>(10)? != 0,
                trashed_at: r.get(11)?,
                rating: r.get(12)?,
                hdr: r.get(13)?,
                place: r.get::<_, Option<String>>(14)?.filter(|p| !p.is_empty()),
                color_label: r.get(16)?,
                flag: r.get(17)?,
                edited_in: r.get(19)?,
                album: None,
            })
        })?;
        let mut items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(rank) = scope.ranking() {
            items.sort_by_key(|i| rank.get(&i.id).copied().unwrap_or(usize::MAX));
        }
        Ok(Self { items })
    }

    /// A collection's photos, one section per album (Round 19 §10), each album loaded exactly
    /// as its own page would show it (hidden photos, its manual order, a smart album's query).
    /// Albums without a photo make no section.
    fn load_collection(conn: &Connection, scope: &LayoutScope, collection: i64, ffmpeg_available: bool) -> Result<Self> {
        let mut items = Vec::new();
        let mut ord = 0u32;
        for (album_id, title) in collection_albums(conn, collection)? {
            let sub = match crate::albums::smart_query(conn, album_id)? {
                Some(q) => LayoutScope { collection_id: None, album_id: None, query: Some(q), ..scope.clone() },
                None => LayoutScope { collection_id: None, album_id: Some(album_id), ..scope.clone() },
            };
            let mut loaded = Self::load_scoped(conn, &sub, ffmpeg_available)?.items;
            if loaded.is_empty() {
                continue;
            }
            let section = Arc::new(AlbumSection { ord, album_id, title });
            ord += 1;
            for it in &mut loaded {
                it.album = Some(section.clone());
            }
            items.append(&mut loaded);
        }
        Ok(Self { items })
    }

    /// `group`: cut the timeline into day, month or year sections with header rows, or
    /// (`None`) lay everything out as one untitled run (album order). A year is a square
    /// grid with `target_row_height` as the side; the others are justified rows.
    pub fn compute(&self, width: u32, target_row_height: u32, gap: u32, group: Option<GroupBy>) -> Layout {
        let items: Vec<&LayoutItem> = self.items.iter().collect();
        // A collection's photos by album: one titled justified section per album, whatever the
        // caller's grouping says (the index itself is the sections).
        let by_album = items.first().is_some_and(|i| i.album.is_some());
        let group = if by_album { Some(GroupBy::Month) } else { group };
        let mut rows: Vec<LayoutRow> = Vec::new();
        let mut sections: Vec<Section> = Vec::new();
        let width = width.max(1) as f32;
        let square = group.is_some_and(GroupBy::square);
        let target = target_row_height.max(if square { 24 } else { 40 }) as f32;
        let gap_f = gap as f32;
        let mut y: u32 = 0;
        let mut i = 0;
        let key_of = |it: &LayoutItem| -> i64 {
            if by_album {
                return it.album.as_ref().map_or(0, |a| a.ord as i64);
            }
            match group {
                None => 0,
                Some(GroupBy::Day) => it.day as i64,
                Some(GroupBy::Month) => it.month as i64,
                Some(GroupBy::Year) => (it.month / 12) as i64,
            }
        };
        while i < items.len() {
            let key = key_of(items[i]);
            let section_index = sections.len() as u32;
            let first_row = rows.len() as u32;
            let section_y = y;
            let year = items[i].month / 12;
            let month = (items[i].month % 12 + 1) as u8;
            let album = items[i].album.as_ref();
            let title = match group {
                None => String::new(),
                Some(_) if by_album => album.map(|a| a.title.clone()).unwrap_or_default(),
                Some(g) => {
                    let name = match g {
                        GroupBy::Day => day_title_for_key(key as i32),
                        GroupBy::Month => month_title_for_key(key as u32),
                        GroupBy::Year => key.to_string(),
                    };
                    match dominant_place(&items[i..], key_of, key) {
                        Some(place) => format!("{name} · {place}"),
                        None => name,
                    }
                }
            };
            if group.is_some() {
                rows.push(LayoutRow {
                    index: first_row,
                    kind: RowKind::Header,
                    y,
                    height: HEADER_HEIGHT,
                    section: section_index,
                    title: Some(title.clone()),
                    tiles: Vec::new(),
                });
                y += HEADER_HEIGHT;
            }
            let mut item_count = 0u32;
            if square {
                // As many columns of the target side as fit, then widened to fill the row
                // exactly; a part-filled last row keeps the same size and sits on the left.
                let cols = (((width + gap_f) / (target + gap_f)).floor() as usize).max(1);
                let side = (width - (cols - 1) as f32 * gap_f) / cols as f32;
                let side_u = side.round().max(1.0) as u32;
                while i < items.len() && key_of(items[i]) == key {
                    let start = i;
                    while i < items.len() && key_of(items[i]) == key && i - start < cols {
                        i += 1;
                    }
                    let n = i - start;
                    let mut tiles: Vec<Tile> = items[start..i]
                        .iter()
                        .enumerate()
                        .map(|(k, item)| tile_for(item, (k as f32 * (side + gap_f)).round() as u32, side_u, side_u))
                        .collect();
                    if n == cols {
                        if let Some(last) = tiles.last_mut() {
                            last.w = (width as u32).saturating_sub(last.x).max(1);
                        }
                    }
                    rows.push(LayoutRow {
                        index: rows.len() as u32,
                        kind: RowKind::Tiles,
                        y,
                        height: side_u,
                        section: section_index,
                        title: None,
                        tiles,
                    });
                    y += side_u + gap;
                    item_count += n as u32;
                }
            }
            while i < items.len() && key_of(items[i]) == key {
                // Greedy fill: take items until the row at target height overflows the width.
                let start = i;
                let mut sum_aspect = 0f32;
                while i < items.len() && key_of(items[i]) == key {
                    let next = sum_aspect + items[i].aspect;
                    let gaps = (i - start) as f32 * gap_f;
                    if start != i && next * target + gaps > width {
                        break;
                    }
                    sum_aspect = next;
                    i += 1;
                }
                // Stopping there always leaves the row *taller* than the target, because the
                // item that would have overflowed is the one that would have pulled it down.
                // Taking it instead often lands closer, so compare both and keep the better
                // row: heights then sit either side of the target rather than always above it.
                if i < items.len() && key_of(items[i]) == key {
                    let row_h = |n: usize, sum: f32| (width - (n.saturating_sub(1)) as f32 * gap_f) / sum;
                    let without = row_h(i - start, sum_aspect);
                    let with = row_h(i - start + 1, sum_aspect + items[i].aspect);
                    if (with - target).abs() < (without - target).abs() {
                        sum_aspect += items[i].aspect;
                        i += 1;
                    }
                }
                let n = i - start;
                let gaps = (n.saturating_sub(1)) as f32 * gap_f;
                let last_in_section = !(i < items.len() && key_of(items[i]) == key);
                let mut row_h = (width - gaps) / sum_aspect;
                let stretch = !(last_in_section && row_h > target * MAX_ROW_FACTOR);
                if stretch {
                    row_h = row_h.clamp(target * MIN_ROW_FACTOR, target * MAX_ROW_FACTOR);
                } else {
                    row_h = target;
                }
                let row_h_u = row_h.round().max(1.0) as u32;
                let mut tiles = Vec::with_capacity(n);
                let mut x = 0f32;
                for (k, item) in items[start..i].iter().enumerate() {
                    let mut w = item.aspect * row_h;
                    // Make full rows span the container exactly: give the remainder to the last tile.
                    if stretch && k == n - 1 {
                        w = width - x;
                    }
                    tiles.push(tile_for(item, x.round() as u32, (w.round() as u32).max(1), row_h_u));
                    x += w + gap_f;
                }
                if stretch {
                    if let Some(last) = tiles.last_mut() {
                        last.w = (width as u32).saturating_sub(last.x).max(1);
                    }
                }
                rows.push(LayoutRow {
                    index: rows.len() as u32,
                    kind: RowKind::Tiles,
                    y,
                    height: row_h_u,
                    section: section_index,
                    title: None,
                    tiles,
                });
                y += row_h_u + gap;
                item_count += n as u32;
            }
            sections.push(Section {
                index: section_index,
                year,
                month,
                title,
                y: section_y,
                height: y - section_y,
                first_row,
                row_count: rows.len() as u32 - first_row,
                item_count,
                album_id: album.map(|a| a.album_id),
            });
        }
        let total_height = y.saturating_sub(if rows.is_empty() { 0 } else { gap });
        let row_heights = rows.iter().map(|r| r.height.min(u16::MAX as u32) as u16).collect();
        Layout {
            summary: LayoutSummary { total_height, item_count: items.len() as u32, row_heights, sections },
            rows,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RowKind {
    Header,
    Tiles,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Tile {
    #[ts(type = "number")]
    pub id: i64,
    pub x: u32,
    pub w: u32,
    pub h: u32,
    /// base64 ThumbHash bytes
    pub thumbhash: Option<String>,
    pub color: Option<String>,
    #[ts(type = "number")]
    pub media_type: i64,
    #[ts(type = "number | null")]
    pub duration_ms: Option<i64>,
    pub renderable: bool,
    pub extension: String,
    pub is_favorite: bool,
    #[ts(type = "number | null")]
    pub trashed_at: Option<i64>,
    #[ts(type = "number")]
    pub rating: i64,
    /// 0 ordinary, 1 gain map, 2 PQ, 3 HLG.
    #[ts(type = "number")]
    pub hdr: i64,
    /// A camera RAW original: the tile carries a RAW badge.
    pub raw: bool,
    /// 0 none, 1 red … 5 purple.
    #[ts(type = "number")]
    pub color_label: i64,
    /// −1 rejected, 0 none, 1 picked.
    #[ts(type = "number")]
    pub flag: i64,
    /// The id of the app a copy was made for, shown as that app's icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub edited_in: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LayoutRow {
    pub index: u32,
    pub kind: RowKind,
    pub y: u32,
    pub height: u32,
    pub section: u32,
    pub title: Option<String>,
    pub tiles: Vec<Tile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Section {
    pub index: u32,
    /// The year of the section's first item, for the scrubber: titles carry a place.
    pub year: u32,
    /// Its month, 1–12, for the scrubber's month ticks.
    pub month: u8,
    pub title: String,
    pub y: u32,
    pub height: u32,
    pub first_row: u32,
    pub row_count: u32,
    pub item_count: u32,
    /// The album a section stands for, in a collection's photos by album: its header opens it.
    #[ts(type = "number | null")]
    pub album_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LayoutSummary {
    pub total_height: u32,
    pub item_count: u32,
    pub row_heights: Vec<u16>,
    pub sections: Vec<Section>,
}

/// A tile in the grid: the photo and the section it is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Slot {
    #[ts(type = "number")]
    pub id: i64,
    pub section: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlotNeighbors {
    pub prev: Option<Slot>,
    pub next: Option<Slot>,
}

pub struct Layout {
    pub summary: LayoutSummary,
    pub rows: Vec<LayoutRow>,
}

impl Layout {
    /// The index of the row holding `id`, for jumping to a photo from another grouping.
    pub fn row_of(&self, id: i64) -> Option<u32> {
        self.rows.iter().find(|r| r.tiles.iter().any(|t| t.id == id)).map(|r| r.index)
    }

    /// The tiles before and after `id` in the order the grid draws them, across rows and
    /// sections, so the viewer pages through exactly what the grid showed.
    pub fn neighbors_of(&self, id: i64) -> Option<Neighbors> {
        self.neighbors_in(id, None)
    }

    /// [`Self::neighbors_of`] for the copy of `id` in `section`: a photo that is in two albums
    /// of a collection pages on from the section it was opened in. `None` means the first copy.
    pub fn neighbors_in(&self, id: i64, section: Option<u32>) -> Option<Neighbors> {
        self.slots_around(id, section).map(|n| Neighbors { prev: n.prev.map(|s| s.id), next: n.next.map(|s| s.id) })
    }

    /// The tiles before and after the copy of `id` in `section`, each with its own section, so
    /// the viewer can keep walking through a photo that is in two albums (Round 19 §10).
    pub fn slots_around(&self, id: i64, section: Option<u32>) -> Option<SlotNeighbors> {
        let mut tiles = self.rows.iter().flat_map(|r| r.tiles.iter().map(move |t| Slot { id: t.id, section: r.section }));
        let mut prev = None;
        for t in tiles.by_ref() {
            if t.id == id && section.is_none_or(|want| want == t.section) {
                return Some(SlotNeighbors { prev, next: tiles.next() });
            }
            prev = Some(t);
        }
        None
    }

    /// Every tile's id, in the order the grid draws them: what the face radar checks (Round 10).
    pub fn ids(&self) -> Vec<i64> {
        self.rows.iter().flat_map(|r| r.tiles.iter().map(|t| t.id)).collect()
    }

    /// Every tile from `a` to `b` inclusive, in the order the grid draws them (either way
    /// round), or `None` when either is not in this layout (Round 5 shift-select).
    pub fn ids_between(&self, a: i64, b: i64) -> Option<Vec<i64>> {
        let order: Vec<i64> = self.rows.iter().flat_map(|r| r.tiles.iter().map(|t| t.id)).collect();
        let i = order.iter().position(|&id| id == a)?;
        let j = order.iter().position(|&id| id == b)?;
        let (from, to) = if i <= j { (i, j) } else { (j, i) };
        Some(order[from..=to].to_vec())
    }

    pub fn rows(&self, start: usize, count: usize) -> &[LayoutRow] {
        let start = start.min(self.rows.len());
        let end = start.saturating_add(count).min(self.rows.len());
        &self.rows[start..end]
    }
}

/// The machine's current UTC offset, applied to UTC timestamps (`imported_at`), never to
/// `captured_at`, which is already the camera's wall clock. One
/// lookup per index load instead of one per item, at the cost of ignoring DST
/// history for month grouping (a photo taken within an hour of midnight on a month
/// boundary may land on the neighbouring month).
pub fn local_offset_secs() -> i32 {
    use chrono::Offset;
    chrono::Local::now().offset().fix().local_minus_utc()
}

/// `year * 12 + month0` for a unix timestamp shifted by `offset_secs`.
pub fn month_key(ts: i64, offset_secs: i32) -> u32 {
    let (y, m, _) = civil_from_days((ts + offset_secs as i64).div_euclid(86_400));
    (y.max(0) as u32) * 12 + (m - 1)
}

/// Local days since 1970-01-01 for a unix timestamp shifted by `offset_secs`.
pub fn day_key(ts: i64, offset_secs: i32) -> i32 {
    (ts + offset_secs as i64).div_euclid(86_400) as i32
}

/// "Tuesday, 16 September 2025" for a day key.
pub fn day_title_for_key(key: i32) -> String {
    const DAYS: [&str; 7] = ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"];
    let (y, m, d) = civil_from_days(key as i64);
    let month = month_title_for_key(m - 1);
    let month = month.split(' ').next().unwrap_or_default();
    format!("{}, {d} {month} {y}", DAYS[(key as i64).rem_euclid(7) as usize])
}

fn tile_for(item: &LayoutItem, x: u32, w: u32, h: u32) -> Tile {
    Tile {
        id: item.id,
        x,
        w,
        h,
        thumbhash: item.thumbhash.clone(),
        color: item.dominant_color.clone(),
        media_type: item.media_type,
        duration_ms: item.duration_ms,
        renderable: item.renderable,
        extension: item.extension.clone(),
        is_favorite: item.is_favorite,
        trashed_at: item.trashed_at,
        rating: item.rating,
        hdr: item.hdr,
        raw: crate::formats::is_raw(&item.extension),
        color_label: item.color_label,
        flag: item.flag,
        edited_in: item.edited_in.clone(),
    }
}

/// The place most of a section was taken, when one clearly dominates. A month split
/// evenly between two cities gets no place rather than an arbitrary winner.
fn dominant_place(rest: &[&LayoutItem], key_of: impl Fn(&LayoutItem) -> i64, key: i64) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut total = 0usize;
    for item in rest.iter().take_while(|it| key_of(it) == key) {
        total += 1;
        if let Some(p) = item.place.as_deref() {
            *counts.entry(p).or_default() += 1;
        }
    }
    if total < 3 {
        return None;
    }
    let (place, n) = counts.into_iter().max_by_key(|(_, n)| *n)?;
    // Three fifths is enough to call it: a trip with a few detours still reads as the trip.
    if n * 5 >= total * 3 {
        Some(place.to_string())
    } else {
        None
    }
}

pub fn month_title_for_key(key: u32) -> String {
    const NAMES: [&str; 12] = [
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
        "December",
    ];
    format!("{} {}", NAMES[(key % 12) as usize], key / 12)
}

/// "September 2026" in local time.
pub fn month_title(ts: i64) -> String {
    month_title_for_key(month_key(ts, local_offset_secs()))
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian (Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A greedy fill that stops *before* the item that would overflow can only ever
    /// produce rows at or above the target height. Over a run of landscape photos that
    /// bias compounds: every row comes out taller than the size the user picked.
    #[test]
    fn row_heights_centre_on_the_target_instead_of_only_overshooting() {
        let items: Vec<LayoutItem> = (0..200i64)
            .map(|i| LayoutItem { id: i, aspect: 1.5, ..LayoutItem::placeholder() }.with_ts(1_758_000_000, 0))
            .collect();
        let l = LayoutIndex { items }.compute(1000, 100, 0, None);
        let heights: Vec<u32> = l.rows.iter().filter(|r| r.kind == RowKind::Tiles).map(|r| r.height).collect();
        let worst = heights.iter().map(|h| (*h as i64 - 100).abs()).max().unwrap();
        assert!(worst <= 8, "rows should sit near the 100px target, worst was {worst} off: {heights:?}");
    }

    #[test]
    fn fills_rows_to_width_and_groups_by_month() {
        let mut items = vec![];
        for i in 0..10i64 {
            items.push(
                LayoutItem { id: i, aspect: if i % 2 == 0 { 1.5 } else { 0.75 }, ..LayoutItem::placeholder() }
                    .with_ts(1_758_000_000 - i * 3600, 0),
            );
        }
        items.push(LayoutItem { id: 99, aspect: 1.0, ..LayoutItem::placeholder() }.with_ts(1_700_000_000, 0));
        let idx = LayoutIndex { items };
        let l = idx.compute(1000, 200, 4, Some(GroupBy::Month));
        assert_eq!(l.summary.sections.len(), 2);
        assert_eq!(l.rows[0].kind, RowKind::Header);
        let first = &l.rows[1];
        assert_eq!(first.kind, RowKind::Tiles);
        let span = first.tiles.last().map(|t| t.x + t.w).unwrap();
        assert_eq!(span, 1000, "full rows span the container exactly");
        assert!(first.height >= 140 && first.height <= 270, "{}", first.height);
        assert_eq!(l.summary.item_count, 11);
        assert_eq!(l.summary.row_heights.len(), l.rows.len());
        assert_eq!(l.summary.total_height, l.rows.last().map(|r| r.y + r.height).unwrap());
        let last_section = l.summary.sections.last().unwrap();
        assert_eq!(l.rows[last_section.first_row as usize].title.as_deref(), Some("November 2023"));
        assert_eq!(last_section.item_count, 1);
        // A lone item in a section is left-aligned at target height, not stretched to 1000px wide.
        let lone = &l.rows[last_section.first_row as usize + 1];
        assert_eq!(lone.height, 200);
        assert_eq!(lone.tiles[0].w, 200);
        // Unsectioned (album order): no header rows, one untitled section.
        let flat = idx.compute(1000, 200, 4, None);
        assert_eq!(flat.summary.sections.len(), 1);
        assert!(flat.rows.iter().all(|r| r.kind == RowKind::Tiles));
        assert_eq!(flat.summary.item_count, 11);
        let empty = LayoutIndex { items: vec![] }.compute(1000, 200, 4, Some(GroupBy::Month));
        assert_eq!(empty.summary.total_height, 0);
        assert!(empty.rows.is_empty());
    }

    #[test]
    fn loads_from_db_in_timeline_order() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, captured_at, imported_at)
                 VALUES (1,'a','p','a.jpg','jpg',0,1,4000,3000,1,500,10),
                        (2,'b','p','b.jpg','jpg',0,1,4000,3000,6,NULL,900),
                        (3,'c','p','c.mov','mov',1,1,NULL,NULL,1,700,10);",
            )?;
            Ok(())
        })
        .unwrap();
        let idx = db.with_conn(|c| LayoutIndex::load(c, false)).unwrap();
        assert_eq!(idx.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![2, 3, 1]);
        let videos = db.with_conn(|c| LayoutIndex::load_scoped(c, &LayoutScope { media_type: Some(1), ..Default::default() }, false)).unwrap();
        assert_eq!(videos.items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![3]);
        assert!((idx.items[2].aspect - 4.0 / 3.0).abs() < 1e-4);
        assert!((idx.items[0].aspect - 3.0 / 4.0).abs() < 1e-4, "orientation 6 swaps the aspect");
        assert!(!idx.items[1].renderable);
        assert_eq!(b64(&[1, 2, 3]), "AQID");
        assert_eq!(b64(&[1, 2]), "AQI=");
    }

    /// Opened from an album, the viewer pages through the album in its manual order and
    /// never onto a library photo outside it (Round 4A: the chevron walked the timeline).
    #[test]
    fn an_album_pages_in_its_own_order_and_nowhere_else() {
        let db = crate::Db::open_in_memory().unwrap();
        let album = db
            .with_conn(|c| {
                c.execute_batch(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, captured_at, imported_at)
                     VALUES (1,'a','p','a.jpg','jpg',0,1,4000,3000,1,100,10),
                            (2,'b','p','b.jpg','jpg',0,1,4000,3000,1,200,10),
                            (3,'c','p','c.jpg','jpg',0,1,4000,3000,1,300,10),
                            (4,'d','p','d.jpg','jpg',0,1,4000,3000,1,250,10);",
                )?;
                let album = crate::albums::create_album(c, "Trip", None, 0)?;
                // Added out of timeline order: 3, 1, 2. Photo 4 is only in the library.
                crate::albums::add_to_album(c, album, &[3, 1, 2], 0)?;
                Ok(album)
            })
            .unwrap();
        let scope = LayoutScope { album_id: Some(album), ..Default::default() };
        let idx = db.with_conn(|c| LayoutIndex::load_scoped(c, &scope, false)).unwrap();
        let l = idx.compute(1000, 200, 4, scope.sectioned().then_some(GroupBy::Month));
        assert_eq!(l.neighbors_of(3), Some(Neighbors { prev: None, next: Some(1) }));
        assert_eq!(l.neighbors_of(1), Some(Neighbors { prev: Some(3), next: Some(2) }));
        assert_eq!(l.neighbors_of(2), Some(Neighbors { prev: Some(1), next: None }), "the album ends here");
        assert_eq!(l.neighbors_of(4), None, "a library photo is not in the album");
    }

    /// Round 19 §10: a collection's photos in sections by album, nested albums included.
    fn collection_library() -> (crate::Db, i64, i64, i64) {
        let db = crate::Db::open_in_memory().unwrap();
        let ids = db
            .with_conn(|c| {
                c.execute_batch(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, captured_at, imported_at)
                     VALUES (1,'a','p','a.jpg','jpg',0,1,4000,3000,1,100,10),
                            (2,'b','p','b.jpg','jpg',0,1,4000,3000,1,200,10),
                            (3,'c','p','c.jpg','jpg',0,1,4000,3000,1,300,10),
                            (4,'d','p','d.jpg','jpg',0,1,4000,3000,1,400,10);",
                )?;
                let trips = crate::album_collections::create(c, "Trips", None, 0)?;
                let europe = crate::album_collections::create(c, "Europe", Some(trips), 0)?;
                let road = crate::albums::create_album(c, "Road", None, 1)?;
                let rome = crate::albums::create_album(c, "Rome", None, 2)?;
                let empty = crate::albums::create_album(c, "Empty", None, 3)?;
                crate::album_collections::move_album(c, road, Some(trips), 4)?;
                crate::album_collections::move_album(c, rome, Some(europe), 4)?;
                crate::album_collections::move_album(c, empty, Some(trips), 4)?;
                // Photo 2 is in both; Road's manual order is 2, 1.
                crate::albums::add_to_album(c, road, &[2, 1], 5)?;
                crate::albums::add_to_album(c, rome, &[2, 3], 5)?;
                Ok((trips, road, rome))
            })
            .unwrap();
        (db, ids.0, ids.1, ids.2)
    }

    #[test]
    fn a_collection_lists_its_albums_as_sections_and_a_photo_in_two_albums_shows_in_both() {
        let (db, trips, road, rome) = collection_library();
        let scope = LayoutScope { collection_id: Some(trips), ..Default::default() };
        let idx = db.with_conn(|c| LayoutIndex::load_scoped(c, &scope, false)).unwrap();
        let l = idx.compute(1000, 200, 4, scope.sectioned().then_some(GroupBy::Month));
        let s = &l.summary.sections;
        assert_eq!(s.len(), 2, "the empty album has no section");
        assert_eq!((s[0].title.as_str(), s[0].album_id, s[0].item_count), ("Road", Some(road), 2));
        assert_eq!((s[1].title.as_str(), s[1].album_id, s[1].item_count), ("Rome · in Europe", Some(rome), 2));
        assert_eq!(l.ids(), vec![2, 1, 2, 3], "sections in album order, each in its album's own order");
        assert_eq!(l.summary.item_count, 4, "photo 2 counts once per section");
        // Every row's height is in the summary, header rows included.
        assert_eq!(l.summary.row_heights.len(), l.rows.len());
        let headers: Vec<_> = l.rows.iter().filter(|r| r.kind == RowKind::Header).map(|r| r.title.clone().unwrap()).collect();
        assert_eq!(headers, vec!["Road".to_string(), "Rome · in Europe".to_string()]);
    }

    /// Paging walks the sections in order; the one photo in two albums is told apart by its section.
    #[test]
    fn a_photo_in_two_sections_pages_from_the_section_it_was_opened_in() {
        let (db, trips, _, _) = collection_library();
        let scope = LayoutScope { collection_id: Some(trips), ..Default::default() };
        let idx = db.with_conn(|c| LayoutIndex::load_scoped(c, &scope, false)).unwrap();
        let l = idx.compute(1000, 200, 4, None);
        assert_eq!(l.neighbors_in(2, Some(0)), Some(Neighbors { prev: None, next: Some(1) }));
        assert_eq!(l.neighbors_in(2, Some(1)), Some(Neighbors { prev: Some(1), next: Some(3) }));
        assert_eq!(l.neighbors_in(3, Some(1)), Some(Neighbors { prev: Some(2), next: None }));
        // Walking on from the last photo of one section lands in the next, on the right copy.
        let from_one = l.slots_around(1, None).unwrap();
        assert_eq!(from_one.next, Some(Slot { id: 2, section: 1 }));
        assert_eq!(l.slots_around(2, Some(1)).unwrap().prev, Some(Slot { id: 1, section: 0 }));
    }

    #[test]
    fn an_empty_collection_is_an_empty_layout() {
        let (db, _, _, _) = collection_library();
        let none = db.with_conn(|c| crate::album_collections::create(c, "Nothing", None, 0)).unwrap();
        let idx = db.with_conn(|c| LayoutIndex::load_scoped(c, &LayoutScope { collection_id: Some(none), ..Default::default() }, false)).unwrap();
        assert!(idx.compute(1000, 200, 4, None).rows.is_empty());
    }

    #[test]
    fn an_edited_crop_changes_the_tile_shape() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, imported_at)
                 VALUES (1, 'h1', 'p1', 'f1', 'jpg', 0, 1, 4000, 3000, 1, 10)",
                [],
            )?;
            let square = crate::edits::Recipe { crop: Some(crate::edits::Crop { x: 0.125, y: 0.0, w: 0.75, h: 1.0 }), ..crate::edits::Recipe::identity() };
            crate::edits::save_recipe(c, 1, &square, 1)?;
            let idx = LayoutIndex::load(c, false)?;
            assert!((idx.items[0].aspect - 1.0).abs() < 1e-4, "{}", idx.items[0].aspect);
            crate::edits::delete_recipe(c, 1)?;
            let idx = LayoutIndex::load(c, false)?;
            assert!((idx.items[0].aspect - 4.0 / 3.0).abs() < 1e-4, "revert restores the original shape");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn civil_dates_and_month_keys() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_617), (2023, 9, 17));
        assert_eq!(month_title_for_key(month_key(1_758_000_000, 0)), "September 2025");
        // 2025-09-30T23:30Z is still September at UTC but October at +02:00.
        let ts = 1_759_275_000;
        assert_ne!(month_key(ts, 0), month_key(ts, 7200));
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;
    use crate::query::AssetQuery;
    use crate::Db;

    #[test]
    #[ignore = "diagnostic: prints the query plan"]
    fn explains_the_structured_query() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 20_000, 3)?;
            let scope = LayoutScope {
                query: Some(AssetQuery { min_rating: Some(3), media_type: Some(0), has_gps: Some(true), ..Default::default() }),
                ..Default::default()
            };
            let (from_where, _, params) = scope.sql();
            let sql = format!("SELECT COUNT(*) {from_where}");
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| r.get::<_, String>(3))?;
            for r in rows {
                println!("PLAN: {}", r?);
            }
            let (gps, rated): (i64, i64) = c.query_row(
                "SELECT SUM(gps_lat IS NOT NULL), SUM(rating >= 3) FROM assets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            println!("rows with gps: {gps}, rated 3+: {rated}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn counts_what_a_scope_selects() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for (i, (name, mt, fav)) in [("a.jpg", 0, 1), ("b.jpg", 0, 0), ("c.mp4", 1, 0)].iter().enumerate() {
                c.execute(
                    "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, is_favorite)
                     VALUES (?1, ?1, ?2, 'x', ?3, 1, 1, ?4, ?5)",
                    rusqlite::params![format!("h{i}"), name, mt, 100 - i as i64, fav],
                )?;
            }
            crate::fts::rebuild(c)?;
            assert_eq!(count_scoped(c, &LayoutScope::library())?, 3);
            assert_eq!(count_scoped(c, &LayoutScope { favorites: true, ..Default::default() })?, 1);
            assert_eq!(count_scoped(c, &LayoutScope { media_type: Some(1), ..Default::default() })?, 1);
            let text = LayoutScope { query: Some(AssetQuery { text: Some("b".into()), ..Default::default() }), ..Default::default() };
            assert_eq!(count_scoped(c, &text)?, 1, "free text narrows the count");
            // Round 4G: the ids themselves, in the scope's order, for Export… and slideshows.
            let all = ids_scoped(c, &LayoutScope::library(), 10)?;
            assert_eq!(all.len(), 3);
            assert_eq!(ids_scoped(c, &LayoutScope::library(), 2)?, all[..2].to_vec());
            assert_eq!(ids_scoped(c, &LayoutScope { favorites: true, ..Default::default() }, 10)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}

#[cfg(test)]
mod place_title_tests {
    use super::*;

    fn item(month: u32, place: Option<&str>) -> LayoutItem {
        LayoutItem { month, place: place.map(str::to_string), ..LayoutItem::placeholder() }
    }

    #[test]
    fn names_a_month_after_the_place_most_of_it_was_taken() {
        let items: Vec<LayoutItem> = (0..5).map(|_| item(24_300, Some("Lisbon, Portugal"))).collect();
        let index = LayoutIndex { items };
        let layout = index.compute(1000, 200, 4, Some(GroupBy::Month));
        assert!(layout.summary.sections[0].title.ends_with("· Lisbon, Portugal"), "{}", layout.summary.sections[0].title);
    }

    #[test]
    fn leaves_a_mixed_month_as_just_the_month() {
        let mut items: Vec<LayoutItem> = (0..3).map(|_| item(24_300, Some("Lisbon, Portugal"))).collect();
        items.extend((0..3).map(|_| item(24_300, Some("Madrid, Spain"))));
        let layout = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Month));
        assert!(!layout.summary.sections[0].title.contains('·'), "{}", layout.summary.sections[0].title);

        // Too few photos to draw a conclusion from.
        let tiny = LayoutIndex { items: vec![item(24_300, Some("Lisbon, Portugal"))] };
        assert!(!tiny.compute(1000, 200, 4, Some(GroupBy::Month)).summary.sections[0].title.contains('·'));

        // No places at all: unchanged.
        let none = LayoutIndex { items: (0..4).map(|_| item(24_300, None)).collect() };
        assert!(!none.compute(1000, 200, 4, Some(GroupBy::Month)).summary.sections[0].title.contains('·'));
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    /// 2025-09-16T10:00Z, a Tuesday.
    const TUE: i64 = 1_758_016_800;

    fn at(id: i64, ts: i64) -> LayoutItem {
        LayoutItem { id, aspect: 1.5, ..LayoutItem::placeholder() }.with_ts(ts, 0)
    }

    #[test]
    fn days_become_their_own_sections_named_with_the_weekday() {
        let items = vec![at(1, TUE + 3600), at(2, TUE), at(3, TUE - 86_400), at(4, TUE - 86_400 * 40)];
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Day));
        let titles: Vec<&str> = l.summary.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Tuesday, 16 September 2025", "Monday, 15 September 2025", "Thursday, 7 August 2025"]);
        assert_eq!(l.summary.sections[0].item_count, 2);
        assert!(l.summary.sections.iter().all(|s| s.year == 2025));
    }

    #[test]
    fn months_are_the_default_grouping() {
        let items = vec![at(1, TUE), at(2, TUE - 86_400), at(3, TUE - 86_400 * 40)];
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Month));
        let titles: Vec<&str> = l.summary.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["September 2025", "August 2025"]);
    }

    /// The scrubber places month ticks by the section's own month (Round 4H).
    #[test]
    fn sections_carry_their_month() {
        let items = vec![at(1, TUE), at(2, TUE - 86_400 * 40)];
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Day));
        let months: Vec<(u32, u8)> = l.summary.sections.iter().map(|s| (s.year, s.month)).collect();
        assert_eq!(months, vec![(2025, 9), (2025, 8)]);
    }

    /// A year is a plain square grid that fills the width exactly: the shapes stop
    /// mattering at this size, and a regular grid is what the eye scans quickly.
    #[test]
    fn a_year_is_a_square_grid_that_fills_the_width() {
        let mut items: Vec<LayoutItem> = (0..25).map(|i| at(i, TUE - i * 86_400 * 3)).collect();
        items.push(LayoutItem { aspect: 0.5, ..at(99, TUE - 86_400 * 400) });
        let l = LayoutIndex { items }.compute(1000, 60, 4, Some(GroupBy::Year));
        let titles: Vec<&str> = l.summary.sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["2025", "2024"]);
        assert_eq!(l.summary.sections[0].year, 2025);
        let tiles: Vec<&Tile> = l.rows.iter().flat_map(|r| r.tiles.iter()).collect();
        assert!(tiles.iter().all(|t| t.w.abs_diff(t.h) <= 1), "every tile is square");
        // 1000 wide at a 60 px target with 4 px gaps: 15 columns of ~63 px.
        let first = l.rows.iter().find(|r| r.kind == RowKind::Tiles).unwrap();
        assert_eq!(first.tiles.len(), 15);
        let last = first.tiles.last().unwrap();
        assert_eq!(last.x + last.w, 1000, "a full row spans the container");
        // The part-filled row keeps the same size and sits on the left.
        let second = &l.rows[2];
        assert_eq!(second.tiles.len(), 10);
        assert_eq!(second.tiles[0].w, first.tiles[0].w);
        assert_eq!(second.height, first.height);
        // A lone portrait is still a square.
        let lone = l.rows.last().unwrap();
        assert_eq!(lone.tiles[0].w, first.tiles[0].w);
    }

    #[test]
    fn ungrouped_is_one_untitled_run() {
        let items = vec![at(1, TUE), at(2, TUE - 86_400 * 400)];
        let l = LayoutIndex { items }.compute(1000, 200, 4, None);
        assert_eq!(l.summary.sections.len(), 1);
        assert!(l.rows.iter().all(|r| r.kind == RowKind::Tiles));
    }

    #[test]
    fn finds_the_row_holding_an_asset() {
        let items: Vec<LayoutItem> = (0..30).map(|i| at(i, TUE - i * 86_400 * 20)).collect();
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Month));
        let row = l.row_of(17).expect("asset 17 is laid out");
        assert!(l.rows[row as usize].tiles.iter().any(|t| t.id == 17));
        assert_eq!(l.row_of(12345), None);
    }

    /// The viewer pages through a grid in the order it is drawn, across rows and sections,
    /// and stops at its ends.
    /// Round 5: shift-click selects everything between two tiles as the grid draws them, in
    /// either direction, however far apart (not only the rows the page has loaded).
    #[test]
    fn ids_between_two_tiles_follow_the_layout() {
        let items: Vec<LayoutItem> = (0..300).map(|i| at(i, TUE - i * 86_400 * 3)).collect();
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Month));
        let order: Vec<i64> = l.rows.iter().flat_map(|r| r.tiles.iter().map(|t| t.id)).collect();
        assert_eq!(l.ids_between(order[2], order[250]), Some(order[2..=250].to_vec()));
        assert_eq!(l.ids_between(order[250], order[2]), Some(order[2..=250].to_vec()), "backwards too");
        assert_eq!(l.ids_between(order[7], order[7]), Some(vec![order[7]]));
        assert_eq!(l.ids_between(order[7], 12345), None);
    }

    #[test]
    fn neighbours_follow_the_layout_across_rows() {
        let items: Vec<LayoutItem> = (0..30).map(|i| at(i, TUE - i * 86_400 * 20)).collect();
        let l = LayoutIndex { items }.compute(1000, 200, 4, Some(GroupBy::Month));
        let order: Vec<i64> = l.rows.iter().flat_map(|r| r.tiles.iter().map(|t| t.id)).collect();
        assert_eq!(order.len(), 30);
        assert_eq!(l.neighbors_of(order[0]), Some(Neighbors { prev: None, next: Some(order[1]) }));
        assert_eq!(l.neighbors_of(order[29]), Some(Neighbors { prev: Some(order[28]), next: None }));
        // The last tile of one row leads to the first of the next, over any header between.
        let first_row = l.rows.iter().find(|r| r.kind == RowKind::Tiles).unwrap();
        let last = first_row.tiles.last().unwrap().id;
        let i = order.iter().position(|&id| id == last).unwrap();
        assert_eq!(l.neighbors_of(last), Some(Neighbors { prev: order.get(i.wrapping_sub(1)).copied(), next: Some(order[i + 1]) }));
        assert_eq!(l.neighbors_of(12345), None);
    }

    #[test]
    fn a_late_evening_photo_stays_on_the_day_it_was_taken() {
        // captured_at is the camera's wall clock stored as if it were UTC: 23:30 on 15 September
        // is the 15th wherever the computer is. Only imported_at (true UTC) takes the offset.
        let evening = 1_757_979_000; // 2025-09-15 23:30 on the camera's clock
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at)
                 VALUES (1, 'a', 'p', 'a.jpg', 'jpg', 0, 1, 1, ?1, 1), (2, 'b', 'p', 'b.jpg', 'jpg', 0, 1, 1, NULL, ?1)",
                [evening],
            )?;
            let index = LayoutIndex::load_scoped(c, &LayoutScope::library(), true)?;
            let by_id = |id: i64| index.items.iter().find(|i| i.id == id).unwrap().clone();
            assert_eq!(by_id(1).day, day_key(evening, 0), "a capture time is already local");
            assert_eq!(by_id(1).month, month_key(evening, 0));
            assert_eq!(by_id(2).day, day_key(evening, local_offset_secs()), "an import time is UTC");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn day_keys_follow_the_local_offset() {
        // 2025-09-15T23:30Z is still the 15th at UTC but the 16th at +02:00.
        let ts = 1_757_979_000;
        assert_eq!(day_key(ts, 7200), day_key(ts, 0) + 1);
        assert_eq!(day_title_for_key(day_key(TUE, 0)), "Tuesday, 16 September 2025");
        assert_eq!(day_title_for_key(0), "Thursday, 1 January 1970");
    }
}
