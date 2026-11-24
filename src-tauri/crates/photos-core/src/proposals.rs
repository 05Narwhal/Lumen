//! What plugins propose for photos (Round 25 P4b, migration 0041): a place, a date, tags, a
//! colour label, a detected face's name, a title or a caption, each with a confidence and an
//! optional reason. Derived data shown on the review screen; nothing here changes a photo; the
//! person's *Accept* goes through the app's own write paths.
//!
//! One row per plugin + asset + kind: submitting again replaces the value. A rejected proposal
//! that comes back with the same value stays rejected; a different value is pending again.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::labels::LABEL_NAMES;
use crate::Result;

/// The most proposals one call may carry.
pub const MAX_BATCH: usize = 200;
const MAX_TAGS: usize = 20;
const MAX_TAG: usize = 60;
const MAX_TITLE: usize = 200;
const MAX_CAPTION: usize = 2000;
const MAX_REASON: usize = 200;
const MAX_NAME: usize = 100;

/// A proposed value; the case is the kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
#[ts(export)]
pub enum ProposalValue {
    Location { lat: f64, lon: f64 },
    /// Wall-clock capture time, `YYYY-MM-DDTHH:MM:SS`.
    Date(String),
    Tags(Vec<String>),
    /// A colour label's name, `Red` … `Purple`.
    Label(String),
    /// A name for a face detected on this photo; never creates a face.
    Person {
        #[ts(type = "number")]
        face: i64,
        name: String,
    },
    Title(String),
    Caption(String),
}

pub const KINDS: [&str; 7] = ["location", "date", "tags", "label", "person", "title", "caption"];

fn text(s: &str, what: &str, max: usize) -> std::result::Result<String, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(format!("a {what} proposal is empty"));
    }
    if t.chars().count() > max {
        return Err(format!("a {what} proposal is longer than {max} characters"));
    }
    Ok(t.to_string())
}

impl ProposalValue {
    pub fn kind(&self) -> &'static str {
        match self {
            ProposalValue::Location { .. } => "location",
            ProposalValue::Date(_) => "date",
            ProposalValue::Tags(_) => "tags",
            ProposalValue::Label(_) => "label",
            ProposalValue::Person { .. } => "person",
            ProposalValue::Title(_) => "title",
            ProposalValue::Caption(_) => "caption",
        }
    }

    /// The value checked and tidied (trimmed text, a label's canonical name, tags without
    /// duplicates), or the plugin-facing reason it can't be proposed.
    pub fn normalised(self) -> std::result::Result<Self, String> {
        Ok(match self {
            ProposalValue::Location { lat, lon } => {
                if !(lat.is_finite() && lon.is_finite() && (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)) {
                    return Err(format!("a location proposal is off the map ({lat}, {lon})"));
                }
                ProposalValue::Location { lat, lon }
            }
            ProposalValue::Date(d) => {
                let d = d.trim().to_string();
                chrono::NaiveDateTime::parse_from_str(&d, "%Y-%m-%dT%H:%M:%S")
                    .map_err(|_| format!("a date proposal isn't YYYY-MM-DDTHH:MM:SS: {d}"))?;
                ProposalValue::Date(d)
            }
            ProposalValue::Tags(tags) => {
                let mut out: Vec<String> = Vec::new();
                for t in &tags {
                    let t = text(t, "tag", MAX_TAG)?;
                    if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
                        out.push(t);
                    }
                }
                if out.is_empty() {
                    return Err("a tags proposal has no tags".into());
                }
                if out.len() > MAX_TAGS {
                    return Err(format!("a tags proposal has more than {MAX_TAGS} tags"));
                }
                ProposalValue::Tags(out)
            }
            ProposalValue::Label(l) => {
                let name = LABEL_NAMES[1..]
                    .iter()
                    .find(|n| n.eq_ignore_ascii_case(l.trim()))
                    .ok_or_else(|| format!("a label proposal isn't one of Red, Yellow, Green, Blue, Purple: {l}"))?;
                ProposalValue::Label((*name).to_string())
            }
            ProposalValue::Person { face, name } => ProposalValue::Person { face, name: text(&name, "person", MAX_NAME)? },
            ProposalValue::Title(t) => ProposalValue::Title(text(&t, "title", MAX_TITLE)?),
            ProposalValue::Caption(t) => ProposalValue::Caption(text(&t, "caption", MAX_CAPTION)?),
        })
    }

    /// The `value` column: the case's content as JSON.
    fn content_json(&self) -> String {
        let v = serde_json::to_value(self).unwrap_or_default();
        v.get("value").map(|c| c.to_string()).unwrap_or_default()
    }

    fn from_row(kind: &str, value: &str) -> Option<Self> {
        let content: serde_json::Value = serde_json::from_str(value).ok()?;
        serde_json::from_value(serde_json::json!({ "kind": kind, "value": content })).ok()
    }
}

/// One proposal as a plugin submits it.
#[derive(Debug, Clone, PartialEq)]
pub struct NewProposal {
    pub asset_id: i64,
    pub value: ProposalValue,
    /// 0–1.
    pub confidence: f32,
    pub reason: Option<String>,
}

/// Checks and tidies a batch; refused whole on the first bad proposal.
pub fn normalise_batch(batch: Vec<NewProposal>) -> std::result::Result<Vec<NewProposal>, String> {
    if batch.len() > MAX_BATCH {
        return Err(format!("a batch has more than {MAX_BATCH} proposals"));
    }
    batch
        .into_iter()
        .map(|p| {
            if !(p.confidence.is_finite() && (0.0..=1.0).contains(&p.confidence)) {
                return Err(format!("a confidence isn't between 0 and 1: {}", p.confidence));
            }
            let reason = match p.reason.as_deref().map(str::trim) {
                None | Some("") => None,
                Some(r) => Some(text(r, "reason", MAX_REASON)?),
            };
            Ok(NewProposal { asset_id: p.asset_id, value: p.value.normalised()?, confidence: p.confidence, reason })
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProposalStatus {
    Pending,
    Accepted,
    Rejected,
}

impl ProposalStatus {
    fn as_str(self) -> &'static str {
        match self {
            ProposalStatus::Pending => "pending",
            ProposalStatus::Accepted => "accepted",
            ProposalStatus::Rejected => "rejected",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "accepted" => ProposalStatus::Accepted,
            "rejected" => ProposalStatus::Rejected,
            _ => ProposalStatus::Pending,
        }
    }
}

/// One row of the review screen.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Proposal {
    #[ts(type = "number")]
    pub id: i64,
    pub plugin_id: String,
    #[ts(type = "number")]
    pub asset_id: i64,
    pub proposed: ProposalValue,
    pub confidence: f32,
    pub reason: Option<String>,
    pub status: ProposalStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalPage {
    pub items: Vec<Proposal>,
    #[ts(type = "number | null")]
    pub next_cursor: Option<i64>,
}

/// Stores a normalised batch; assets (and, for a person, faces on that asset) that don't exist
/// are skipped. Returns how many rows were stored or changed.
pub fn submit(conn: &Connection, plugin: &str, batch: &[NewProposal], now: i64) -> Result<usize> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO plugin_proposals (plugin_id, asset_id, kind, value, confidence, reason, created_at)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
          WHERE EXISTS (SELECT 1 FROM assets WHERE id = ?2)
            AND (?8 IS NULL OR EXISTS (SELECT 1 FROM faces WHERE id = ?8 AND asset_id = ?2))
         ON CONFLICT (plugin_id, asset_id, kind) DO UPDATE SET
           status     = CASE WHEN value = excluded.value THEN status ELSE 'pending' END,
           decided_at = CASE WHEN value = excluded.value THEN decided_at ELSE NULL END,
           value      = excluded.value,
           confidence = excluded.confidence,
           reason     = excluded.reason",
    )?;
    let mut n = 0;
    for p in batch {
        let face = match &p.value {
            ProposalValue::Person { face, .. } => Some(*face),
            _ => None,
        };
        n += stmt.execute(params![plugin, p.asset_id, p.value.kind(), p.value.content_json(), p.confidence, p.reason, now, face])?;
    }
    Ok(n)
}

/// A page of one kind's proposals in `status`, oldest first, optionally from one plugin.
pub fn page(conn: &Connection, kind: &str, status: ProposalStatus, plugin: Option<&str>, after: Option<i64>, limit: usize) -> Result<ProposalPage> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, plugin_id, asset_id, kind, value, confidence, reason, status FROM plugin_proposals
          WHERE kind = ?1 AND status = ?2 AND (?3 IS NULL OR plugin_id = ?3) AND id > ?4
          ORDER BY id LIMIT ?5",
    )?;
    let limit = limit.clamp(1, 500);
    let rows = stmt.query_map(params![kind, status.as_str(), plugin, after.unwrap_or(0), limit as i64 + 1], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, f32>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, String>(7)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (id, plugin_id, asset_id, kind, value, confidence, reason, status) = row?;
        // A row a later Lumen can't read is left out, never an error.
        let Some(proposed) = ProposalValue::from_row(&kind, &value) else { continue };
        items.push(Proposal { id, plugin_id, asset_id, proposed, confidence, reason, status: ProposalStatus::parse(&status) });
    }
    let next_cursor = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|p| p.id)
    } else {
        None
    };
    Ok(ProposalPage { items, next_cursor })
}

/// One proposal by id.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Proposal>> {
    let row = conn
        .query_row(
            "SELECT plugin_id, asset_id, kind, value, confidence, reason, status FROM plugin_proposals WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, f32>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(plugin_id, asset_id, kind, value, confidence, reason, status)| {
        let proposed = ProposalValue::from_row(&kind, &value)?;
        Some(Proposal { id, plugin_id, asset_id, proposed, confidence, reason, status: ProposalStatus::parse(&status) })
    }))
}

/// Marks `ids` accepted, rejected or pending again (undo); returns how many changed.
pub fn set_status(conn: &Connection, ids: &[i64], status: ProposalStatus, now: i64) -> Result<usize> {
    let mut stmt = conn.prepare_cached(
        "UPDATE plugin_proposals SET status = ?2, decided_at = CASE WHEN ?2 = 'pending' THEN NULL ELSE ?3 END
          WHERE id = ?1 AND status <> ?2",
    )?;
    let mut n = 0;
    for &id in ids {
        n += stmt.execute(params![id, status.as_str(), now])?;
    }
    Ok(n)
}

/// How many proposals of each kind wait for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProposalCount {
    pub kind: String,
    #[ts(type = "number")]
    pub pending: i64,
}

pub fn pending_counts(conn: &Connection) -> Result<Vec<ProposalCount>> {
    let mut stmt = conn.prepare_cached("SELECT kind, count(*) FROM plugin_proposals WHERE status = 'pending' GROUP BY kind ORDER BY kind")?;
    let rows = stmt.query_map([], |r| Ok(ProposalCount { kind: r.get(0)?, pending: r.get(1)? }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            c.execute("INSERT INTO faces (id, asset_id, x, y, w, h, score, vec, model) VALUES (7, 1, 0, 0, 0.1, 0.1, 0.9, x'00', 'm')", [])?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn p(asset_id: i64, value: ProposalValue) -> NewProposal {
        NewProposal { asset_id, value, confidence: 0.8, reason: None }
    }

    fn pending(db: &Db, kind: &str) -> Vec<Proposal> {
        db.with_conn(|c| page(c, kind, ProposalStatus::Pending, None, None, 50)).unwrap().items
    }

    #[test]
    fn values_are_checked_and_tidied() {
        let ok = normalise_batch(vec![
            p(1, ProposalValue::Label(" green ".into())),
            p(1, ProposalValue::Tags(vec!["Beach".into(), "beach".into(), " sea ".into()])),
            p(1, ProposalValue::Title("  Sunset  ".into())),
        ])
        .unwrap();
        assert_eq!(ok[0].value, ProposalValue::Label("Green".into()));
        assert_eq!(ok[1].value, ProposalValue::Tags(vec!["Beach".into(), "sea".into()]));
        assert_eq!(ok[2].value, ProposalValue::Title("Sunset".into()));

        for bad in [
            ProposalValue::Location { lat: 91.0, lon: 0.0 },
            ProposalValue::Location { lat: f64::NAN, lon: 0.0 },
            ProposalValue::Date("2021-05-04".into()),
            ProposalValue::Label("Orange".into()),
            ProposalValue::Tags(vec![]),
            ProposalValue::Tags(vec!["  ".into()]),
            ProposalValue::Caption("".into()),
            ProposalValue::Title("x".repeat(201)),
        ] {
            assert!(normalise_batch(vec![p(1, bad.clone())]).is_err(), "{bad:?}");
        }
        let mut c = p(1, ProposalValue::Title("t".into()));
        c.confidence = 1.5;
        assert!(normalise_batch(vec![c]).is_err());
        assert!(normalise_batch((0..=MAX_BATCH as i64).map(|i| p(i, ProposalValue::Title("t".into()))).collect()).is_err());
        assert!(normalise_batch(vec![p(1, ProposalValue::Date("2021-05-04T10:00:00".into()))]).is_ok());
    }

    #[test]
    fn submit_stores_one_row_per_plugin_asset_and_kind() {
        let db = library();
        db.with_conn(|c| {
            submit(c, "a", &[p(1, ProposalValue::Title("One".into())), p(2, ProposalValue::Location { lat: 46.0, lon: 14.0 })], 100)?;
            // Again with a new title: replaced, not added. Another plugin: its own row.
            submit(c, "a", &[p(1, ProposalValue::Title("Two".into()))], 101)?;
            submit(c, "b", &[p(1, ProposalValue::Title("Other".into()))], 102)
        })
        .unwrap();
        let titles = pending(&db, "title");
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0].proposed, ProposalValue::Title("Two".into()));
        assert_eq!(titles[1].plugin_id, "b");
        assert_eq!(pending(&db, "location")[0].proposed, ProposalValue::Location { lat: 46.0, lon: 14.0 });
        let only_b = db.with_conn(|c| page(c, "title", ProposalStatus::Pending, Some("b"), None, 50)).unwrap();
        assert_eq!(only_b.items.len(), 1);
    }

    #[test]
    fn missing_assets_and_faces_are_skipped() {
        let db = library();
        let n = db
            .with_conn(|c| {
                submit(
                    c,
                    "a",
                    &[
                        p(99, ProposalValue::Title("gone".into())),
                        p(1, ProposalValue::Person { face: 7, name: "Ana".into() }),
                        // Face 7 is on asset 1, not 2.
                        p(2, ProposalValue::Person { face: 7, name: "Ana".into() }),
                    ],
                    100,
                )
            })
            .unwrap();
        assert_eq!(n, 1);
        assert_eq!(pending(&db, "person")[0].asset_id, 1);
        assert!(pending(&db, "title").is_empty());
    }

    #[test]
    fn a_rejected_proposal_stays_rejected_unless_its_value_changes() {
        let db = library();
        let id = db
            .with_conn(|c| {
                submit(c, "a", &[p(1, ProposalValue::Caption("A dog".into()))], 100)?;
                Ok(page(c, "caption", ProposalStatus::Pending, None, None, 10)?.items[0].id)
            })
            .unwrap();
        db.with_conn(|c| set_status(c, &[id], ProposalStatus::Rejected, 110)).unwrap();
        db.with_conn(|c| submit(c, "a", &[p(1, ProposalValue::Caption("A dog".into()))], 120)).unwrap();
        assert!(pending(&db, "caption").is_empty());
        db.with_conn(|c| submit(c, "a", &[p(1, ProposalValue::Caption("A cat".into()))], 130)).unwrap();
        let back = pending(&db, "caption");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].id, id);
        assert_eq!(back[0].proposed, ProposalValue::Caption("A cat".into()));
    }

    #[test]
    fn pages_are_keyset_and_status_changes_count() {
        let db = library();
        db.with_conn(|c| {
            submit(c, "a", &(1..=3).map(|i| p(i, ProposalValue::Label("Red".into()))).collect::<Vec<_>>(), 100)
        })
        .unwrap();
        let first = db.with_conn(|c| page(c, "label", ProposalStatus::Pending, None, None, 2)).unwrap();
        assert_eq!(first.items.len(), 2);
        let rest = db.with_conn(|c| page(c, "label", ProposalStatus::Pending, None, first.next_cursor, 2)).unwrap();
        assert_eq!(rest.items.len(), 1);
        assert_eq!(rest.next_cursor, None);

        let ids: Vec<i64> = first.items.iter().map(|p| p.id).collect();
        assert_eq!(db.with_conn(|c| set_status(c, &ids, ProposalStatus::Accepted, 200)).unwrap(), 2);
        assert_eq!(db.with_conn(|c| set_status(c, &ids, ProposalStatus::Accepted, 201)).unwrap(), 0);
        assert_eq!(db.with_conn(pending_counts).unwrap(), [ProposalCount { kind: "label".into(), pending: 1 }]);
        let got = db.with_conn(|c| get(c, ids[0])).unwrap().unwrap();
        assert_eq!(got.status, ProposalStatus::Accepted);
        // Undo puts it back.
        db.with_conn(|c| set_status(c, &ids, ProposalStatus::Pending, 202)).unwrap();
        assert_eq!(pending(&db, "label").len(), 3);
    }

    #[test]
    fn deleting_an_asset_drops_its_proposals() {
        let db = library();
        db.with_conn(|c| {
            submit(c, "a", &[p(2, ProposalValue::Title("t".into()))], 100)?;
            c.execute("DELETE FROM assets WHERE id = 2", [])?;
            Ok(())
        })
        .unwrap();
        assert!(pending(&db, "title").is_empty());
    }
}
