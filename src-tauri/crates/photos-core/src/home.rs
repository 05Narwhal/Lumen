//! What the Home page needs beyond the lists the app already has (Round 20 C): the photos
//! most recently changed in Develop, how many files Lumen can't open, and the places with the
//! most photos. Only photos the Library shows count.

use rusqlite::Connection;
use serde::Serialize;
use ts_rs::TS;

use crate::develop_source::DevelopSourceFile;
use crate::develop_strip;
use crate::query::PlaceRef;
use crate::search_overview::{place_summary, PlaceSummary};
use crate::Result;

/// How many drafts Home shows.
const DRAFTS_SHOWN: i64 = 6;

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct HomeSummary {
    /// The *In Develop* card's photos, at most six: the remembered filmstrip (the last photo
    /// open first, then the strip's order), else the photos with a Develop draft, the most
    /// recently changed first.
    #[ts(type = "Array<number>")]
    pub drafts: Vec<i64>,
    /// The card's count: the remembered filmstrip's size, else every photo with a draft.
    #[ts(type = "number")]
    pub draft_count: i64,
    /// The remembered filmstrip's label (*Selection*, an album's name); `None` when the card
    /// shows the photos with Develop work instead.
    pub strip_label: Option<String>,
    /// Files Lumen can't open (`assets.unreadable`).
    #[ts(type = "number")]
    pub unreadable: i64,
    /// People groups waiting for a name. No cheap query exists for the People page's
    /// suggestions, so this stays 0 and Home draws no pill for it.
    #[ts(type = "number")]
    pub people_to_review: i64,
    /// The places with the most photos (`places` as given, those with coordinates only).
    pub places: Vec<PlaceSummary>,
}

/// The most photos *Open Develop* opens when nothing is remembered.
pub const DRAFT_IDS_CAP: i64 = 5000;

/// The Library's own predicate (as the tag facets use it), for photos with Develop work.
fn drafts_from() -> String {
    let shown = format!("a.state = 0 AND a.in_vault = 0 AND {} AND NOT {}", crate::concealed::SHOWN, crate::assets::MOVIE_HALF);
    format!("FROM develop_drafts d JOIN assets a ON a.id = d.asset_id WHERE {shown}")
}

/// Every photo with Develop work, the most recently changed first, at most `cap`.
pub fn draft_ids(conn: &Connection, cap: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(&format!("SELECT a.id {} ORDER BY d.updated_at DESC, a.id DESC LIMIT {}", drafts_from(), cap.clamp(1, DRAFT_IDS_CAP)))?;
    let rows = stmt.query_map([], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
    Ok(rows)
}

/// The remembered strip as a card: its size, six photos (the last one open first, then the
/// strip's order) and its label. `None` when the strip is empty or every photo in it is gone.
fn strip_card(conn: &Connection, file: &DevelopSourceFile) -> Result<Option<(i64, Vec<i64>, String)>> {
    let Some(strip) = &file.source else { return Ok(None) };
    let only = strip.only.as_deref();
    let count = develop_strip::strip_count(conn, &strip.scope, only)?;
    if only.is_some_and(<[i64]>::is_empty) || count == 0 {
        return Ok(None);
    }
    let last = match file.last {
        Some(id) if develop_strip::strip_has(conn, &strip.scope, only, id)? => Some(id),
        _ => None,
    };
    let page = develop_strip::strip_page(conn, &strip.scope, only, strip.sort, None, DRAFTS_SHOWN as u32 + 1)?;
    let ids = last.into_iter().chain(page.items.iter().map(|i| i.id).filter(|i| Some(*i) != last)).take(DRAFTS_SHOWN as usize).collect();
    Ok(Some((count, ids, strip.label.clone())))
}

pub fn summary(conn: &Connection, places: &[PlaceRef], remembered: Option<&DevelopSourceFile>) -> Result<HomeSummary> {
    let from = drafts_from();
    let (draft_count, drafts, strip_label) = match remembered.map(|f| strip_card(conn, f)).transpose()?.flatten() {
        Some((count, ids, label)) => (count, ids, Some(label)),
        None => {
            let count: i64 = conn.query_row(&format!("SELECT COUNT(*) {from}"), [], |r| r.get(0))?;
            (count, draft_ids(conn, DRAFTS_SHOWN)?, None)
        }
    };
    let mut summaries = Vec::new();
    for place in places {
        let s = place_summary(conn, place)?;
        if s.lat.is_some() && s.lon.is_some() {
            summaries.push(s);
        }
    }
    Ok(HomeSummary { drafts, draft_count, strip_label, unreadable: crate::unreadable::count(conn)?, people_to_review: 0, places: summaries })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop_source::RememberedStrip;
    use crate::develop_strip::StripSort;
    use crate::tag_search::Facets;

    fn db() -> crate::Db {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at, place_name, gps_lat, gps_lon, state, concealed, unreadable)
                   VALUES (1, 'h1', 'p', 'a.jpg', 'jpg', 0, 1, 1, 1688000000, 1, 'Lisbon, Portugal', 38.7, -9.1, 0, 0, 0),
                          (2, 'h2', 'p', 'b.jpg', 'jpg', 0, 1, 1, 1689000000, 1, 'Lisbon, Portugal', 38.8, -9.2, 0, 0, 0),
                          (3, 'h3', 'p', 'c.jpg', 'jpg', 0, 1, 1, 1690000000, 1, 'Porto, Portugal', 41.1, -8.6, 0, 0, 0),
                          (4, 'h4', 'p', 'd.jpg', 'jpg', 0, 1, 1, 1691000000, 1, 'Faro, Portugal', NULL, NULL, 0, 0, 0),
                          (5, 'h5', 'p', 'e.jpg', 'jpg', 0, 1, 1, 1692000000, 1, NULL, NULL, NULL, 1, 0, 0),
                          (6, 'h6', 'p', 'f.jpg', 'jpg', 0, 1, 1, 1693000000, 1, NULL, NULL, NULL, 0, 1, 1),
                          (7, 'h7', 'p', 'g.jpg', 'jpg', 0, 1, 1, 1694000000, 1, NULL, NULL, NULL, 0, 0, 0),
                          (8, 'h8', 'p', 'h.jpg', 'jpg', 0, 1, 1, 1695000000, 1, NULL, NULL, NULL, 0, 0, 0),
                          (9, 'h9', 'p', 'i.jpg', 'jpg', 0, 1, 1, 1696000000, 1, NULL, NULL, NULL, 0, 0, 0),
                          (10, 'h10', 'p', 'j.jpg', 'jpg', 0, 1, 1, 1697000000, 1, NULL, NULL, NULL, 0, 0, 0);",
            )?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn draft(c: &Connection, id: i64, updated_at: i64) {
        c.execute("INSERT INTO develop_drafts (asset_id, recipe_json, version, updated_at) VALUES (?1, '{}', ?1, ?2)", rusqlite::params![id, updated_at]).unwrap();
    }

    #[test]
    fn drafts_newest_first_capped_at_six_and_without_trashed_photos() {
        let db = db();
        db.with_conn(|c| {
            // Eight drafts, asset 5 trashed: seven count, six show.
            for (id, at) in [(1, 100), (2, 200), (3, 300), (5, 900), (7, 700), (8, 800), (9, 650), (10, 50)] {
                draft(c, id, at);
            }
            let s = summary(c, &[], None)?;
            assert_eq!(s.drafts, vec![8, 7, 9, 3, 2, 1], "newest change first, the trashed photo's draft left out, six at most");
            assert_eq!(s.draft_count, 7);
            assert!(s.places.is_empty());
            Ok(())
        })
        .unwrap();
    }

    fn remembered(only: Option<Vec<i64>>, last: Option<i64>) -> DevelopSourceFile {
        DevelopSourceFile {
            source: Some(RememberedStrip { scope: crate::scope::LayoutScope::library(), only, label: "Selection".into(), sort: StripSort::Captured }),
            last,
        }
    }

    #[test]
    fn the_card_is_the_remembered_strip_last_photo_first() {
        let db = db();
        db.with_conn(|c| {
            draft(c, 1, 100); // a draft elsewhere does not count when a strip is remembered
            // 5 is trashed; 3, 7, 8, 9 remain, in capture order 3, 7, 8, 9; the last photo leads.
            let s = summary(c, &[], Some(&remembered(Some(vec![9, 5, 3, 8, 7]), Some(9))))?;
            assert_eq!(s.draft_count, 4);
            assert_eq!(s.drafts, vec![9, 3, 7, 8]);
            assert_eq!(s.strip_label.as_deref(), Some("Selection"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_remembered_scope_counts_its_photos_and_shows_six() {
        let db = db();
        db.with_conn(|c| {
            let s = summary(c, &[], Some(&remembered(None, None)))?;
            // 1 2 3 4 7 8 9 10 are shown photos; 5 is trashed and 6 is unreadable and concealed.
            assert_eq!(s.draft_count, 8);
            assert_eq!(s.drafts.len(), 6);
            // A last photo outside the strip does not lead.
            let s = summary(c, &[], Some(&remembered(Some(vec![7, 8]), Some(1))))?;
            assert_eq!(s.drafts, vec![7, 8]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn with_nothing_remembered_the_card_falls_back_to_the_drafts() {
        let db = db();
        db.with_conn(|c| {
            draft(c, 2, 200);
            draft(c, 3, 300);
            for remembered in [None, Some(remembered(Some(vec![]), None)), Some(remembered(Some(vec![5]), None)), Some(DevelopSourceFile::default())] {
                let s = summary(c, &[], remembered.as_ref())?;
                assert_eq!((s.drafts.clone(), s.draft_count, s.strip_label.clone()), (vec![3, 2], 2, None));
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn every_draft_opens_not_six_newest_first() {
        let db = db();
        db.with_conn(|c| {
            for id in [1, 2, 3, 5, 7, 8, 9, 10] {
                draft(c, id, id * 100);
            }
            // 5 is trashed; the order is the newest change first.
            assert_eq!(draft_ids(c, 5000)?, vec![10, 9, 8, 7, 3, 2, 1]);
            assert_eq!(draft_ids(c, 2)?, vec![10, 9]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn counts_files_lumen_cannot_open() {
        let db = db();
        db.with_conn(|c| {
            assert_eq!(summary(c, &[], None)?.unreadable, 1);
            assert_eq!(summary(c, &[], None)?.draft_count, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn summarises_two_places_and_skips_one_without_coordinates() {
        let db = db();
        db.with_conn(|c| {
            let lisbon = PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) };
            let faro = PlaceRef::City { name: "Faro".into(), country: Some("Portugal".into()) };
            let porto = PlaceRef::City { name: "Porto".into(), country: Some("Portugal".into()) };
            let s = summary(c, &[lisbon, faro, porto], None)?;
            assert_eq!(s.places.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), vec!["Lisbon, Portugal", "Porto, Portugal"]);
            assert_eq!(s.places[0].photo_count, 2);
            assert_eq!(s.places[0].cover_asset_id, Some(2));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_facets_name_the_places_with_the_most_photos() {
        let db = db();
        db.with_conn(|c| {
            let top = Facets::load(c)?.top_places(2);
            // Portugal (4 shown photos with a place) leads; Lisbon (2) is next.
            assert_eq!(top[0], PlaceRef::Country { name: "Portugal".into() });
            assert_eq!(top[1], PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) });
            assert_eq!(top.len(), 2);
            Ok(())
        })
        .unwrap();
    }
}
