//! What the map shows: clusters of geotagged assets inside a viewport.
//!
//! Points are snapped to a grid whose cell size follows the zoom, then each cell reports
//! its centroid, its count and its newest asset as the cover. SQLite does the grouping,
//! so the work stays where the data is and the wire carries a few hundred clusters rather
//! than a hundred thousand points.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::scope::LayoutScope;
use crate::Result;

/// A rectangle of the world. `min_lon > max_lon` means it crosses the antimeridian.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bounds {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

impl Bounds {
    pub fn world() -> Self {
        Self { min_lat: -90.0, max_lat: 90.0, min_lon: -180.0, max_lon: 180.0 }
    }

    /// Clamped to the valid range, and ordered, so a dragged viewport cannot produce a
    /// query that selects nothing by accident.
    pub fn normalized(self) -> Self {
        let (min_lat, max_lat) = if self.min_lat <= self.max_lat { (self.min_lat, self.max_lat) } else { (self.max_lat, self.min_lat) };
        Self {
            min_lat: min_lat.clamp(-90.0, 90.0),
            max_lat: max_lat.clamp(-90.0, 90.0),
            min_lon: self.min_lon.clamp(-180.0, 180.0),
            max_lon: self.max_lon.clamp(-180.0, 180.0),
        }
    }

    pub fn crosses_antimeridian(&self) -> bool {
        self.min_lon > self.max_lon
    }
}

/// One pin on the map: either a single photo or a group standing in for many.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Cluster {
    pub lat: f64,
    pub lon: f64,
    #[ts(type = "number")]
    pub count: i64,
    /// Newest asset in the cell, used as the marker's thumbnail.
    #[ts(type = "number")]
    pub cover_asset_id: i64,
    /// Set only when the cluster holds exactly one asset, so a click can open the viewer.
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
    /// The cell, so a click can zoom to exactly what it contains.
    pub bounds: Bounds,
}

/// Degrees per grid cell at `zoom` (fractional: the map zooms smoothly). Four cells per
/// 256px tile is 64px, about a marker; the cell never drops below a metre or so.
pub fn cell_size(zoom: f64) -> f64 {
    (360.0 / (2f64.powf(zoom.clamp(0.0, 22.0)) * 4.0)).max(0.000_01)
}

/// Closest two marker centres may be, in screen pixels: a 52px thumbnail plus a gap.
pub const MARKER_PX: f64 = 60.0;

/// Web Mercator pixel position at `zoom` (the world is `256 · 2^zoom` wide).
pub fn world_px(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
    let size = 256.0 * 2f64.powf(zoom);
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let x = (lon + 180.0) / 360.0 * size;
    let y = (0.5 - (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln() / (2.0 * std::f64::consts::PI)) * size;
    (x, y)
}

/// Merge clusters whose markers would overlap on screen at `zoom`.
///
/// Grid cells alone cannot promise that: two photos either side of a cell boundary land in
/// different cells however close they are. The biggest groups absorb their neighbours
/// first, so a busy place keeps its position and a stray photo joins it rather than the
/// other way round; merging moves a centroid, so it repeats until nothing is too close.
fn merge_overlapping(mut clusters: Vec<Cluster>, zoom: f64) -> Vec<Cluster> {
    loop {
        clusters.sort_by(|a, b| b.count.cmp(&a.count).then(b.cover_asset_id.cmp(&a.cover_asset_id)));
        let mut kept: Vec<(Cluster, (f64, f64))> = Vec::with_capacity(clusters.len());
        let mut merged_any = false;
        for c in clusters {
            let p = world_px(c.lat, c.lon, zoom);
            let near = kept.iter_mut().find(|(_, q)| (p.0 - q.0).hypot(p.1 - q.1) < MARKER_PX);
            match near {
                Some((k, q)) => {
                    let n = (k.count + c.count) as f64;
                    k.lat = (k.lat * k.count as f64 + c.lat * c.count as f64) / n;
                    k.lon = (k.lon * k.count as f64 + c.lon * c.count as f64) / n;
                    k.count += c.count;
                    k.cover_asset_id = k.cover_asset_id.max(c.cover_asset_id);
                    k.asset_id = None;
                    k.bounds = Bounds {
                        min_lat: k.bounds.min_lat.min(c.bounds.min_lat),
                        max_lat: k.bounds.max_lat.max(c.bounds.max_lat),
                        min_lon: k.bounds.min_lon.min(c.bounds.min_lon),
                        max_lon: k.bounds.max_lon.max(c.bounds.max_lon),
                    };
                    *q = world_px(k.lat, k.lon, zoom);
                    merged_any = true;
                }
                None => kept.push((c, p)),
            }
        }
        clusters = kept.into_iter().map(|(c, _)| c).collect();
        if !merged_any {
            return clusters;
        }
    }
}

/// Clusters inside `bounds`, honouring the same scope the grid uses. A viewport that
/// crosses the antimeridian is served as two queries and merged.
pub fn map_clusters(conn: &Connection, bounds: Bounds, zoom: f64, scope: &LayoutScope) -> Result<Vec<Cluster>> {
    let bounds = bounds.normalized();
    let cells = if bounds.crosses_antimeridian() {
        let west = Bounds { max_lon: 180.0, ..bounds };
        let east = Bounds { min_lon: -180.0, ..bounds };
        let mut out = clusters_in(conn, west, zoom, scope)?;
        out.extend(clusters_in(conn, east, zoom, scope)?);
        out
    } else {
        clusters_in(conn, bounds, zoom, scope)?
    };
    Ok(merge_overlapping(cells, zoom))
}

/// The cell index of a coordinate, in portable SQL.
///
/// This SQLite is built without the optional math functions, so there is no `FLOOR`, and
/// a plain CAST truncates toward zero — which would fold the cell just south of the
/// equator into the one just north of it. Shifting the coordinate into positive space
/// first (latitude by 90, longitude by 180) makes truncation *equal* floor, at one
/// division per row instead of the three a sign-corrected expression needs. At 250k rows
/// that difference is most of the query.
#[cfg(test)]
fn cell_index(column: &str, offset: f64, cell: f64) -> String {
    format!("CAST(({column} + {offset}) / {cell} AS INTEGER)")
}

fn clusters_in(conn: &Connection, b: Bounds, zoom: f64, scope: &LayoutScope) -> Result<Vec<Cluster>> {
    let cell = cell_size(zoom);
    let (from_where, _, mut params) = scope.sql();
    params.push(b.min_lat.into());
    params.push(b.max_lat.into());
    params.push(b.min_lon.into());
    params.push(b.max_lon.into());
    // The grouping happens here rather than in SQL. `GROUP BY` on computed cell keys
    // makes SQLite build a temporary B-tree, which cost more than the scan itself; this
    // way the statement is a straight walk of the covering index and the buckets are a
    // hash map. Measured at 250k rows: roughly half the time.
    let sql = format!(
        "SELECT a.gps_lat, a.gps_lon, a.id
         {from_where}
           AND a.gps_lat IS NOT NULL
           AND a.gps_lat BETWEEN ? AND ?
           AND a.gps_lon BETWEEN ? AND ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter()))?;

    struct Cell {
        count: i64,
        lat_sum: f64,
        lon_sum: f64,
        newest: i64,
    }
    let mut cells: std::collections::HashMap<(i32, i32), Cell> = std::collections::HashMap::new();
    while let Some(r) = rows.next()? {
        let lat: f64 = r.get(0)?;
        let lon: f64 = r.get(1)?;
        let id: i64 = r.get(2)?;
        // Shift into positive space so a truncating cast is a floor.
        let key = (((lat + 90.0) / cell) as i32, ((lon + 180.0) / cell) as i32);
        let e = cells.entry(key).or_insert(Cell { count: 0, lat_sum: 0.0, lon_sum: 0.0, newest: id });
        e.count += 1;
        e.lat_sum += lat;
        e.lon_sum += lon;
        e.newest = e.newest.max(id);
    }

    Ok(cells
        .into_iter()
        .map(|((cy, cx), c)| Cluster {
            lat: c.lat_sum / c.count as f64,
            lon: c.lon_sum / c.count as f64,
            count: c.count,
            cover_asset_id: c.newest,
            asset_id: if c.count == 1 { Some(c.newest) } else { None },
            bounds: Bounds {
                min_lat: cy as f64 * cell - 90.0,
                max_lat: (cy + 1) as f64 * cell - 90.0,
                min_lon: cx as f64 * cell - 180.0,
                max_lon: (cx + 1) as f64 * cell - 180.0,
            },
        })
        .collect())
}

/// The box that holds every geotagged asset in `scope`, for "fit to library".
pub fn map_bounds(conn: &Connection, scope: &LayoutScope) -> Result<Option<Bounds>> {
    let (from_where, _, params) = scope.sql();
    let sql = format!(
        "SELECT MIN(a.gps_lat), MAX(a.gps_lat), MIN(a.gps_lon), MAX(a.gps_lon) {from_where} AND a.gps_lat IS NOT NULL"
    );
    let row: (Option<f64>, Option<f64>, Option<f64>, Option<f64>) =
        conn.query_row(&sql, rusqlite::params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    Ok(match row {
        (Some(min_lat), Some(max_lat), Some(min_lon), Some(max_lon)) => Some(Bounds { min_lat, max_lat, min_lon, max_lon }),
        _ => None,
    })
}

/// Asset ids inside `bounds`, newest first — the strip under the map.
pub fn assets_in_bounds(conn: &Connection, bounds: Bounds, scope: &LayoutScope, limit: i64) -> Result<Vec<i64>> {
    let b = bounds.normalized();
    let (from_where, order, mut params) = scope.sql();
    params.push(b.min_lat.into());
    params.push(b.max_lat.into());
    params.push(b.min_lon.into());
    params.push(b.max_lon.into());
    params.push(limit.into());
    let sql = format!(
        "SELECT a.id {from_where}
           AND a.gps_lat IS NOT NULL
           AND a.gps_lat BETWEEN ? AND ?
           AND a.gps_lon BETWEEN ? AND ?
         ORDER BY {order} LIMIT ?"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| r.get(0))?
        .collect::<std::result::Result<Vec<i64>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Db;

    fn insert(c: &Connection, id: i64, lat: Option<f64>, lon: Option<f64>) {
        c.execute(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, gps_lat, gps_lon)
             VALUES (?1, ?2, ?2, ?2, 'jpg', 0, 1, 1, ?1, ?3, ?4)",
            rusqlite::params![id, format!("h{id}"), lat, lon],
        )
        .unwrap();
    }

    #[test]
    #[ignore = "diagnostic: prints the cluster query plan"]
    fn explains_the_cluster_query() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 50_000, 3)?;
            crate::db::analyze(c)?;
            let cell = cell_size(3.0);
            let (cy, cx) = (cell_index("a.gps_lat", 90.0, cell), cell_index("a.gps_lon", 180.0, cell));
            let (from_where, _, _) = LayoutScope::library().sql();
            let sql = format!(
                "SELECT {cy} AS cy, {cx} AS cx, COUNT(*), AVG(a.gps_lat), AVG(a.gps_lon), MAX(a.id)
                 {from_where} AND a.gps_lat IS NOT NULL AND a.gps_lat BETWEEN -90 AND 90 AND a.gps_lon BETWEEN -180 AND 180
                 GROUP BY cy, cx"
            );
            let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(3))?;
            for r in rows {
                println!("PLAN: {}", r?);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn cell_indexes_step_evenly_across_zero() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Latitudes one cell apart must land in consecutive cells, including across
            // the equator where a truncating cast would otherwise repeat a cell.
            let mut last: Option<i64> = None;
            for step in 0..6 {
                let lat = -2.5 + step as f64;
                let sql = format!("SELECT {}", super::cell_index(&format!("({lat})"), 90.0, 1.0));
                let got: i64 = c.query_row(&sql, [], |r| r.get(0))?;
                if let Some(prev) = last {
                    assert_eq!(got, prev + 1, "lat {lat} should be one cell past the last");
                }
                last = Some(got);
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn cells_shrink_as_you_zoom_in() {
        assert!(cell_size(0.0) > cell_size(10.0));
        assert!(cell_size(10.0) > cell_size(18.0));
        assert!(cell_size(10.0) > cell_size(10.5), "fractional zoom counts");
        assert_eq!(cell_size(0.0), 90.0);
        assert!(cell_size(22.0) > 0.0, "never collapses to zero");
    }

    /// Grid cells alone let two markers sit on top of each other whenever their photos
    /// fall either side of a cell boundary. Markers closer than a marker's width merge.
    #[test]
    fn markers_that_would_overlap_on_screen_become_one() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Longitude 0 is a cell boundary at every zoom. At zoom 10 these are about 29px
            // apart: different cells, overlapping markers.
            insert(c, 1, Some(0.0), Some(-0.02));
            insert(c, 2, Some(0.0), Some(0.02));
            // About 218px away: clearly its own place.
            insert(c, 3, Some(0.0), Some(0.32));
            let got = map_clusters(c, Bounds::world(), 10.0, &LayoutScope::library())?;
            assert_eq!(got.len(), 2, "{got:?}");
            let merged = got.iter().find(|k| k.count == 2).expect("the two close photos merge");
            assert!(merged.lon.abs() < 0.001, "the merged marker sits between them: {}", merged.lon);
            assert!(merged.bounds.min_lon <= -0.02 && merged.bounds.max_lon >= 0.02, "zooming to it shows both");
            assert_eq!(merged.cover_asset_id, 2);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn no_two_markers_overlap_anywhere() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 3_000, 11)?;
            for zoom in [2.0, 4.5, 7.0, 11.3] {
                let got = map_clusters(c, Bounds::world(), zoom, &LayoutScope::library())?;
                let px = |k: &Cluster| world_px(k.lat, k.lon, zoom);
                for (i, a) in got.iter().enumerate() {
                    for b in &got[i + 1..] {
                        let (pa, pb) = (px(a), px(b));
                        let d = ((pa.0 - pb.0).powi(2) + (pa.1 - pb.1).powi(2)).sqrt();
                        assert!(d >= MARKER_PX, "zoom {zoom}: markers {d:.1}px apart");
                    }
                }
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn groups_neighbours_and_keeps_single_photos_clickable() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Two photos a few metres apart in Lisbon, one in Madrid, one with no GPS.
            insert(c, 1, Some(38.7223), Some(-9.1393));
            insert(c, 2, Some(38.7224), Some(-9.1394));
            insert(c, 3, Some(40.4168), Some(-3.7038));
            insert(c, 4, None, None);

            let all = map_clusters(c, Bounds::world(), 5.0, &LayoutScope::library())?;
            assert_eq!(all.len(), 2, "Lisbon and Madrid, and never the photo without GPS");
            let lisbon = all.iter().find(|c| c.lon < -5.0).unwrap();
            assert_eq!(lisbon.count, 2);
            assert_eq!(lisbon.asset_id, None, "a group is not one photo");
            assert_eq!(lisbon.cover_asset_id, 2, "newest is the cover");
            assert!((lisbon.lat - 38.72235).abs() < 0.001, "centroid, not a corner");
            let madrid = all.iter().find(|c| c.lon > -5.0).unwrap();
            assert_eq!((madrid.count, madrid.asset_id), (1, Some(3)));

            // Zoomed right in, the two Lisbon photos separate.
            let close = map_clusters(c, Bounds::world(), 20.0, &LayoutScope::library())?;
            assert_eq!(close.len(), 3);

            // A viewport around Madrid excludes Lisbon.
            let near_madrid = Bounds { min_lat: 40.0, max_lat: 41.0, min_lon: -4.0, max_lon: -3.0 };
            assert_eq!(map_clusters(c, near_madrid, 8.0, &LayoutScope::library())?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reports_bounds_and_the_assets_inside_them() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            assert_eq!(map_bounds(c, &LayoutScope::library())?, None, "nothing geotagged yet");
            insert(c, 1, Some(38.0), Some(-9.0));
            insert(c, 2, Some(40.0), Some(-3.0));
            let b = map_bounds(c, &LayoutScope::library())?.unwrap();
            assert_eq!((b.min_lat, b.max_lat, b.min_lon, b.max_lon), (38.0, 40.0, -9.0, -3.0));
            assert_eq!(assets_in_bounds(c, b, &LayoutScope::library(), 10)?, vec![2, 1], "newest first");
            let only_lisbon = Bounds { min_lat: 37.0, max_lat: 39.0, min_lon: -10.0, max_lon: -8.0 };
            assert_eq!(assets_in_bounds(c, only_lisbon, &LayoutScope::library(), 10)?, vec![1]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn normalises_a_dragged_viewport() {
        let upside_down = Bounds { min_lat: 50.0, max_lat: 10.0, min_lon: -20.0, max_lon: 20.0 }.normalized();
        assert_eq!((upside_down.min_lat, upside_down.max_lat), (10.0, 50.0));
        let too_big = Bounds { min_lat: -200.0, max_lat: 200.0, min_lon: -400.0, max_lon: 400.0 }.normalized();
        assert_eq!((too_big.min_lat, too_big.max_lat), (-90.0, 90.0));
        assert!(Bounds { min_lon: 170.0, max_lon: -170.0, min_lat: 0.0, max_lat: 10.0 }.crosses_antimeridian());
    }
}
