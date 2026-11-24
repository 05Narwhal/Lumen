//! *Accept* on the review screen (Round 25 P4b-2b): a plugin's proposal becomes a change to the
//! library, only through what the app already does by hand, and only to Lumen's own records:
//! a date (the wall-clock capture time), tags (added to the photo's keywords), a colour label,
//! a title or caption, a place (only on a photo that has none, marked *Estimated*) and a face's
//! name (the face moves to the person of that name, or to a new one; nobody is renamed).
//! Nothing here opens or changes an original file.
//!
//! Every accept returns an [`AcceptUndo`] holding what the rows were before; the screen offers it for
//! 6 s and hands it back to [`undo`].

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::assets::PLACE_SOURCE_ESTIMATED;
use crate::labels::{self, LABEL_NAMES};
use crate::people::{self, FacePlace};
use crate::proposals::{self, Proposal, ProposalStatus, ProposalValue};
use crate::Result;

/// One proposal to accept, with the value the person typed instead when they edited it.
#[derive(Debug, Clone, PartialEq, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptItem {
    #[ts(type = "number")]
    pub id: i64,
    /// Replaces the proposed value (same kind); `None` accepts it as proposed.
    pub value: Option<ProposalValue>,
}

/// Why a proposal was not accepted.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptSkipped {
    #[ts(type = "number")]
    pub id: i64,
    pub reason: String,
}

/// The columns an accept can change, as they were.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AssetSnapshot {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub captured_at: Option<i64>,
    #[ts(type = "number | null")]
    pub captured_at_utc: Option<i64>,
    #[ts(type = "number | null")]
    pub tz_offset_minutes: Option<i64>,
    pub capture_source: Option<String>,
    pub keywords: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    #[ts(type = "number")]
    pub color_label: i64,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub place_name: Option<String>,
    pub locality: Option<String>,
    #[ts(type = "number")]
    pub place_source: i64,
}

/// One change to take back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum AcceptReversal {
    Asset { snapshot: Box<AssetSnapshot> },
    /// A face moved to a person; `created` is the person made for it, if one was.
    Face {
        place: FacePlace,
        #[ts(type = "number | null")]
        created: Option<i64>,
    },
}

/// Everything one accept changed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptUndo {
    #[ts(type = "number[]")]
    pub proposals: Vec<i64>,
    pub reversals: Vec<AcceptReversal>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptReport {
    #[ts(type = "number[]")]
    pub accepted: Vec<i64>,
    pub skipped: Vec<AcceptSkipped>,
    pub undo: AcceptUndo,
}

/// Accepts `items`; each is independent, so one that can't be applied is skipped with its reason
/// and the rest go on.
pub fn accept(conn: &Connection, items: &[AcceptItem], now: i64) -> Result<AcceptReport> {
    let mut report = AcceptReport::default();
    for item in items {
        let mut skip = |reason: &str| report_skip(&mut report.skipped, item.id, reason);
        let Some(p) = proposals::get(conn, item.id)? else {
            skip("That proposal is gone.");
            continue;
        };
        if p.status != ProposalStatus::Pending {
            skip("That proposal was already decided.");
            continue;
        }
        let value = match &item.value {
            None => p.proposed.clone(),
            Some(v) if v.kind() != p.proposed.kind() => {
                skip("An edit has to keep the kind of the proposal.");
                continue;
            }
            Some(v) => match v.clone().normalised() {
                Ok(v) => v,
                Err(why) => {
                    skip(&why);
                    continue;
                }
            },
        };
        match apply(conn, p.asset_id, &value, now)? {
            Outcome::Skip(why) => skip(&why),
            Outcome::Unchanged => {
                proposals::set_status(conn, &[item.id], ProposalStatus::Accepted, now)?;
                report.accepted.push(item.id);
                report.undo.proposals.push(item.id);
            }
            Outcome::Changed(reversal) => {
                proposals::set_status(conn, &[item.id], ProposalStatus::Accepted, now)?;
                report.accepted.push(item.id);
                report.undo.proposals.push(item.id);
                report.undo.reversals.push(reversal);
            }
        }
    }
    Ok(report)
}

fn report_skip(skipped: &mut Vec<AcceptSkipped>, id: i64, reason: &str) {
    skipped.push(AcceptSkipped { id, reason: reason.to_string() });
}

enum Outcome {
    Changed(AcceptReversal),
    /// Already as proposed: accepted, nothing to take back.
    Unchanged,
    Skip(String),
}

fn snapshot(conn: &Connection, id: i64) -> Result<Option<AssetSnapshot>> {
    Ok(conn
        .query_row(
            "SELECT captured_at, captured_at_utc, tz_offset_minutes, capture_source, keywords, title, caption, color_label,
                    gps_lat, gps_lon, place_name, locality, place_source
               FROM assets WHERE id = ?1 AND state = 0",
            [id],
            |r| {
                Ok(AssetSnapshot {
                    id,
                    captured_at: r.get(0)?,
                    captured_at_utc: r.get(1)?,
                    tz_offset_minutes: r.get(2)?,
                    capture_source: r.get(3)?,
                    keywords: r.get(4)?,
                    title: r.get(5)?,
                    caption: r.get(6)?,
                    color_label: r.get(7)?,
                    gps_lat: r.get(8)?,
                    gps_lon: r.get(9)?,
                    place_name: r.get(10)?,
                    locality: r.get(11)?,
                    place_source: r.get(12)?,
                })
            },
        )
        .optional()?)
}

fn apply(conn: &Connection, asset: i64, value: &ProposalValue, now: i64) -> Result<Outcome> {
    if let ProposalValue::Person { face, name } = value {
        return name_face(conn, asset, *face, name, now);
    }
    let Some(before) = snapshot(conn, asset)? else {
        return Ok(Outcome::Skip("That photo is in the trash or gone.".into()));
    };
    match value {
        ProposalValue::Date(d) => {
            let Ok(dt) = chrono::NaiveDateTime::parse_from_str(d, "%Y-%m-%dT%H:%M:%S") else {
                return Ok(Outcome::Skip("That date isn't YYYY-MM-DDTHH:MM:SS.".into()));
            };
            // `captured_at` is wall-clock time kept as seconds; UTC follows from the photo's zone.
            let local = dt.and_utc().timestamp();
            let utc = local - before.tz_offset_minutes.unwrap_or(0) * 60;
            conn.execute("UPDATE assets SET captured_at = ?2, captured_at_utc = ?3, capture_source = 'plugin' WHERE id = ?1", params![asset, local, utc])?;
        }
        ProposalValue::Tags(tags) => {
            let mut all: Vec<String> = before.keywords.as_deref().unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            for t in tags {
                if !all.iter().any(|k| k.eq_ignore_ascii_case(t)) {
                    all.push(t.clone());
                }
            }
            conn.execute("UPDATE assets SET keywords = ?2 WHERE id = ?1", params![asset, all.join("\n")])?;
            crate::fts::index_ids(conn, &[asset])?;
        }
        ProposalValue::Label(name) => {
            let Some(n) = LABEL_NAMES[1..].iter().position(|l| l.eq_ignore_ascii_case(name)) else {
                return Ok(Outcome::Skip("That isn't a colour label.".into()));
            };
            labels::set_color_label(conn, &[asset], n as i64 + 1)?;
        }
        ProposalValue::Title(t) => {
            conn.execute("UPDATE assets SET title = ?2 WHERE id = ?1", params![asset, t])?;
            crate::fts::index_ids(conn, &[asset])?;
        }
        ProposalValue::Caption(t) => {
            conn.execute("UPDATE assets SET caption = ?2 WHERE id = ?1", params![asset, t])?;
            crate::fts::index_ids(conn, &[asset])?;
        }
        ProposalValue::Location { lat, lon } => {
            // A camera's or the person's own place is never replaced, as with Wayfinder's write.
            if before.gps_lat.is_some() && before.gps_lon.is_some() {
                return Ok(Outcome::Skip("That photo already has a location.".into()));
            }
            let label = crate::places::reverse_geocode(*lat, *lon).and_then(|p| p.label());
            conn.execute(
                "UPDATE assets SET gps_lat = ?2, gps_lon = ?3, place_name = ?4, locality = ?4, place_source = ?5 WHERE id = ?1",
                params![asset, lat, lon, label, PLACE_SOURCE_ESTIMATED],
            )?;
            crate::fts::index_ids(conn, &[asset])?;
        }
        ProposalValue::Person { .. } => unreachable!("handled above"),
    }
    Ok(Outcome::Changed(AcceptReversal::Asset { snapshot: Box::new(before) }))
}

/// The face goes to the listed person of that name (any case), or to a new person of that name.
/// Nobody is renamed and no other face moves.
fn name_face(conn: &Connection, asset: i64, face: i64, name: &str, now: i64) -> Result<Outcome> {
    let on_photo: Option<(Option<i64>, i64)> = conn
        .query_row(
            "SELECT f.person_id, a.state FROM faces f JOIN assets a ON a.id = f.asset_id WHERE f.id = ?1 AND f.asset_id = ?2",
            params![face, asset],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((current, state)) = on_photo else {
        return Ok(Outcome::Skip("That face isn't on this photo any more.".into()));
    };
    if state != 0 {
        return Ok(Outcome::Skip("That photo is in the trash or gone.".into()));
    }
    let named: Option<i64> = conn
        .query_row("SELECT id FROM people WHERE name = ?1 COLLATE NOCASE AND status = 'listed' ORDER BY id LIMIT 1", [name], |r| r.get(0))
        .optional()?;
    match named {
        Some(person) if current == Some(person) => Ok(Outcome::Unchanged),
        Some(person) => Ok(Outcome::Changed(AcceptReversal::Face { place: people::tag_face(conn, face, person)?, created: None })),
        None => {
            let (person, place) = people::create_person_with_face(conn, face, name, now)?;
            Ok(Outcome::Changed(AcceptReversal::Face { place, created: Some(person) }))
        }
    }
}

/// Takes an accept back: the rows are as they were and the proposals are pending again.
pub fn undo(conn: &Connection, undo: &AcceptUndo, now: i64) -> Result<()> {
    // Newest change first, so a face moved twice ends where it started.
    for reversal in undo.reversals.iter().rev() {
        match reversal {
            AcceptReversal::Asset { snapshot: s } => {
                conn.execute(
                    "UPDATE assets SET captured_at = ?2, captured_at_utc = ?3, tz_offset_minutes = ?4, capture_source = ?5, keywords = ?6,
                            title = ?7, caption = ?8, color_label = ?9, gps_lat = ?10, gps_lon = ?11, place_name = ?12, locality = ?13, place_source = ?14
                      WHERE id = ?1",
                    params![
                        s.id,
                        s.captured_at,
                        s.captured_at_utc,
                        s.tz_offset_minutes,
                        s.capture_source,
                        s.keywords,
                        s.title,
                        s.caption,
                        s.color_label,
                        s.gps_lat,
                        s.gps_lon,
                        s.place_name,
                        s.locality,
                        s.place_source
                    ],
                )?;
                crate::fts::index_ids(conn, &[s.id])?;
            }
            AcceptReversal::Face { place, created: Some(person) } => people::undo_create_person(conn, *person, place)?,
            AcceptReversal::Face { place, created: None } => people::restore_faces(conn, std::slice::from_ref(place))?,
        }
    }
    proposals::set_status(conn, &undo.proposals, ProposalStatus::Pending, now)?;
    Ok(())
}

/// One row of the review screen: the proposal, the photo's file name and what the photo has now
/// for that kind (the same shape as the proposed value), so the screen shows both side by side.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalReviewItem {
    pub proposal: Proposal,
    pub filename: String,
    pub current: Option<ProposalValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalReviewPage {
    pub items: Vec<ProposalReviewItem>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

/// A page of one kind's proposals with what each photo has now.
pub fn review_page(conn: &Connection, kind: &str, status: ProposalStatus, plugin: Option<&str>, after: Option<i64>, limit: usize) -> Result<ProposalReviewPage> {
    let page = proposals::page(conn, kind, status, plugin, after, limit)?;
    let mut items = Vec::with_capacity(page.items.len());
    for proposal in page.items {
        let (filename, current) = current_of(conn, &proposal)?;
        items.push(ProposalReviewItem { proposal, filename, current });
    }
    Ok(ProposalReviewPage { items, next_cursor: page.next_cursor })
}

/// The photo's file name and what it has now for the proposal's kind.
fn current_of(conn: &Connection, p: &Proposal) -> Result<(String, Option<ProposalValue>)> {
    let row = conn
        .query_row(
            "SELECT filename, captured_at, keywords, title, caption, color_label, gps_lat, gps_lon FROM assets WHERE id = ?1",
            [p.asset_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((filename, captured, keywords, title, caption, label, lat, lon)) = row else {
        return Ok((String::new(), None));
    };
    let text = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let current = match &p.proposed {
        ProposalValue::Date(_) => captured
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|d| ProposalValue::Date(d.naive_utc().format("%Y-%m-%dT%H:%M:%S").to_string())),
        ProposalValue::Tags(_) => {
            let tags: Vec<String> = keywords.as_deref().unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            (!tags.is_empty()).then_some(ProposalValue::Tags(tags))
        }
        ProposalValue::Label(_) => labels::label_name(label).map(|n| ProposalValue::Label(n.to_string())),
        ProposalValue::Title(_) => text(title).map(ProposalValue::Title),
        ProposalValue::Caption(_) => text(caption).map(ProposalValue::Caption),
        ProposalValue::Location { .. } => lat.zip(lon).map(|(lat, lon)| ProposalValue::Location { lat, lon }),
        ProposalValue::Person { face, .. } => conn
            .query_row("SELECT p.name FROM faces f JOIN people p ON p.id = f.person_id WHERE f.id = ?1 AND p.name IS NOT NULL", [face], |r| r.get::<_, String>(0))
            .optional()?
            .map(|name| ProposalValue::Person { face: *face, name }),
    };
    Ok((filename, current))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposals::{self, NewProposal, ProposalStatus};
    use crate::Db;

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for i in 1..=3 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 10)",
                    params![i, format!("h{i}")],
                )?;
            }
            // Photo 3 already has a place.
            c.execute("UPDATE assets SET gps_lat = 10.0, gps_lon = 20.0, place_source = 0 WHERE id = 3", [])?;
            c.execute("INSERT INTO faces (id, asset_id, x, y, w, h, score, vec, model) VALUES (7, 1, 0, 0, 0.1, 0.1, 0.9, x'00', 'm')", [])?;
            Ok(())
        })
        .unwrap();
        db
    }

    /// Submits one proposal and returns its id.
    fn propose(db: &Db, asset: i64, value: ProposalValue) -> i64 {
        db.with_conn(|c| {
            let kind = value.kind();
            proposals::submit(c, "acme.finder", &[NewProposal { asset_id: asset, value, confidence: 0.9, reason: None }], 100)?;
            Ok(c.query_row("SELECT id FROM plugin_proposals WHERE asset_id = ?1 AND kind = ?2", params![asset, kind], |r| r.get(0))?)
        })
        .unwrap()
    }

    fn take(id: i64) -> Vec<AcceptItem> {
        vec![AcceptItem { id, value: None }]
    }

    fn status(db: &Db, id: i64) -> ProposalStatus {
        db.with_conn(|c| Ok(proposals::get(c, id)?.unwrap().status)).unwrap()
    }

    fn row<T: rusqlite::types::FromSql>(db: &Db, sql: &str, id: i64) -> T {
        db.with_conn(|c| Ok(c.query_row(sql, [id], |r| r.get(0))?)).unwrap()
    }

    #[test]
    fn a_date_sets_the_wall_clock_time_and_undo_puts_it_back() {
        let db = library();
        let id = propose(&db, 1, ProposalValue::Date("2021-05-04T10:00:00".into()));
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(report.accepted, vec![id]);
        // 2021-05-04 10:00:00 as a wall-clock time stored like every other captured_at.
        assert_eq!(row::<i64>(&db, "SELECT captured_at FROM assets WHERE id = ?1", 1), 1_620_122_400);
        assert_eq!(row::<String>(&db, "SELECT capture_source FROM assets WHERE id = ?1", 1), "plugin");
        assert_eq!(status(&db, id), ProposalStatus::Accepted);

        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<Option<i64>>(&db, "SELECT captured_at FROM assets WHERE id = ?1", 1), None);
        assert_eq!(row::<Option<String>>(&db, "SELECT capture_source FROM assets WHERE id = ?1", 1), None);
        assert_eq!(status(&db, id), ProposalStatus::Pending);
    }

    #[test]
    fn a_date_keeps_the_photo_s_time_zone() {
        let db = library();
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET tz_offset_minutes = 120 WHERE id = 1", [])?)).unwrap();
        let id = propose(&db, 1, ProposalValue::Date("2021-05-04T10:00:00".into()));
        db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(row::<i64>(&db, "SELECT captured_at_utc FROM assets WHERE id = ?1", 1), 1_620_122_400 - 120 * 60);
    }

    #[test]
    fn tags_join_the_keywords_without_duplicates_and_are_searchable() {
        let db = library();
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET keywords = 'beach' WHERE id = 1", [])?)).unwrap();
        let id = propose(&db, 1, ProposalValue::Tags(vec!["Beach".into(), "sea".into()]));
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(row::<String>(&db, "SELECT keywords FROM assets WHERE id = ?1", 1), "beach\nsea");
        let found: i64 = row(&db, "SELECT count(*) FROM assets_fts WHERE assets_fts MATCH 'sea' AND rowid = ?1", 1);
        assert_eq!(found, 1);
        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<String>(&db, "SELECT keywords FROM assets WHERE id = ?1", 1), "beach");
    }

    #[test]
    fn a_label_a_title_and_a_caption_are_set_and_undone() {
        let db = library();
        let label = propose(&db, 1, ProposalValue::Label("Green".into()));
        let title = propose(&db, 1, ProposalValue::Title("Sunset".into()));
        let caption = propose(&db, 2, ProposalValue::Caption("A long day".into()));
        let report = db.with_conn(|c| accept(c, &[AcceptItem { id: label, value: None }, AcceptItem { id: title, value: None }, AcceptItem { id: caption, value: None }], 200)).unwrap();
        assert_eq!(report.accepted.len(), 3);
        assert_eq!(row::<i64>(&db, "SELECT color_label FROM assets WHERE id = ?1", 1), 3);
        assert_eq!(row::<String>(&db, "SELECT title FROM assets WHERE id = ?1", 1), "Sunset");
        assert_eq!(row::<String>(&db, "SELECT caption FROM assets WHERE id = ?1", 2), "A long day");
        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<i64>(&db, "SELECT color_label FROM assets WHERE id = ?1", 1), 0);
        assert_eq!(row::<Option<String>>(&db, "SELECT title FROM assets WHERE id = ?1", 1), None);
        assert_eq!(row::<Option<String>>(&db, "SELECT caption FROM assets WHERE id = ?1", 2), None);
    }

    #[test]
    fn an_edited_value_replaces_the_proposed_one_when_it_is_the_same_kind_and_valid() {
        let db = library();
        let id = propose(&db, 1, ProposalValue::Title("Sunset".into()));
        let report = db.with_conn(|c| accept(c, &[AcceptItem { id, value: Some(ProposalValue::Title("  Dusk ".into())) }], 200)).unwrap();
        assert_eq!(report.accepted, vec![id]);
        assert_eq!(row::<String>(&db, "SELECT title FROM assets WHERE id = ?1", 1), "Dusk");

        // Another kind, or a value that doesn't pass, is skipped and nothing changes.
        let label = propose(&db, 2, ProposalValue::Label("Red".into()));
        let report = db
            .with_conn(|c| accept(c, &[AcceptItem { id: label, value: Some(ProposalValue::Title("x".into())) }, AcceptItem { id: label, value: Some(ProposalValue::Label("Orange".into())) }], 200))
            .unwrap();
        assert!(report.accepted.is_empty());
        assert_eq!(report.skipped.len(), 2);
        assert_eq!(row::<i64>(&db, "SELECT color_label FROM assets WHERE id = ?1", 2), 0);
        assert_eq!(status(&db, label), ProposalStatus::Pending);
    }

    #[test]
    fn a_location_fills_a_photo_with_none_and_marks_it_estimated() {
        let db = library();
        let id = propose(&db, 1, ProposalValue::Location { lat: 46.05, lon: 14.5 });
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(report.accepted, vec![id]);
        assert_eq!(row::<f64>(&db, "SELECT gps_lat FROM assets WHERE id = ?1", 1), 46.05);
        assert_eq!(row::<i64>(&db, "SELECT place_source FROM assets WHERE id = ?1", 1), crate::assets::PLACE_SOURCE_ESTIMATED);
        assert!(row::<Option<String>>(&db, "SELECT place_name FROM assets WHERE id = ?1", 1).is_some(), "named from the atlas");
        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<Option<f64>>(&db, "SELECT gps_lat FROM assets WHERE id = ?1", 1), None);
        assert_eq!(row::<i64>(&db, "SELECT place_source FROM assets WHERE id = ?1", 1), 0);
    }

    #[test]
    fn a_location_never_replaces_a_place_the_photo_has() {
        let db = library();
        let id = propose(&db, 3, ProposalValue::Location { lat: 46.05, lon: 14.5 });
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert!(report.accepted.is_empty());
        assert_eq!(report.skipped[0].id, id);
        assert_eq!(row::<f64>(&db, "SELECT gps_lat FROM assets WHERE id = ?1", 3), 10.0);
        assert_eq!(status(&db, id), ProposalStatus::Pending);
    }

    #[test]
    fn a_name_moves_the_face_to_a_new_person_and_undo_removes_them() {
        let db = library();
        let id = propose(&db, 1, ProposalValue::Person { face: 7, name: "Ana".into() });
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(report.accepted, vec![id]);
        let person: i64 = row(&db, "SELECT person_id FROM faces WHERE id = ?1", 7);
        assert_eq!(row::<String>(&db, "SELECT name FROM people WHERE id = ?1", person), "Ana");
        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<Option<i64>>(&db, "SELECT person_id FROM faces WHERE id = ?1", 7), None);
        assert_eq!(row::<i64>(&db, "SELECT count(*) FROM people WHERE id = ?1", person), 0);
        assert_eq!(status(&db, id), ProposalStatus::Pending);
    }

    #[test]
    fn a_name_that_exists_joins_that_person_and_nobody_is_renamed() {
        let db = library();
        let ana = db.with_conn(|c| crate::people::create_person(c, "Ana", 5)).unwrap();
        let id = propose(&db, 1, ProposalValue::Person { face: 7, name: "ana".into() });
        let report = db.with_conn(|c| accept(c, &take(id), 200)).unwrap();
        assert_eq!(row::<i64>(&db, "SELECT person_id FROM faces WHERE id = ?1", 7), ana);
        assert_eq!(row::<i64>(&db, "SELECT count(*) FROM people WHERE ?1 > 0", 1), 1, "no second Ana");
        db.with_conn(|c| undo(c, &report.undo, 201)).unwrap();
        assert_eq!(row::<Option<i64>>(&db, "SELECT person_id FROM faces WHERE id = ?1", 7), None);
        // The person that already existed stays.
        assert_eq!(row::<String>(&db, "SELECT name FROM people WHERE id = ?1", ana), "Ana");
    }

    #[test]
    fn what_cannot_be_accepted_is_skipped_with_a_reason() {
        let db = library();
        let a = propose(&db, 1, ProposalValue::Title("One".into()));
        let b = propose(&db, 2, ProposalValue::Title("Two".into()));
        db.with_conn(|c| Ok(c.execute("UPDATE assets SET state = 1 WHERE id = 2", [])?)).unwrap(); // in the trash
        db.with_conn(|c| proposals::set_status(c, &[a], ProposalStatus::Rejected, 150)).unwrap();
        let report = db.with_conn(|c| accept(c, &[AcceptItem { id: a, value: None }, AcceptItem { id: b, value: None }, AcceptItem { id: 999, value: None }], 200)).unwrap();
        assert!(report.accepted.is_empty());
        assert_eq!(report.skipped.len(), 3);
        assert!(report.skipped.iter().all(|s| !s.reason.is_empty()));
        assert_eq!(row::<Option<String>>(&db, "SELECT title FROM assets WHERE id = ?1", 1), None);
        assert_eq!(status(&db, a), ProposalStatus::Rejected);
        assert!(report.undo.reversals.is_empty());
    }

    #[test]
    fn the_review_shows_what_the_photo_has_now_beside_each_proposal() {
        let db = library();
        db.with_conn(|c| {
            c.execute("UPDATE assets SET title = 'Old title', keywords = 'beach' || char(10) || 'sea', color_label = 2, captured_at = 1620122400, filename = 'IMG_1.jpg' WHERE id = 1", [])?;
            Ok(())
        })
        .unwrap();
        let ana = db.with_conn(|c| crate::people::create_person(c, "Ben", 5)).unwrap();
        db.with_conn(|c| Ok(c.execute("UPDATE faces SET person_id = ?1 WHERE id = 7", [ana])?)).unwrap();
        propose(&db, 1, ProposalValue::Title("New".into()));
        propose(&db, 1, ProposalValue::Tags(vec!["x".into()]));
        propose(&db, 1, ProposalValue::Label("Green".into()));
        propose(&db, 1, ProposalValue::Date("2022-01-01T00:00:00".into()));
        propose(&db, 1, ProposalValue::Caption("Hi".into()));
        propose(&db, 1, ProposalValue::Person { face: 7, name: "Ana".into() });
        propose(&db, 3, ProposalValue::Location { lat: 46.0, lon: 14.0 });
        let first = |kind: &str| db.with_conn(|c| review_page(c, kind, ProposalStatus::Pending, None, None, 50)).unwrap().items.remove(0);

        let title = first("title");
        assert_eq!(title.filename, "IMG_1.jpg");
        assert_eq!(title.current, Some(ProposalValue::Title("Old title".into())));
        assert_eq!(first("tags").current, Some(ProposalValue::Tags(vec!["beach".into(), "sea".into()])));
        assert_eq!(first("label").current, Some(ProposalValue::Label("Yellow".into())));
        assert_eq!(first("date").current, Some(ProposalValue::Date("2021-05-04T10:00:00".into())));
        assert_eq!(first("caption").current, None);
        assert_eq!(first("person").current, Some(ProposalValue::Person { face: 7, name: "Ben".into() }));
        assert_eq!(first("location").current, Some(ProposalValue::Location { lat: 10.0, lon: 20.0 }));
    }

    #[test]
    fn the_review_pages_by_cursor_and_filters_by_status() {
        let db = library();
        for a in 1..=3 {
            propose(&db, a, ProposalValue::Title(format!("T{a}")));
        }
        let page = db.with_conn(|c| review_page(c, "title", ProposalStatus::Pending, None, None, 2)).unwrap();
        assert_eq!(page.items.len(), 2);
        let next = page.next_cursor.expect("a second page");
        let rest = db.with_conn(|c| review_page(c, "title", ProposalStatus::Pending, None, Some(next), 2)).unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.next_cursor, None);
        let done = db.with_conn(|c| review_page(c, "title", ProposalStatus::Accepted, None, None, 2)).unwrap();
        assert!(done.items.is_empty());
    }
}
