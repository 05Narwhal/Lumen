//! `lumen:plugin/library` (Round 24 §8.5, `library.read`): the library read-only, 200 assets a
//! page, keyset by id (no `OFFSET`). Trashed assets and the movie half of a Live Photo are not
//! offered.

use std::sync::Arc;

use chrono::NaiveDateTime;
use photos_core::assets::{MEDIA_TYPE_LIVE, PLACE_SOURCE_ESTIMATED};
use photos_core::Library;
use photos_plugins::host::{Asset, AssetDetails, AssetFilter, AssetPage, Gps, GpsSource, LibraryHost, MediaKind};
use rusqlite::types::Value;

pub const PAGE: usize = 200;
/// What `details` returns of a photo's text: lines and characters a line.
const TEXT_LINES: usize = 12;
const TEXT_CHARS: usize = 120;
/// A line the reader was less sure of than this is left out.
const TEXT_CONFIDENCE: f64 = 0.6;
const WALL_CLOCK: &str = "%Y-%m-%dT%H:%M:%S";

/// The person's current selection, read when a filter asks for it.
pub type Selection = Arc<dyn Fn() -> Vec<i64> + Send + Sync>;

pub struct LibraryAssets {
    lib: Arc<Library>,
    selection: Selection,
}

impl LibraryAssets {
    pub fn new(lib: Arc<Library>, selection: Selection) -> Self {
        Self { lib, selection }
    }
}

/// `captured_at` is the camera's wall clock stored as if it were UTC (CLAUDE.md, *Dates*).
fn wall_clock(unix: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(unix, 0).map(|d| d.naive_utc().format(WALL_CLOCK).to_string())
}

fn parse_wall_clock(s: &str) -> Result<i64, String> {
    NaiveDateTime::parse_from_str(s, WALL_CLOCK).map(|d| d.and_utc().timestamp()).map_err(|_| format!("'{s}' isn't a time like 2026-03-12T14:02:00"))
}

/// Scene tags are stored joined by spaces and some have spaces ("group of people"), so they are
/// read back by matching the known list.
fn scene_list(stored: &str) -> Vec<String> {
    let padded = format!(" {stored} ");
    photos_core::embedding::SCENE_TAGS.iter().filter(|t| padded.contains(&format!(" {t} "))).map(|t| t.to_string()).collect()
}

fn lines(stored: Option<String>) -> Vec<String> {
    stored.map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()).unwrap_or_default()
}

impl LibraryHost for LibraryAssets {
    fn details(&self, ids: &[u64]) -> Result<Vec<AssetDetails>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if ids.len() > PAGE {
            return Err(format!("details takes at most {PAGE} assets a call"));
        }
        // Integers we were given as u64, so they go in as literals (as the selection does).
        let list = ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        self.lib
            .db
            .with_conn(|c| {
                let mut out: Vec<AssetDetails> = Vec::new();
                {
                    let mut stmt = c.prepare(&format!(
                        "SELECT id, capture_source, title, caption, keywords, scene_tags FROM assets
                         WHERE id IN ({list}) AND state = 0 AND trashed_at IS NULL ORDER BY id"
                    ))?;
                    let rows = stmt.query_map([], |r| {
                        Ok(AssetDetails {
                            id: r.get::<_, i64>(0)? as u64,
                            capture_source: r.get(1)?,
                            title: r.get::<_, Option<String>>(2)?.filter(|s| !s.trim().is_empty()),
                            caption: r.get::<_, Option<String>>(3)?.filter(|s| !s.trim().is_empty()),
                            keywords: lines(r.get(4)?),
                            scene: r.get::<_, Option<String>>(5)?.map(|s| scene_list(&s)).unwrap_or_default(),
                            people: Vec::new(),
                            albums: Vec::new(),
                            text: Vec::new(),
                        })
                    })?;
                    for row in rows {
                        out.push(row?);
                    }
                }
                let slot = |out: &mut Vec<AssetDetails>, id: i64| out.binary_search_by_key(&(id as u64), |d| d.id).ok();
                // People who are named and listed, on faces that are not refused.
                let mut stmt = c.prepare(&format!(
                    "SELECT DISTINCT f.asset_id, p.name FROM faces f JOIN people p ON p.id = f.person_id
                     WHERE f.asset_id IN ({list}) AND f.rejected IS NULL AND p.status = 'listed'
                       AND p.name IS NOT NULL AND TRIM(p.name) <> '' ORDER BY f.asset_id, p.name"
                ))?;
                for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                    let (id, name) = row?;
                    if let Some(i) = slot(&mut out, id) {
                        out[i].people.push(name);
                    }
                }
                let mut stmt = c.prepare(&format!(
                    "SELECT ai.asset_id, al.name FROM album_items ai JOIN albums al ON al.id = ai.album_id
                     WHERE ai.asset_id IN ({list}) ORDER BY ai.asset_id, al.name"
                ))?;
                for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                    let (id, name) = row?;
                    if let Some(i) = slot(&mut out, id) {
                        out[i].albums.push(name);
                    }
                }
                let mut stmt = c.prepare(&format!(
                    "SELECT asset_id, text FROM asset_text WHERE asset_id IN ({list}) AND conf >= {TEXT_CONFIDENCE} ORDER BY asset_id, ord"
                ))?;
                for row in stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                    let (id, text) = row?;
                    if let Some(i) = slot(&mut out, id) {
                        let line: String = text.trim().chars().take(TEXT_CHARS).collect();
                        if !line.is_empty() && out[i].text.len() < TEXT_LINES {
                            out[i].text.push(line);
                        }
                    }
                }
                Ok(out)
            })
            .map_err(|e| e.to_string())
    }

    fn assets(&self, f: &AssetFilter, cursor: Option<&str>) -> Result<AssetPage, String> {
        let mut sql = String::from(
            "SELECT a.id, a.captured_at, a.tz_offset_minutes, a.filename, a.source_dir, a.media_type, a.gps_lat, a.gps_lon,
                    a.place_source, a.place_name, a.live_photo_group, d.display_name
             FROM assets a LEFT JOIN devices d ON d.id = a.device_id
             WHERE a.state = 0 AND a.trashed_at IS NULL AND NOT (a.media_type = 1 AND a.live_photo_group IS NOT NULL)",
        );
        let mut args: Vec<Value> = Vec::new();
        let mut and = |clause: &str, v: Option<Value>| {
            sql.push_str(" AND ");
            sql.push_str(clause);
            args.extend(v);
        };
        if let Some(c) = cursor {
            and("a.id > ?", Some(Value::Integer(c.parse::<i64>().map_err(|_| "bad cursor".to_string())?)));
        }
        match f.has_gps {
            Some(true) => and("a.gps_lat IS NOT NULL AND a.gps_lon IS NOT NULL", None),
            Some(false) => and("(a.gps_lat IS NULL OR a.gps_lon IS NULL)", None),
            None => {}
        }
        if let Some(s) = &f.since {
            and("a.captured_at >= ?", Some(Value::Integer(parse_wall_clock(s)?)));
        }
        if let Some(s) = &f.until {
            and("a.captured_at <= ?", Some(Value::Integer(parse_wall_clock(s)?)));
        }
        if let Some(album) = f.album {
            and("a.id IN (SELECT asset_id FROM album_items WHERE album_id = ?)", Some(Value::Integer(album)));
        }
        if f.selection {
            // Ids are integers we read ourselves, so they go in as literals.
            let ids: Vec<String> = (self.selection)().iter().map(i64::to_string).collect();
            if ids.is_empty() {
                return Ok(AssetPage { items: Vec::new(), next_cursor: None });
            }
            and(&format!("a.id IN ({})", ids.join(",")), None);
        }
        sql.push_str(&format!(" ORDER BY a.id LIMIT {}", PAGE + 1));

        let mut items = self
            .lib
            .db
            .with_conn(|c| {
                let mut stmt = c.prepare_cached(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
                    let (lat, lon): (Option<f64>, Option<f64>) = (r.get(6)?, r.get(7)?);
                    let source: i64 = r.get(8)?;
                    let kind: i64 = r.get(5)?;
                    Ok(Asset {
                        id: r.get::<_, i64>(0)? as u64,
                        captured_at: r.get::<_, Option<i64>>(1)?.and_then(wall_clock),
                        offset_minutes: r.get(2)?,
                        file_name: r.get(3)?,
                        folder: r.get::<_, Option<String>>(4)?.filter(|s| !s.is_empty()),
                        kind: if kind == 1 { MediaKind::Video } else { MediaKind::Photo },
                        gps: lat.zip(lon).map(|(lat, lon)| Gps {
                            lat,
                            lon,
                            source: if source == PLACE_SOURCE_ESTIMATED { GpsSource::Estimated } else { GpsSource::Camera },
                        }),
                        place: r.get::<_, Option<String>>(9)?.filter(|s| !s.is_empty()),
                        group: r.get(10)?,
                        camera: r.get(11)?,
                    })
                })?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .map_err(|e| e.to_string())?;
        let more = items.len() > PAGE;
        items.truncate(PAGE);
        let next_cursor = more.then(|| items.last().map(|a| a.id.to_string())).flatten();
        let _ = MEDIA_TYPE_LIVE; // a Live Photo's still is a photo: `kind` is photo or video only
        Ok(AssetPage { items, next_cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(rows: usize) -> (tempfile::TempDir, Arc<Library>) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Arc::new(Library::create(dir.path()).unwrap());
        lib.db
            .with_conn(|c| {
                for i in 1..=rows as i64 {
                    // Every third has GPS; every ninth is the camera's, the others estimated.
                    let (lat, lon, src) = if i % 3 == 0 { (Some(46.0), Some(14.0), if i % 9 == 0 { 0 } else { 3 }) } else { (None, None, 0) };
                    c.execute(
                        "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, imported_at,
                                             captured_at, gps_lat, gps_lon, place_source, source_dir)
                         VALUES (?1, ?2, 'x', ?3, 'jpg', 0, 1, 1, ?4, ?5, ?6, ?7, 'DCIM')",
                        rusqlite::params![i, format!("h{i}"), format!("IMG_{i}.jpg"), 1_700_000_000 + i * 60, lat, lon, src],
                    )?;
                }
                Ok(())
            })
            .unwrap();
        (dir, lib)
    }

    fn filter() -> AssetFilter {
        AssetFilter { has_gps: None, since: None, until: None, album: None, selection: false }
    }

    #[test]
    fn library_page_keyset() {
        let (_d, lib) = library(450);
        let host = LibraryAssets::new(lib, Arc::new(Vec::new));
        let p1 = host.assets(&filter(), None).unwrap();
        assert_eq!((p1.items.len(), p1.next_cursor.as_deref()), (200, Some("200")));
        let p2 = host.assets(&filter(), p1.next_cursor.as_deref()).unwrap();
        assert_eq!((p2.items.first().unwrap().id, p2.items.len(), p2.next_cursor.as_deref()), (201, 200, Some("400")));
        let p3 = host.assets(&filter(), p2.next_cursor.as_deref()).unwrap();
        assert_eq!((p3.items.len(), p3.next_cursor), (50, None));

        // The first asset: its wall clock, folder and no GPS.
        let a = &p1.items[0];
        assert_eq!((a.file_name.as_str(), a.folder.as_deref(), a.gps.is_none()), ("IMG_1.jpg", Some("DCIM"), true));
        assert_eq!(a.captured_at.as_deref(), Some("2023-11-14T22:14:20"));
    }

    #[test]
    fn library_filters() {
        let (_d, lib) = library(30);
        let host = LibraryAssets::new(lib, Arc::new(|| vec![3, 4, 6]));
        let ids = |f: AssetFilter| host.assets(&f, None).unwrap().items.iter().map(|a| a.id).collect::<Vec<_>>();
        assert_eq!(ids(AssetFilter { has_gps: Some(true), ..filter() }).len(), 10);
        assert_eq!(ids(AssetFilter { has_gps: Some(false), ..filter() }).len(), 20);
        assert_eq!(ids(AssetFilter { selection: true, ..filter() }), vec![3, 4, 6]);
        assert_eq!(ids(AssetFilter { selection: true, has_gps: Some(true), ..filter() }), vec![3, 6]);
        // since / until are inclusive wall-clock bounds: asset 2 is 22:15:20, asset 4 22:17:20.
        let f = AssetFilter { since: Some("2023-11-14T22:15:20".into()), until: Some("2023-11-14T22:17:20".into()), ..filter() };
        assert_eq!(ids(f), vec![2, 3, 4]);
        assert!(host.assets(&AssetFilter { since: Some("yesterday".into()), ..filter() }, None).is_err());
        // Source: asset 3 estimated, asset 9 the camera's.
        let all = host.assets(&AssetFilter { has_gps: Some(true), ..filter() }, None).unwrap().items;
        let src = |id: u64| all.iter().find(|a| a.id == id).unwrap().gps.as_ref().unwrap().source;
        assert_eq!((src(3), src(9)), (GpsSource::Estimated, GpsSource::Camera));
    }

    #[test]
    fn details_gather_people_albums_text_and_scene() {
        let (_d, lib) = library(3);
        lib.db
            .with_conn(|c| {
                c.execute("UPDATE assets SET title = 'Dawn', caption = '  ', keywords = 'iceland
waterfall', scene_tags = 'group of people sea', capture_source = 'original' WHERE id = 1", [])?;
                c.execute("INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 1), (2, NULL, 1)", [])?;
                c.execute("INSERT INTO people (id, name, status, created_at) VALUES (3, 'Gone', 'removed', 1)", [])?;
                for (asset, person, rejected) in [(1, 1, None), (1, 1, None), (1, 2, None), (1, 3, None), (2, 1, Some("x"))] {
                    c.execute(
                        "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id, rejected) VALUES (?1, 0, 0, 1, 1, 1, x'00', 'm', ?2, ?3)",
                        rusqlite::params![asset, person, rejected],
                    )?;
                }
                c.execute("INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (1, 'Iceland', 0, 1, 1)", [])?;
                c.execute("INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (1, 1, 1, 1)", [])?;
                for (ord, text, conf) in [(0, "  OPEN 24h ", 0.9), (1, "smudge", 0.3), (2, "Exit", 0.8)] {
                    c.execute(
                        "INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (1, ?1, ?2, ?3, x'00', 'Latn')",
                        rusqlite::params![ord, text, conf],
                    )?;
                }
                c.execute("UPDATE assets SET trashed_at = 5 WHERE id = 3", [])?;
                Ok(())
            })
            .unwrap();
        let host = LibraryAssets::new(lib, Arc::new(Vec::new));
        let got = host.details(&[1, 2, 3, 99]).unwrap();
        assert_eq!(got.iter().map(|d| d.id).collect::<Vec<_>>(), vec![1, 2], "trashed and unknown ids are left out");
        let a = &got[0];
        assert_eq!((a.title.as_deref(), a.caption.as_deref(), a.capture_source.as_deref()), (Some("Dawn"), None, Some("original")));
        assert_eq!((a.keywords.as_slice(), a.scene.as_slice()), (&["iceland".to_string(), "waterfall".to_string()][..], &["group of people".to_string(), "sea".to_string()][..]));
        assert_eq!(a.people, vec!["Anna"], "a person once, only named and listed");
        assert_eq!((a.albums.as_slice(), a.text.as_slice()), (&["Iceland".to_string()][..], &["OPEN 24h".to_string(), "Exit".to_string()][..]));
        assert!(got[1].people.is_empty(), "a refused face names nobody");
        assert!(host.details(&(0..201).collect::<Vec<u64>>()).is_err());
    }

    #[test]
    fn empty_selection_is_an_empty_page() {
        let (_d, lib) = library(5);
        let host = LibraryAssets::new(lib, Arc::new(Vec::new));
        assert!(host.assets(&AssetFilter { selection: true, ..filter() }, None).unwrap().items.is_empty());
    }
}
