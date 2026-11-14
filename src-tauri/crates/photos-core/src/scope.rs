//! What a grid shows: the library, one device, one album, favorites or the trash,
//! optionally narrowed to photos or videos and to a search. One type, one SQL builder,
//! one cache key.

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::assets::TIMELINE_ORDER;
use crate::query::AssetQuery;

/// Settings → Library → *Hide RAW files that have a JPEG*: set by the app from the settings,
/// read by every scope but the Trash. Process-wide so every view (grid, counts, map, strip)
/// follows it; the app drops its layout cache when it changes.
static HIDE_RAW_WITH_JPEG: AtomicBool = AtomicBool::new(false);

pub fn set_hide_raw_with_jpeg(on: bool) {
    HIDE_RAW_WITH_JPEG.store(on, Ordering::Relaxed);
}

pub fn hide_raw_with_jpeg() -> bool {
    HIDE_RAW_WITH_JPEG.load(Ordering::Relaxed)
}

/// A RAW file the camera also saved as a JPEG (or HEIC/PNG/TIFF): same name before the
/// extension, same capture time.
fn raw_with_companion() -> String {
    let stem = |t: &str| format!("lower(substr({t}.filename, 1, length({t}.filename) - length({t}.extension)))");
    format!(
        "(lower(a.extension) IN {raw} AND EXISTS (SELECT 1 FROM assets j WHERE j.state = 0 AND j.captured_at = a.captured_at AND j.id != a.id          AND lower(j.extension) IN ('jpg','jpeg','heic','heif','png','tif','tiff') AND {} = {}))",
        stem("j"),
        stem("a"),
        raw = crate::formats::raw_sql_list(),
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS, Default)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LayoutScope {
    /// 0 photos, 1 videos, None all
    #[ts(type = "number | null")]
    #[serde(default)]
    pub media_type: Option<i64>,
    #[ts(type = "number | null")]
    #[serde(default)]
    pub device_id: Option<i64>,
    #[ts(type = "number | null")]
    #[serde(default)]
    pub album_id: Option<i64>,
    #[serde(default)]
    pub favorites: bool,
    #[serde(default)]
    pub trash: bool,
    /// Albums keep their manual order unless this is set.
    #[serde(default)]
    pub sort_by_date: bool,
    /// Free text and structured filters. `None` and an empty query mean the same thing.
    #[serde(default)]
    pub query: Option<AssetQuery>,
    /// A memory's curated set (its *Highlights*), in timeline order.
    #[ts(type = "number | null")]
    #[serde(default)]
    pub memory_id: Option<i64>,
    /// One person's photos (Phase 7b).
    #[ts(type = "number | null")]
    #[serde(default)]
    pub person_id: Option<i64>,
    /// One pet's photos (Round 4C).
    #[ts(type = "number | null")]
    #[serde(default)]
    pub pet_id: Option<i64>,
    /// The Hidden folder (round 3): only its photos.
    #[serde(default)]
    pub vault: bool,
    /// A search's *Best match* order (Round 4K): the query's `rank` first, best first, then
    /// everything else in timeline order, as one untitled run.
    #[serde(default)]
    pub best_match: bool,
    /// An album collection's photos (Round 19 §10): the layout cuts them into one section per
    /// album, nested albums included, so a photo in two albums is in both sections. `sql()`
    /// (counts, ids) sees each photo once.
    #[ts(type = "number | null")]
    #[serde(default)]
    pub collection_id: Option<i64>,
}

impl LayoutScope {
    pub fn library() -> Self {
        Self::default()
    }

    /// Month sections make sense for time-ordered views only.
    pub fn sectioned(&self) -> bool {
        !self.best_match && ((self.album_id.is_none() && self.memory_id.is_none()) || self.sort_by_date)
    }

    /// Position of each ranked id when the scope is in *Best match* order. SQL keeps the
    /// timeline order and the caller sorts by this afterwards (a stable sort), because
    /// looking a rank up per row in SQL costs more than the whole sort.
    pub fn ranking(&self) -> Option<std::collections::HashMap<i64, usize>> {
        if !self.best_match {
            return None;
        }
        let rank = self.query.as_ref()?.rank.as_ref()?;
        Some(rank.iter().enumerate().map(|(i, id)| (*id, i)).collect())
    }

    /// The Library timeline with nothing narrowing it (Photos / Videos split aside): no trash,
    /// Hidden folder, album, collection, memory, person, pet, device, favourites, search or tag.
    pub fn is_plain_library(&self) -> bool {
        !self.trash
            && !self.vault
            && !self.favorites
            && self.album_id.is_none()
            && self.collection_id.is_none()
            && self.memory_id.is_none()
            && self.person_id.is_none()
            && self.pet_id.is_none()
            && self.device_id.is_none()
            && self.query.as_ref().is_none_or(AssetQuery::is_empty)
    }

    /// Whether *Hide RAW files* (`on`) applies here: only the unfiltered Library. An album shows
    /// what was put in it, and any search, tag or filter shows every file it matches.
    pub fn hides_raw_pairs(&self, on: bool) -> bool {
        on && self.is_plain_library()
    }

    /// `(FROM … WHERE …, ORDER BY …, params)` for the layout index query. Columns are
    /// selected by the caller from alias `a`.
    pub fn sql(&self) -> (String, String, Vec<Value>) {
        let mut from = String::from("FROM assets a");
        let mut params: Vec<Value> = Vec::new();
        if let Some(album) = self.album_id {
            from.push_str(" JOIN album_items ai ON ai.asset_id = a.id AND ai.album_id = ?");
            params.push(Value::Integer(album));
        }
        if let Some(memory) = self.memory_id {
            from.push_str(" JOIN memory_items mi ON mi.asset_id = a.id AND mi.memory_id = ? AND mi.curated = 1");
            params.push(Value::Integer(memory));
        }
        let mut wh = vec![format!("a.state = {}", if self.trash { 1 } else { 0 })];
        // Hidden photos (round 3): the Hidden folder shows only its own, nothing else shows
        // them, and an album that hides its photos is the one place that still does.
        if self.vault {
            wh.push("a.in_vault = 1 AND a.unreadable = 0".into());
        } else {
            wh.push("a.in_vault = 0".into());
            if let (Some(album), false) = (self.album_id, self.trash) {
                // An album that hides its photos still shows them; the subquery does not
                // depend on the row, so SQLite answers it once.
                wh.push(format!("({} OR (a.unreadable = 0 AND EXISTS (SELECT 1 FROM albums al WHERE al.id = ? AND al.hides_items = 1)))", crate::concealed::SHOWN));
                params.push(Value::Integer(album));
            } else if !self.trash {
                wh.push(crate::concealed::SHOWN.into());
            }
        }
        // A Live Photo's MOV half never shows as its own tile.
        wh.push(format!("NOT {}", crate::assets::MOVIE_HALF));
        if self.hides_raw_pairs(hide_raw_with_jpeg()) {
            wh.push(format!("NOT {}", raw_with_companion()));
        }
        if let Some(mt) = self.media_type {
            wh.push(if mt == 1 { "a.media_type = 1".into() } else { "a.media_type != 1".into() });
        }
        if let Some(d) = self.device_id {
            wh.push("a.device_id = ?".into());
            params.push(Value::Integer(d));
        }
        if self.favorites {
            wh.push("a.is_favorite = 1".into());
        }
        if let Some(c) = self.collection_id {
            wh.push(
                "a.id IN (SELECT ai3.asset_id FROM album_items ai3 JOIN albums al3 ON al3.id = ai3.album_id WHERE al3.collection_id IN \
                 (WITH RECURSIVE sub(id) AS (SELECT ? UNION SELECT c.id FROM album_collections c JOIN sub ON c.parent_id = sub.id) SELECT id FROM sub))"
                    .into(),
            );
            params.push(Value::Integer(c));
        }
        if let Some(p) = self.person_id {
            // EXISTS rather than a join: a photo with two faces of one person is still one tile.
            wh.push("EXISTS (SELECT 1 FROM faces f WHERE f.asset_id = a.id AND f.person_id = ?)".into());
            params.push(Value::Integer(p));
        }
        if let Some(p) = self.pet_id {
            wh.push("EXISTS (SELECT 1 FROM pet_detections pd WHERE pd.asset_id = a.id AND pd.pet_id = ? AND pd.rejected IS NULL)".into());
            params.push(Value::Integer(p));
        }
        // The query's own joins and clauses come last, so its parameters follow the scope's.
        if let Some(q) = self.query.as_ref().filter(|q| !q.is_empty()) {
            let (joins, clauses, qp) = q.sql_parts();
            from.push_str(&joins);
            wh.extend(clauses);
            params.extend(qp);
        }
        let order = if self.trash {
            "a.trashed_at DESC, a.id DESC".to_string()
        } else if self.album_id.is_some() && !self.sort_by_date {
            "ai.position ASC, a.id ASC".to_string()
        } else {
            TIMELINE_ORDER.replace("captured_at", "a.captured_at").replace("imported_at", "a.imported_at").replace(" id DESC", " a.id DESC")
        };
        (format!("{from} WHERE {}", wh.join(" AND ")), order, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_match_is_a_layout_order() {
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for (id, at) in [(1, 100), (2, 400), (3, 300), (4, 200)] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at)
                     VALUES (?1, ?1, 'p', 'x.jpg', 'jpg', 0, 1, 1, ?2, 1)",
                    rusqlite::params![id, at],
                )?;
            }
            let query = AssetQuery { favorites: false, rank: Some(vec![1, 3]), ..Default::default() };
            let ranked = LayoutScope { best_match: true, query: Some(query.clone()), ..Default::default() };
            let newest = LayoutScope { query: Some(query), ..Default::default() };
            let order = |s: &LayoutScope| crate::layout::LayoutIndex::load_scoped(c, s, true).map(|i| i.items.iter().map(|x| x.id).collect::<Vec<_>>());
            assert_eq!(order(&ranked)?, vec![1, 3, 2, 4], "the ranked first, then the rest newest first");
            assert_eq!(order(&newest)?, vec![2, 3, 4, 1], "rank orders nothing without best_match");
            assert_eq!(crate::layout::ids_scoped(c, &ranked, 3)?, vec![1, 3, 2], "the limit applies after the sort");
            assert!(!ranked.sectioned(), "a ranking has no month headers");
            assert!(!ranked.sql().0.contains("OFFSET") && !ranked.sql().1.contains("OFFSET"));
            assert!(AssetQuery { rank: Some(vec![1]), ..Default::default() }.is_empty(), "a rank alone narrows nothing");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_raw_with_a_jpeg_is_recognised() {
        let db = crate::db::Db::open_in_memory().unwrap();
        let hidden: Vec<i64> = db
            .with_conn(|c| {
                for (id, name, ext, at) in [(1, "DSC_1.NEF", "nef", 100), (2, "DSC_1.JPG", "jpg", 100), (3, "DSC_2.NEF", "nef", 200), (4, "DSC_3.nef", "nef", 300), (5, "dsc_3.jpg", "jpg", 300)] {
                    c.execute(
                        "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at)
                         VALUES (?1, ?1, ?2, ?2, ?3, 0, 1, 1, ?4, 1)",
                        rusqlite::params![id, name, ext, at],
                    )?;
                }
                let mut stmt = c.prepare(&format!("SELECT a.id FROM assets a WHERE {} ORDER BY a.id", raw_with_companion()))?;
                let ids = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
                Ok(ids)
            })
            .unwrap();
        assert_eq!(hidden, vec![1, 4], "a lone RAW stays");
    }

    #[test]
    fn only_the_library_hides_raws_with_a_jpeg() {
        assert!(LayoutScope::library().hides_raw_pairs(true));
        assert!(LayoutScope { media_type: Some(0), ..Default::default() }.hides_raw_pairs(true));
        assert!(!LayoutScope::library().hides_raw_pairs(false), "the switch is off");
        assert!(!LayoutScope { album_id: Some(7), ..Default::default() }.hides_raw_pairs(true), "an album shows what was put in it");
        assert!(!LayoutScope { trash: true, ..Default::default() }.hides_raw_pairs(true));
        let raw = AssetQuery { include_raw_pairs: true, ..Default::default() };
        assert!(!LayoutScope { query: Some(raw), ..Default::default() }.hides_raw_pairs(true), "asking for RAWs shows them");
    }

    #[test]
    fn any_narrowing_shows_raws_beside_their_jpeg() {
        let hides = |s: LayoutScope| s.hides_raw_pairs(true);
        assert!(hides(LayoutScope { query: Some(AssetQuery::default()), ..Default::default() }), "an empty query narrows nothing");
        let text = AssetQuery { text: Some("paris".into()), ..Default::default() };
        assert!(!hides(LayoutScope { query: Some(text), ..Default::default() }), "any text shows them");
        let tag = AssetQuery { kind: Some(crate::query::AssetKind::Raw), ..Default::default() };
        assert!(!hides(LayoutScope { query: Some(tag), ..Default::default() }), "any filter shows them");
        assert!(!hides(LayoutScope { person_id: Some(1), ..Default::default() }));
        assert!(!hides(LayoutScope { pet_id: Some(1), ..Default::default() }));
        assert!(!hides(LayoutScope { memory_id: Some(1), ..Default::default() }));
        assert!(!hides(LayoutScope { collection_id: Some(1), ..Default::default() }));
        assert!(!hides(LayoutScope { device_id: Some(1), ..Default::default() }));
        assert!(!hides(LayoutScope { favorites: true, ..Default::default() }));
        assert!(!hides(LayoutScope { vault: true, ..Default::default() }));
    }

    #[test]
    fn builds_sql_per_scope() {
        let (w, o, p) = LayoutScope::library().sql();
        assert!(w.contains("a.state = 0") && !w.contains("album_items"));
        assert!(o.starts_with("COALESCE(a.captured_at"));
        assert!(p.is_empty());

        let s = LayoutScope { album_id: Some(7), ..Default::default() };
        let (w, o, p) = s.sql();
        // The album id twice: the join, then whether the album hides its photos.
        assert!(w.contains("JOIN album_items ai") && p == vec![Value::Integer(7), Value::Integer(7)]);
        assert_eq!(o, "ai.position ASC, a.id ASC");
        assert!(!s.sectioned());
        assert!(LayoutScope { album_id: Some(7), sort_by_date: true, ..Default::default() }.sectioned());

        let (w, o, _) = LayoutScope { trash: true, ..Default::default() }.sql();
        assert!(w.contains("a.state = 1"));
        assert!(o.starts_with("a.trashed_at DESC"));

        let (w, _, p) = LayoutScope { device_id: Some(3), favorites: true, media_type: Some(1), ..Default::default() }.sql();
        assert!(w.contains("a.device_id = ?") && w.contains("a.is_favorite = 1") && w.contains("a.media_type = 1"));
        assert_eq!(p, vec![Value::Integer(3)]);
    }

    #[test]
    fn a_query_joins_fts_and_binds_after_the_scope() {
        let scope = LayoutScope {
            album_id: Some(7),
            query: Some(AssetQuery { text: Some("beach".into()), min_rating: Some(4), ..Default::default() }),
            ..Default::default()
        };
        let (w, _, p) = scope.sql();
        assert!(w.contains("JOIN album_items ai"));
        assert!(w.contains("JOIN assets_fts f ON f.rowid = a.id"));
        assert!(w.contains("assets_fts MATCH ?") && w.contains("a.rating >= ?"));
        assert_eq!(p, vec![Value::Integer(7), Value::Integer(7), Value::Text("{filename device_name place_name keywords} : (\"beach\"*)".into()), Value::Integer(4)]);
        // An empty query changes nothing.
        let plain = LayoutScope { query: Some(AssetQuery::default()), ..Default::default() };
        assert_eq!(plain.sql().0, LayoutScope::library().sql().0);
    }
}
