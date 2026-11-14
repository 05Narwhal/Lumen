//! Pets (Round 4C): cats and dogs, one group per animal, by name, like People.
//!
//! Detections (D-FINE-S boxes, described by DINOv2 ViT-S/14) are stored in `pet_detections`
//! and grouped into `pets` by People's own grouping (`people::ReclusterInput::from_rows`), run
//! once per species so a cat and a dog are never one pet. The tables mirror People's (migration
//! 0025), and so do the rules: a detection the user placed (pinned) never moves, one they took
//! out of a pet is never offered to it again (`pet_rejections`), *Not a pet* rejects every
//! detection of a pet, *Remove from Pets* keeps it out of sight with its look-alikes.
//!
//! Thresholds come from public data only (`scripts/ml/eval_pets.py`, `tests/pets_grouping_public`).
//! Two look-alike pets come out of a first look as one; *Split* separates them once, and then
//! their new photos are placed apart or asked about.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::embedding::Vector;
use crate::people::{GroupingRows, ReclusterInput, Suggestion, Thresholds};
use crate::{CoreError, Result};

pub const SPECIES: [&str; 2] = ["cat", "dog"];

/// Measured on DogFaceNet (2,323 detector crops of 500 dogs, `tests/pets_grouping_public`):
/// households of 2–4 random dogs group at 96 % pair precision; once pets are named, 80 % of new
/// photos are placed, all of them on the right pet. The 40 most alike pairs of dogs merge on a
/// first look; split once, 46 % of their new photos are placed (98 % right) and the rest asked.
pub const MEASURED: Thresholds = Thresholds { join: 0.55, margin: 0.10, suggest: 0.45, cluster: 0.60, place_quality: 0.0 };

/// Detections boxes overlapping this much are the same animal on a rescan.
const SAME_IOU: f32 = 0.5;

fn blob(v: &[i8]) -> Vec<u8> {
    v.iter().map(|x| *x as u8).collect()
}

fn unblob(b: &[u8]) -> Vector {
    b.iter().map(|x| *x as i8).collect()
}

fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let iy = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let inter = ix * iy;
    inter / (a[2] * a[3] + b[2] * b[3] - inter)
}

/// One animal the scan found: its box (0..1 of the unedited, display-oriented photo), what it is
/// and how it looks.
#[derive(Debug, Clone, PartialEq)]
pub struct NewPetDetection {
    pub species: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub vec: Vector,
    /// `pet_filter::quality`: ranks exemplars and covers.
    pub quality: f32,
    /// `pet_filter::crop_reason` when this is not a pet (a plush, a cartoon).
    pub rejected: Option<String>,
}

/// Stores a photo's scan, keeping who each animal is: a box matching an old one (same
/// species, IoU ≥ 0.5) updates it in place, so its pet, pin and refusals survive a new model.
/// A detection the user touched (placed, refused for someone, *Not a pet*, drawn by hand) is
/// kept even when the scan no longer finds it; the rest that are not found again go.
pub fn replace_detections(conn: &Connection, asset_id: i64, model: &str, found: &[NewPetDetection]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    replace_in(&tx, asset_id, model, found)?;
    tx.commit()?;
    Ok(())
}

/// [`replace_detections`] inside the caller's transaction.
fn replace_in(tx: &Connection, asset_id: i64, model: &str, found: &[NewPetDetection]) -> Result<()> {
    let pets: Vec<i64> = tx.prepare("SELECT DISTINCT pet_id FROM pet_detections WHERE asset_id = ?1 AND pet_id IS NOT NULL")?.query_map([asset_id], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    // (id, species, box, kept, judged by the user)
    let old: Vec<(i64, String, [f32; 4], bool, bool)> = tx
        .prepare(
            "SELECT id, species, x, y, w, h,
                    pinned OR rejected IS 'user' OR source = 'manual' OR EXISTS (SELECT 1 FROM pet_rejections r WHERE r.detection_id = pet_detections.id),
                    pinned OR rejected IS 'user' OR source = 'manual'
             FROM pet_detections WHERE asset_id = ?1",
        )?
        .query_map([asset_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                [r.get::<_, f64>(2)? as f32, r.get::<_, f64>(3)? as f32, r.get::<_, f64>(4)? as f32, r.get::<_, f64>(5)? as f32],
                r.get::<_, i64>(6)? != 0,
                r.get::<_, i64>(7)? != 0,
            ))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (i, f) in found.iter().enumerate() {
        for (j, o) in old.iter().enumerate() {
            let v = iou([f.x, f.y, f.w, f.h], o.2);
            if o.1 == f.species && v >= SAME_IOU {
                pairs.push((v, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (mut new_of, mut old_used) = (vec![None; found.len()], vec![false; old.len()]);
    for (_, i, j) in pairs {
        if new_of[i].is_none() && !old_used[j] {
            new_of[i] = Some(j);
            old_used[j] = true;
        }
    }
    for (j, o) in old.iter().enumerate() {
        if !old_used[j] && !o.3 {
            tx.execute("DELETE FROM pet_detections WHERE id = ?1", [o.0])?;
        }
    }
    {
        let mut ins = tx.prepare("INSERT INTO pet_detections (asset_id, species, x, y, w, h, score, vec, model, quality, rejected) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)")?;
        let mut upd = tx.prepare("UPDATE pet_detections SET x = ?2, y = ?3, w = ?4, h = ?5, score = ?6, vec = ?7, model = ?8, quality = ?9 WHERE id = ?1")?;
        // A machine judgement is redone; a rejected detection has no pet.
        let mut judge = tx.prepare("UPDATE pet_detections SET rejected = ?2, pet_id = CASE WHEN ?2 IS NULL THEN pet_id ELSE NULL END WHERE id = ?1")?;
        for (i, f) in found.iter().enumerate() {
            match new_of[i] {
                Some(j) => {
                    let (id, .., judged) = &old[j];
                    upd.execute(params![id, f.x as f64, f.y as f64, f.w as f64, f.h as f64, f.score as f64, blob(&f.vec), model, f.quality as f64])?;
                    if !judged {
                        judge.execute(params![id, f.rejected])?;
                    }
                }
                None => {
                    ins.execute(params![asset_id, f.species, f.x as f64, f.y as f64, f.w as f64, f.h as f64, f.score as f64, blob(&f.vec), model, f.quality as f64, f.rejected])?;
                }
            }
        }
    }
    mark_scanned(tx, asset_id, model)?;
    fix_covers(tx, &pets)?;
    if !pets.is_empty() {
        crate::fts::index_ids(tx, &[asset_id])?;
    }
    Ok(())
}

/// A batch of the scan's results in one transaction (Round 8A): `Some(found)` stores what a
/// photo shows ([`replace_detections`]); `None` (unreadable, or skipped as showing no animal)
/// only marks it scanned, keeping whatever an earlier scan found.
pub fn save_scans(conn: &Connection, model: &str, scans: &[(i64, Option<Vec<NewPetDetection>>)]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (asset_id, found) in scans {
        match found {
            Some(found) => replace_in(&tx, *asset_id, model, found)?,
            None => mark_scanned(&tx, *asset_id, model)?,
        }
    }
    tx.commit()?;
    Ok(())
}

/// Splits a batch of unscanned photos (Round 8A): `(look, skip)`. The detector may skip a photo
/// whose CLIP embedding shows no dog or cat ([`crate::pet_filter::may_show_a_pet`]) and in which
/// no earlier scan found an animal; a photo without an embedding is always looked at.
pub fn split_for_detector(conn: &Connection, ids: &[i64], protos: &crate::embedding::Prototypes) -> Result<(Vec<i64>, Vec<i64>)> {
    let vecs = crate::embedding::load(conn, ids)?;
    let found = with_detections(conn, ids)?;
    Ok(ids.iter().partition(|id| found.contains(id) || vecs.get(id).is_none_or(|f| crate::pet_filter::may_show_a_pet(&f.vec, protos))))
}

/// Which of `ids` already have a detection of any kind: the scan never skips those unseen, so
/// a prefilter can only spare the detector, never forget an animal.
pub fn with_detections(conn: &Connection, ids: &[i64]) -> Result<std::collections::HashSet<i64>> {
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!("SELECT DISTINCT asset_id FROM pet_detections WHERE asset_id IN ({list})"))?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

pub fn mark_scanned(conn: &Connection, asset_id: i64, model: &str) -> Result<()> {
    conn.execute("INSERT INTO pet_scan (asset_id, model) VALUES (?1, ?2) ON CONFLICT(asset_id) DO UPDATE SET model = excluded.model", params![asset_id, model])?;
    Ok(())
}

const UNSCANNED: &str = "FROM assets a LEFT JOIN pet_scan s ON s.asset_id = a.id
     WHERE a.state = 0 AND a.media_type != 1 AND (s.asset_id IS NULL OR s.model != ?1)";

/// Photos the pet scan has not looked at with `model`, newest first: (id, content hash).
pub fn unscanned(conn: &Connection, model: &str, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(&format!("SELECT a.id, a.content_hash {UNSCANNED} AND NOT {} ORDER BY a.id DESC LIMIT ?2", crate::assets::MOVIE_HALF))?;
    let rows = stmt.query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn unscanned_count(conn: &Connection, model: &str) -> Result<usize> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) {UNSCANNED} AND NOT {}", crate::assets::MOVIE_HALF), [model], |r| r.get(0))?;
    Ok(n as usize)
}

/// One species' grouping input, read from the pet tables.
fn grouping_rows(conn: &Connection, species: &str) -> Result<GroupingRows> {
    let members = conn
        .prepare("SELECT id, asset_id, pet_id, pinned, vec, COALESCE(quality, score) FROM pet_detections WHERE rejected IS NULL AND species = ?1")?
        .query_map([species], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, i64>(3)? != 0, unblob(&r.get::<_, Vec<u8>>(4)?), r.get::<_, f64>(5)? as f32, false)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let evidence = conn
        .prepare(
            "SELECT pet_id, vec, COALESCE(quality, score), pinned FROM pet_detections
             WHERE pet_id IS NOT NULL AND rejected IS NULL AND COALESCE(quality, score) > 0 AND species = ?1
             ORDER BY pet_id, COALESCE(quality, score) DESC, id",
        )?
        .query_map([species], |r| Ok((r.get::<_, i64>(0)?, unblob(&r.get::<_, Vec<u8>>(1)?), r.get::<_, f64>(2)? as f32, r.get::<_, i64>(3)? != 0)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let refusals = conn
        .prepare("SELECT r.detection_id, r.pet_id FROM pet_rejections r JOIN pet_detections d ON d.id = r.detection_id WHERE d.species = ?1")?
        .query_map([species], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let dated = conn
        .prepare(
            "SELECT a.id, a.captured_at FROM assets a
             WHERE a.captured_at IS NOT NULL AND EXISTS (SELECT 1 FROM pet_detections d WHERE d.asset_id = a.id AND d.species = ?1)
             ORDER BY a.captured_at, a.id",
        )?
        .query_map([species], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(GroupingRows { members, evidence, refusals, dated })
}

/// Places detections on pets and forms new ones, one species at a time (People's grouping on
/// the pet tables). Placing is guarded as People's is: only a detection still open, only on a pet
/// that still exists. Returns how many found a pet.
pub fn recluster(conn: &Connection, t: &Thresholds, now: i64) -> Result<usize> {
    let mut plans = Vec::new();
    for species in SPECIES {
        plans.push((species, ReclusterInput::from_rows(grouping_rows(conn, species)?).plan(t)));
    }
    let tx = conn.unchecked_transaction()?;
    let mut placed = 0;
    let mut touched: Vec<i64> = Vec::new();
    {
        let mut set = tx.prepare(
            "UPDATE pet_detections SET pet_id = ?2
             WHERE id = ?1 AND pet_id IS NULL AND pinned = 0 AND rejected IS NULL AND EXISTS (SELECT 1 FROM pets WHERE id = ?2)",
        )?;
        for (species, plan) in &plans {
            for &(det, pet, asset) in plan.places() {
                if set.execute(params![det, pet])? > 0 {
                    placed += 1;
                    touched.push(asset);
                }
            }
            for (cover, members) in plan.new_groups() {
                tx.execute("INSERT INTO pets (species, created_at, cover_detection_id) VALUES (?1, ?2, ?3)", params![species, now, cover])?;
                let pet = tx.last_insert_rowid();
                for &(det, asset) in members {
                    if set.execute(params![det, pet])? > 0 {
                        placed += 1;
                        touched.push(asset);
                    }
                }
            }
        }
    }
    tx.execute("DELETE FROM pets WHERE name IS NULL AND NOT EXISTS (SELECT 1 FROM pet_detections d WHERE d.pet_id = pets.id)", [])?;
    let pets: Vec<i64> = tx.prepare("SELECT id FROM pets")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    fix_covers(&tx, &pets)?;
    tx.commit()?;
    touched.sort_unstable();
    touched.dedup();
    crate::fts::index_ids(conn, &touched)?;
    Ok(placed)
}

/// Photos that may show `pet`, asked about on its page ("Is this Milo?").
pub fn suggestions(conn: &Connection, pet: i64, t: &Thresholds, limit: usize) -> Result<Vec<Suggestion>> {
    let Some(species) = conn.query_row("SELECT species FROM pets WHERE id = ?1", [pet], |r| r.get::<_, String>(0)).optional()? else { return Ok(Vec::new()) };
    let rows = grouping_rows(conn, &species)?;
    let open = rows.members.iter().filter(|m| m.2.is_none() && !m.3).map(|m| (m.0, m.1, m.4.clone(), 1.0)).collect();
    Ok(crate::people::suggestions_from(rows.evidence, rows.refusals, open, pet, t, limit))
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PetSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: Option<String>,
    /// "cat" or "dog".
    pub species: String,
    #[ts(type = "number")]
    pub detection_count: i64,
    #[ts(type = "number")]
    pub photo_count: i64,
    #[ts(type = "number | null")]
    pub cover_detection_id: Option<i64>,
}

fn pets_with_status(conn: &Connection, status: &str) -> Result<Vec<PetSummary>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.species, COUNT(d.id), COUNT(DISTINCT d.asset_id), p.cover_detection_id
         FROM pets p JOIN pet_detections d ON d.pet_id = p.id JOIN assets a ON a.id = d.asset_id AND a.state = 0 AND a.concealed = 0
         WHERE p.status = ?1
         GROUP BY p.id
         ORDER BY p.name IS NULL, p.name COLLATE NOCASE, COUNT(d.id) DESC, p.id",
    )?;
    let rows = stmt.query_map([status], |r| {
        Ok(PetSummary { id: r.get(0)?, name: r.get(1)?, species: r.get(2)?, detection_count: r.get(3)?, photo_count: r.get(4)?, cover_detection_id: r.get(5)? })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Every pet shown on the Pets page: named first.
pub fn list_pets(conn: &Connection) -> Result<Vec<PetSummary>> {
    pets_with_status(conn, "listed")
}

/// Pets taken off the Pets page with *Remove from Pets* (Settings → People and pets).
pub fn removed_pets(conn: &Connection) -> Result<Vec<PetSummary>> {
    pets_with_status(conn, "removed")
}

fn assets_of(conn: &Connection, pet: i64) -> Result<Vec<i64>> {
    Ok(conn.prepare("SELECT DISTINCT asset_id FROM pet_detections WHERE pet_id = ?1")?.query_map([pet], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?)
}

/// Names a pet (an empty name forgets it); the name joins search.
pub fn name_pet(conn: &Connection, pet: i64, name: &str) -> Result<()> {
    let name = name.trim();
    let n = conn.execute("UPDATE pets SET name = ?2 WHERE id = ?1", params![pet, (!name.is_empty()).then_some(name)])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("pet {pet}")));
    }
    crate::fts::index_ids(conn, &assets_of(conn, pet)?)?;
    Ok(())
}

fn fix_covers(conn: &Connection, pets: &[i64]) -> Result<()> {
    for p in pets {
        conn.execute(
            "UPDATE pets SET cover_detection_id = (SELECT id FROM pet_detections WHERE pet_id = ?1 ORDER BY COALESCE(quality, score) DESC, id LIMIT 1)
             WHERE id = ?1 AND (cover_detection_id IS NULL OR cover_detection_id NOT IN (SELECT id FROM pet_detections WHERE pet_id = ?1))",
            [p],
        )?;
    }
    Ok(())
}

/// *Use as main photo*. Returns the cover it had, for Undo.
pub fn set_cover(conn: &Connection, pet: i64, detection: i64) -> Result<Option<i64>> {
    let theirs: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM pet_detections WHERE id = ?1 AND pet_id = ?2 AND rejected IS NULL)", params![detection, pet], |r| r.get(0))?;
    if !theirs {
        return Err(CoreError::Invalid("That photo is not of this pet.".into()));
    }
    let before: Option<i64> = conn.query_row("SELECT cover_detection_id FROM pets WHERE id = ?1", [pet], |r| r.get(0))?;
    conn.execute("UPDATE pets SET cover_detection_id = ?2 WHERE id = ?1", params![pet, detection])?;
    Ok(before)
}

/// A pet as it was, for Undo of a merge, a split or *Not a pet*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PetRow {
    #[ts(type = "number")]
    pub id: i64,
    pub name: Option<String>,
    pub species: String,
    pub status: String,
    #[ts(type = "number | null")]
    pub cover_detection_id: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
}

fn pet_row(conn: &Connection, pet: i64) -> Result<PetRow> {
    conn.query_row("SELECT id, name, species, status, cover_detection_id, created_at FROM pets WHERE id = ?1", [pet], |r| {
        Ok(PetRow { id: r.get(0)?, name: r.get(1)?, species: r.get(2)?, status: r.get(3)?, cover_detection_id: r.get(4)?, created_at: r.get(5)? })
    })
    .optional()?
    .ok_or_else(|| CoreError::NotFound(format!("pet {pet}")))
}

fn insert_row(conn: &Connection, p: &PetRow) -> Result<()> {
    conn.execute(
        "INSERT INTO pets (id, name, species, status, cover_detection_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![p.id, p.name, p.species, p.status, p.cover_detection_id, p.created_at],
    )?;
    Ok(())
}

/// What Undo needs after `from` went into `into`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PetMergeUndo {
    pub from: PetRow,
    #[ts(type = "number")]
    pub into: i64,
    #[ts(type = "number[]")]
    pub detections: Vec<i64>,
    /// Whether `into` took `from`'s name.
    pub took_name: bool,
    /// Refusals of `from` that `into` did not have, now `into`'s.
    #[ts(type = "number[]")]
    pub moved_refusals: Vec<i64>,
    /// `from`'s species, which its photos had before they took `into`'s (Round 9A).
    pub from_species: String,
}

/// Two pets are one: `from`'s photos and refusals go to `into`, which keeps its name or takes
/// `from`'s. Across species (Round 9A: the detector called a cat a dog), `into`'s species wins
/// and `from`'s photos take it.
pub fn merge_pets(conn: &Connection, into: i64, from: i64) -> Result<PetMergeUndo> {
    let (a, b) = (pet_row(conn, into)?, pet_row(conn, from)?);
    if into == from {
        return Err(CoreError::Invalid("That is the same pet.".into()));
    }
    let tx = conn.unchecked_transaction()?;
    let detections: Vec<i64> = tx.prepare("SELECT id FROM pet_detections WHERE pet_id = ?1 ORDER BY id")?.query_map([from], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    let moved_refusals: Vec<i64> = tx
        .prepare("SELECT detection_id FROM pet_rejections r WHERE pet_id = ?1 AND NOT EXISTS (SELECT 1 FROM pet_rejections o WHERE o.detection_id = r.detection_id AND o.pet_id = ?2) ORDER BY detection_id")?
        .query_map(params![from, into], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    for d in &moved_refusals {
        tx.execute("INSERT OR IGNORE INTO pet_rejections (detection_id, pet_id, created_at) VALUES (?1, ?2, ?3)", params![d, into, crate::now_unix()])?;
    }
    // What `into` refused of `from`'s own photos, the merge now says is `into`.
    tx.execute("DELETE FROM pet_rejections WHERE pet_id = ?2 AND detection_id IN (SELECT id FROM pet_detections WHERE pet_id = ?1)", params![from, into])?;
    tx.execute("UPDATE pet_detections SET pet_id = ?1, species = ?3 WHERE pet_id = ?2", params![into, from, a.species])?;
    let took_name = a.name.is_none() && b.name.is_some();
    if took_name {
        tx.execute("UPDATE pets SET name = ?2 WHERE id = ?1", params![into, b.name])?;
    }
    tx.execute("DELETE FROM pets WHERE id = ?1", [from])?;
    fix_covers(&tx, &[into])?;
    crate::fts::index_ids(&tx, &assets_of(&tx, into)?)?;
    tx.commit()?;
    let from_species = b.species.clone();
    Ok(PetMergeUndo { from: b, into, detections, took_name, moved_refusals, from_species })
}

/// Undoes [`merge_pets`].
pub fn undo_merge(conn: &Connection, undo: &PetMergeUndo) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    insert_row(&tx, &undo.from)?;
    for d in &undo.detections {
        tx.execute("UPDATE pet_detections SET pet_id = ?2, species = ?3 WHERE id = ?1", params![d, undo.from.id, undo.from_species])?;
    }
    for d in &undo.moved_refusals {
        tx.execute("DELETE FROM pet_rejections WHERE detection_id = ?1 AND pet_id = ?2", params![d, undo.into])?;
        tx.execute("INSERT OR IGNORE INTO pet_rejections (detection_id, pet_id, created_at) VALUES (?1, ?2, ?3)", params![d, undo.from.id, crate::now_unix()])?;
    }
    if undo.took_name {
        tx.execute("UPDATE pets SET name = NULL WHERE id = ?1", [undo.into])?;
    }
    fix_covers(&tx, &[undo.into, undo.from.id])?;
    let mut assets = assets_of(&tx, undo.into)?;
    assets.extend(assets_of(&tx, undo.from.id)?);
    crate::fts::index_ids(&tx, &assets)?;
    tx.commit()?;
    Ok(())
}

/// Where a detection was before the user moved it, for Undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PetPlace {
    #[ts(type = "number")]
    pub detection: i64,
    #[ts(type = "number | null")]
    pub pet: Option<i64>,
    pub pinned: bool,
}

/// *Split*: these photos of `pet` show another animal (two look-alikes grouped as one). They
/// become a new pet of the same species, placed there for good, and are never offered to
/// `pet` again; their look-alikes follow by the usual rules. Returns the new pet and Undo.
pub fn split(conn: &Connection, pet: i64, assets: &[i64], now: i64) -> Result<(i64, Vec<PetPlace>)> {
    let row = pet_row(conn, pet)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO pets (species, created_at) VALUES (?1, ?2)", params![row.species, now])?;
    let new_pet = tx.last_insert_rowid();
    let moved = move_out(&tx, pet, assets, Some(new_pet))?;
    if moved.is_empty() {
        return Err(CoreError::Invalid("None of those photos are of this pet.".into()));
    }
    fix_covers(&tx, &[pet, new_pet])?;
    tx.commit()?;
    Ok((new_pet, moved))
}

/// The photos are not of `pet`: its detections there go to `to` (placed for good) or back into
/// play, refused by `pet` either way. Returns where each was, for Undo ([`restore_places`]).
pub fn move_photos(conn: &Connection, pet: i64, assets: &[i64], to: Option<i64>) -> Result<Vec<PetPlace>> {
    let tx = conn.unchecked_transaction()?;
    let moved = move_out(&tx, pet, assets, to)?;
    fix_covers(&tx, &[pet].into_iter().chain(to).collect::<Vec<_>>())?;
    tx.commit()?;
    Ok(moved)
}

fn move_out(tx: &Connection, pet: i64, assets: &[i64], to: Option<i64>) -> Result<Vec<PetPlace>> {
    let mut moved = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id, pinned FROM pet_detections WHERE pet_id = ?1 AND asset_id = ?2 ORDER BY id")?;
        for &asset in assets {
            for p in stmt.query_map(params![pet, asset], |r| Ok(PetPlace { detection: r.get(0)?, pet: Some(pet), pinned: r.get::<_, i64>(1)? != 0 }))? {
                moved.push(p?);
            }
        }
    }
    for p in &moved {
        tx.execute("UPDATE pet_detections SET pet_id = ?2, pinned = (?2 IS NOT NULL) WHERE id = ?1", params![p.detection, to])?;
        tx.execute("INSERT OR IGNORE INTO pet_rejections (detection_id, pet_id, created_at) VALUES (?1, ?2, ?3)", params![p.detection, pet, crate::now_unix()])?;
    }
    crate::fts::index_ids(tx, assets)?;
    Ok(moved)
}

/// Puts detections back where they were (Undo of a move or a split) and lifts the refusals the
/// move recorded. A pet left with nothing and no name goes.
pub fn restore_places(conn: &Connection, places: &[PetPlace]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let mut pets = Vec::new();
    let mut assets = Vec::new();
    for p in places {
        let (asset, now_on): (i64, Option<i64>) = tx.query_row("SELECT asset_id, pet_id FROM pet_detections WHERE id = ?1", [p.detection], |r| Ok((r.get(0)?, r.get(1)?)))?;
        tx.execute("UPDATE pet_detections SET pet_id = ?2, pinned = ?3 WHERE id = ?1", params![p.detection, p.pet, p.pinned])?;
        if let Some(was) = p.pet {
            tx.execute("DELETE FROM pet_rejections WHERE detection_id = ?1 AND pet_id = ?2", params![p.detection, was])?;
            pets.push(was);
        }
        pets.extend(now_on);
        assets.push(asset);
    }
    tx.execute("DELETE FROM pets WHERE name IS NULL AND NOT EXISTS (SELECT 1 FROM pet_detections d WHERE d.pet_id = pets.id)", [])?;
    pets.retain(|p| tx.query_row("SELECT EXISTS (SELECT 1 FROM pets WHERE id = ?1)", [p], |r| r.get::<_, bool>(0)).unwrap_or(false));
    fix_covers(&tx, &pets)?;
    crate::fts::index_ids(&tx, &assets)?;
    tx.commit()?;
    Ok(())
}

/// *Tag…*: this detection is `pet`, placed for good. Returns where it was, for Undo.
pub fn tag_detection(conn: &Connection, detection: i64, pet: i64) -> Result<PetPlace> {
    let (asset, was, pinned, species): (i64, Option<i64>, bool, String) =
        conn.query_row("SELECT asset_id, pet_id, pinned, species FROM pet_detections WHERE id = ?1", [detection], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?;
    if pet_row(conn, pet)?.species != species {
        return Err(CoreError::Invalid(format!("That is a {species}.")));
    }
    conn.execute("UPDATE pet_detections SET pet_id = ?2, pinned = 1, rejected = NULL WHERE id = ?1", params![detection, pet])?;
    conn.execute("DELETE FROM pet_rejections WHERE detection_id = ?1 AND pet_id = ?2", params![detection, pet])?;
    fix_covers(conn, &[pet].into_iter().chain(was).collect::<Vec<_>>())?;
    crate::fts::index_ids(conn, &[asset])?;
    Ok(PetPlace { detection, pet: was, pinned })
}

/// "Is this Milo?" — No: the detection is never offered to `pet` again.
pub fn refuse(conn: &Connection, detection: i64, pet: i64, now: i64) -> Result<()> {
    conn.execute("INSERT OR IGNORE INTO pet_rejections (detection_id, pet_id, created_at) VALUES (?1, ?2, ?3)", params![detection, pet, now])?;
    Ok(())
}

/// Undo of [`refuse`].
pub fn lift_refusal(conn: &Connection, detection: i64, pet: i64) -> Result<()> {
    conn.execute("DELETE FROM pet_rejections WHERE detection_id = ?1 AND pet_id = ?2", params![detection, pet])?;
    Ok(())
}

/// *Remove from Pets*: off the page, out of search; its photos keep it, so its look-alikes join
/// it quietly rather than coming back as a new pet.
pub fn remove_pet(conn: &Connection, pet: i64) -> Result<()> {
    conn.execute("UPDATE pets SET status = 'removed' WHERE id = ?1", [pet])?;
    crate::fts::index_ids(conn, &assets_of(conn, pet)?)?;
    Ok(())
}

pub fn restore_removed(conn: &Connection, pet: i64) -> Result<()> {
    conn.execute("UPDATE pets SET status = 'listed' WHERE id = ?1", [pet])?;
    crate::fts::index_ids(conn, &assets_of(conn, pet)?)?;
    Ok(())
}

/// What Undo needs after *Not a pet*.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NotAPetUndo {
    pub pet: PetRow,
    #[ts(type = "number[]")]
    pub detections: Vec<i64>,
}

/// *Not a pet*: every detection of it is rejected (never grouped again) and the pet goes.
pub fn not_a_pet(conn: &Connection, pet: i64) -> Result<NotAPetUndo> {
    let row = pet_row(conn, pet)?;
    let tx = conn.unchecked_transaction()?;
    let detections: Vec<i64> = tx.prepare("SELECT id FROM pet_detections WHERE pet_id = ?1 ORDER BY id")?.query_map([pet], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    let assets = assets_of(&tx, pet)?;
    tx.execute("UPDATE pet_detections SET rejected = 'user', pet_id = NULL WHERE pet_id = ?1", [pet])?;
    tx.execute("DELETE FROM pets WHERE id = ?1", [pet])?;
    crate::fts::index_ids(&tx, &assets)?;
    tx.commit()?;
    Ok(NotAPetUndo { pet: row, detections })
}

pub fn undo_not_a_pet(conn: &Connection, undo: &NotAPetUndo) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    insert_row(&tx, &undo.pet)?;
    for d in &undo.detections {
        tx.execute("UPDATE pet_detections SET rejected = NULL, pet_id = ?2 WHERE id = ?1", params![d, undo.pet.id])?;
    }
    crate::fts::index_ids(&tx, &assets_of(&tx, undo.pet.id)?)?;
    tx.commit()?;
    Ok(())
}

/// A pet in a photo, for the viewer's info panel.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AssetPet {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub pet_id: Option<i64>,
    pub name: Option<String>,
    pub species: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The pets in a photo (not rejected, not on a removed pet), unknown ones included.
pub fn asset_pets(conn: &Connection, asset: i64) -> Result<Vec<AssetPet>> {
    let mut stmt = conn.prepare(
        "SELECT d.id, CASE WHEN p.status = 'listed' THEN d.pet_id END, CASE WHEN p.status = 'listed' THEN p.name END, d.species, d.x, d.y, d.w, d.h
         FROM pet_detections d LEFT JOIN pets p ON p.id = d.pet_id
         WHERE d.asset_id = ?1 AND d.rejected IS NULL AND (p.id IS NULL OR p.status = 'listed')
         ORDER BY d.x",
    )?;
    let rows = stmt.query_map([asset], |r| {
        Ok(AssetPet {
            id: r.get(0)?,
            pet_id: r.get(1)?,
            name: r.get(2)?,
            species: r.get(3)?,
            x: r.get::<_, f64>(4)? as f32,
            y: r.get::<_, f64>(5)? as f32,
            w: r.get::<_, f64>(6)? as f32,
            h: r.get::<_, f64>(7)? as f32,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// A detection's photo and box, for its crop.
pub fn detection_rect(conn: &Connection, detection: i64) -> Result<Option<(i64, [f32; 4])>> {
    Ok(conn
        .query_row("SELECT asset_id, x, y, w, h FROM pet_detections WHERE id = ?1", [detection], |r| {
            Ok((r.get::<_, i64>(0)?, [r.get::<_, f64>(1)? as f32, r.get::<_, f64>(2)? as f32, r.get::<_, f64>(3)? as f32, r.get::<_, f64>(4)? as f32]))
        })
        .optional()?)
}

// Round 8B: one box at a time, from the viewer's pet boxes and the info panel's rows.

/// Where one detection is, read for a change to it: (photo, pet, pinned).
fn detection_place(conn: &Connection, detection: i64) -> Result<(i64, Option<i64>, bool)> {
    conn.query_row("SELECT asset_id, pet_id, pinned FROM pet_detections WHERE id = ?1", [detection], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("pet detection {detection}")))
}

/// *Not {name}* on one box: it becomes an unknown pet again, refused by the pet it was, so
/// grouping never puts it back there. Undo is [`restore_places`].
pub fn untag_detection(conn: &Connection, detection: i64) -> Result<PetPlace> {
    let (asset, pet, pinned) = detection_place(conn, detection)?;
    let Some(pet) = pet else { return Err(CoreError::Invalid("That pet has no name to take off.".into())) };
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE pet_detections SET pet_id = NULL, pinned = 0 WHERE id = ?1", [detection])?;
    tx.execute("INSERT OR IGNORE INTO pet_rejections (detection_id, pet_id, created_at) VALUES (?1, ?2, ?3)", params![detection, pet, crate::now_unix()])?;
    fix_covers(&tx, &[pet])?;
    crate::fts::index_ids(&tx, &[asset])?;
    tx.commit()?;
    Ok(PetPlace { detection, pet: Some(pet), pinned })
}

/// *Not a pet* on one box: rejected by the user, out of its pet and never grouped again; the
/// rest of the pet stays. Undo is [`undo_reject_detection`].
pub fn reject_detection(conn: &Connection, detection: i64) -> Result<PetPlace> {
    let (asset, pet, pinned) = detection_place(conn, detection)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE pet_detections SET rejected = 'user', pet_id = NULL, pinned = 0 WHERE id = ?1", [detection])?;
    tx.execute("UPDATE pets SET cover_detection_id = NULL WHERE cover_detection_id = ?1", [detection])?;
    fix_covers(&tx, &pet.into_iter().collect::<Vec<_>>())?;
    crate::fts::index_ids(&tx, &[asset])?;
    tx.commit()?;
    Ok(PetPlace { detection, pet, pinned })
}

/// Undo of [`reject_detection`]: a pet again, back where it was.
pub fn undo_reject_detection(conn: &Connection, place: &PetPlace) -> Result<()> {
    conn.execute("UPDATE pet_detections SET rejected = NULL WHERE id = ?1", [place.detection])?;
    restore_places(conn, std::slice::from_ref(place))
}

/// *New pet…* on one box: a new pet called `name`, of the box's species, holding it (placed for
/// good, and its cover). Returns the pet and where the box was, for [`undo_create_pet`].
pub fn create_pet_with_detection(conn: &Connection, detection: i64, name: &str, now: i64) -> Result<(i64, PetPlace)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(CoreError::Invalid("A pet needs a name.".into()));
    }
    let species: String = conn
        .query_row("SELECT species FROM pet_detections WHERE id = ?1", [detection], |r| r.get(0))
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("pet detection {detection}")))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO pets (name, species, created_at) VALUES (?1, ?2, ?3)", params![name, species, now])?;
    let pet = tx.last_insert_rowid();
    let place = tag_detection(&tx, detection, pet)?;
    tx.execute("UPDATE pets SET cover_detection_id = ?2 WHERE id = ?1", params![pet, detection])?;
    tx.commit()?;
    Ok((pet, place))
}

/// Undo of [`create_pet_with_detection`]: the box goes back, and the pet goes if nothing else
/// was put on it since.
pub fn undo_create_pet(conn: &Connection, pet: i64, place: &PetPlace) -> Result<()> {
    restore_places(conn, std::slice::from_ref(place))?;
    let empty: bool = conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM pet_detections WHERE pet_id = ?1)", [pet], |r| r.get(0))?;
    if empty {
        conn.execute("DELETE FROM pet_rejections WHERE pet_id = ?1", [pet])?;
        conn.execute("DELETE FROM pets WHERE id = ?1", [pet])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for i in 1..=12 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at)
                     VALUES (?1, ?2, 'p', ?3, 'jpg', 0, 1, 1, 10, ?4)",
                    params![i, format!("h{i}"), format!("{i}.jpg"), 1_000 + i * 60],
                )?;
            }
            crate::fts::rebuild(c)?;
            Ok(())
        })
        .unwrap();
        db
    }

    /// A unit vector near axis `k`, nudged by `j` so photos of one animal differ a little.
    fn near(k: usize, j: usize) -> Vector {
        let mut v = vec![0i8; 32];
        v[k] = 120;
        v[16 + (j % 16)] = 20;
        v
    }

    #[test]
    fn a_batch_saves_at_once_and_a_skip_keeps_what_was_found() {
        let db = library();
        db.with_conn(|c| {
            replace_detections(c, 1, "old", &[det("cat", near(0, 0))])?;
            save_scans(c, "new", &[(1, None), (2, Some(vec![det("dog", near(1, 0))])), (3, Some(vec![])), (4, None)])?;
            let count = |sql: &str| -> rusqlite::Result<i64> { c.query_row(sql, [], |r| r.get(0)) };
            assert_eq!(count("SELECT COUNT(*) FROM pet_detections WHERE asset_id = 1")?, 1, "a skip keeps the old find");
            assert_eq!(count("SELECT COUNT(*) FROM pet_detections WHERE asset_id = 2")?, 1);
            assert_eq!(count("SELECT COUNT(*) FROM pet_scan WHERE model = 'new'")?, 4, "every photo is marked");
            assert_eq!(unscanned_count(c, "new")?, 8);
            let mut has: Vec<i64> = with_detections(c, &[1, 2, 3, 4])?.into_iter().collect();
            has.sort();
            assert_eq!(has, vec![1, 2]);
            assert!(with_detections(c, &[])?.is_empty());
            // 5 looks like a beach, 6 like a dog, 7 has no embedding yet, 1 had a cat found before.
            let axis = |k: usize| {
                let mut v = vec![0i8; 8];
                v[k] = 127;
                v
            };
            let protos = crate::embedding::Prototypes { model: "m".into(), items: vec![("tag:dog".into(), axis(0)), ("tag:cat".into(), axis(1)), ("tag:beach".into(), axis(2))] };
            for (id, v) in [(5, axis(2)), (6, axis(0)), (1, axis(2))] {
                crate::embedding::store(c, id, "m", &v, None)?;
            }
            let (mut look, skip) = split_for_detector(c, &[1, 5, 6, 7], &protos)?;
            look.sort();
            assert_eq!((look, skip), (vec![1, 6, 7], vec![5]));
            Ok(())
        })
        .unwrap();
    }

    fn det(species: &str, v: Vector) -> NewPetDetection {
        NewPetDetection { species: species.into(), x: 0.2, y: 0.2, w: 0.4, h: 0.4, score: 0.9, vec: v, quality: 0.9, rejected: None }
    }

    const T: Thresholds = Thresholds { join: 0.8, margin: 0.05, suggest: 0.7, cluster: 0.8, place_quality: 0.0 };

    #[test]
    fn photos_of_one_animal_become_one_pet_and_a_cat_never_joins_a_dog() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=4 {
                replace_detections(c, a, "m", &[det("dog", near(0, a as usize))])?;
            }
            // A cat that looks, to the embedder, exactly like the dog.
            for a in 5..=7 {
                replace_detections(c, a, "m", &[det("cat", near(0, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let pets = list_pets(c)?;
            assert_eq!(pets.len(), 2);
            let species: Vec<&str> = pets.iter().map(|p| p.species.as_str()).collect();
            assert!(species.contains(&"cat") && species.contains(&"dog"));
            assert_eq!(pets.iter().find(|p| p.species == "dog").unwrap().photo_count, 4);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_named_pet_is_searchable_and_selects_its_photos() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=3 {
                replace_detections(c, a, "m", &[det("dog", near(1, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let milo = list_pets(c)?[0].id;
            name_pet(c, milo, "Milo")?;
            let q = crate::query::AssetQuery { text: Some("milo".into()), ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(q), ..Default::default() })?, 3);
            let by_id = crate::query::AssetQuery { pet_ids: vec![milo], ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(by_id), ..Default::default() })?, 3);
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { pet_id: Some(milo), ..Default::default() })?, 3);
            // The search bar knows him, and a pet chip AND-s with a person chip.
            assert_eq!(crate::query_parse::Vocabulary::load(c)?.pets, vec![(milo, "Milo".to_string())]);
            let with_someone = crate::query::AssetQuery { pet_ids: vec![milo], person_ids: vec![999], ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(with_someone), ..Default::default() })?, 0);
            // Removed: out of search, still grouping its photos.
            remove_pet(c, milo)?;
            let q = crate::query::AssetQuery { text: Some("milo".into()), ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(q), ..Default::default() })?, 0);
            assert!(list_pets(c)?.is_empty());
            assert_eq!(removed_pets(c)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_split_separates_look_alikes_for_good_and_undo_joins_them_again() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=6 {
                replace_detections(c, a, "m", &[det("cat", near(2, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let both = list_pets(c)?[0].id;
            let (other, undo) = split(c, both, &[4, 5, 6], 2)?;
            recluster(c, &T, 3)?;
            let pets = list_pets(c)?;
            assert_eq!(pets.len(), 2, "grouping does not put them back together");
            assert_eq!(pets.iter().find(|p| p.id == other).unwrap().photo_count, 3);
            restore_places(c, &undo)?;
            assert_eq!(list_pets(c)?.len(), 1, "the split's pet goes with its last photo");
            assert_eq!(list_pets(c)?[0].photo_count, 6);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_rescan_keeps_names_and_what_the_user_decided() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=3 {
                replace_detections(c, a, "m", &[det("dog", near(3, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let rex = list_pets(c)?[0].id;
            name_pet(c, rex, "Rex")?;
            // A new model finds the same dogs, a little shifted.
            for a in 1..=3 {
                let mut d = det("dog", near(3, a as usize + 1));
                d.x += 0.02;
                replace_detections(c, a, "m2", &[d])?;
            }
            let pets = list_pets(c)?;
            assert_eq!((pets.len(), pets[0].name.as_deref(), pets[0].photo_count), (1, Some("Rex"), 3));
            assert_eq!(unscanned_count(c, "m2")?, 9, "the other nine photos wait for the new model");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn merging_and_not_a_pet_undo_exactly() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=3 {
                replace_detections(c, a, "m", &[det("dog", near(4, a as usize))])?;
            }
            for a in 4..=6 {
                replace_detections(c, a, "m", &[det("dog", near(5, a as usize))])?;
            }
            for a in 7..=9 {
                replace_detections(c, a, "m", &[det("cat", near(6, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let dogs: Vec<i64> = list_pets(c)?.into_iter().filter(|p| p.species == "dog").map(|p| p.id).collect();
            let cat = list_pets(c)?.into_iter().find(|p| p.species == "cat").unwrap().id;
            assert!(matches!(merge_pets(c, cat, cat), Err(CoreError::Invalid(_))), "that is the same pet");
            name_pet(c, dogs[1], "Bo")?;
            let undo = merge_pets(c, dogs[0], dogs[1])?;
            let merged = list_pets(c)?.into_iter().find(|p| p.id == dogs[0]).unwrap();
            assert_eq!((merged.name.as_deref(), merged.photo_count), (Some("Bo"), 6));
            undo_merge(c, &undo)?;
            assert_eq!(list_pets(c)?.len(), 3);
            assert_eq!(list_pets(c)?.into_iter().find(|p| p.id == dogs[0]).unwrap().name, None);

            let gone = not_a_pet(c, cat)?;
            recluster(c, &T, 2)?;
            assert_eq!(list_pets(c)?.len(), 2, "rejected detections are never grouped again");
            undo_not_a_pet(c, &gone)?;
            assert_eq!(list_pets(c)?.len(), 3);
            Ok(())
        })
        .unwrap();
    }

    /// Round 9A: the detector called Milo a dog in some photos. A dog merged into a cat
    /// becomes that cat, photos and all; Undo gives back both, each with its species.
    #[test]
    fn a_dog_merged_into_a_cat_becomes_the_cat_and_undo_restores_both() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=3 {
                replace_detections(c, a, "m", &[det("cat", near(6, a as usize))])?;
            }
            for a in 4..=6 {
                replace_detections(c, a, "m", &[det("dog", near(8, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let cat = list_pets(c)?.into_iter().find(|p| p.species == "cat").unwrap().id;
            let dog = list_pets(c)?.into_iter().find(|p| p.species == "dog").unwrap().id;
            name_pet(c, dog, "Rex")?;
            let undo = merge_pets(c, cat, dog)?;
            assert_eq!(undo.from_species, "dog");
            let pets = list_pets(c)?;
            assert_eq!(pets.len(), 1);
            assert_eq!((pets[0].id, pets[0].species.as_str(), pets[0].photo_count, pets[0].name.as_deref()), (cat, "cat", 6, Some("Rex")));
            let species: Vec<String> = c.prepare("SELECT DISTINCT species FROM pet_detections WHERE pet_id = ?1")?.query_map([cat], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            assert_eq!(species, vec!["cat".to_string()], "its photos are of a cat now");
            recluster(c, &T, 2)?;
            assert_eq!(list_pets(c)?.len(), 1, "regrouping leaves the merge alone");

            // A split after the merge keeps the merged species.
            let (split_off, places) = split(c, cat, &[5], 3)?;
            assert_eq!(pet_row(c, split_off)?.species, "cat");
            restore_places(c, &places)?;

            undo_merge(c, &undo)?;
            let mut back: Vec<(i64, String, i64)> = list_pets(c)?.into_iter().map(|p| (p.id, p.species, p.photo_count)).collect();
            back.sort();
            let mut want = vec![(cat, "cat".to_string(), 3), (dog, "dog".to_string(), 3)];
            want.sort();
            assert_eq!(back, want);
            let dog_species: Vec<String> = c.prepare("SELECT DISTINCT species FROM pet_detections WHERE pet_id = ?1")?.query_map([dog], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            assert_eq!(dog_species, vec!["dog".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_photo_shows_its_pets_and_tagging_by_hand_places_one() {
        let db = library();
        db.with_conn(|c| {
            for a in 1..=3 {
                replace_detections(c, a, "m", &[det("dog", near(7, a as usize))])?;
            }
            recluster(c, &T, 1)?;
            let pet = list_pets(c)?[0].id;
            // A dog the grouping cannot place: far from anyone.
            replace_detections(c, 10, "m", &[det("dog", near(9, 0))])?;
            recluster(c, &T, 2)?;
            let here = asset_pets(c, 10)?;
            assert_eq!((here.len(), here[0].pet_id), (1, None));
            tag_detection(c, here[0].id, pet)?;
            assert_eq!(asset_pets(c, 10)?[0].pet_id, Some(pet));
            assert_eq!(list_pets(c)?[0].photo_count, 4);
            Ok(())
        })
        .unwrap();
    }

    /// Three dogs grouped as one, plus the pet's id.
    fn milo(c: &Connection) -> Result<i64> {
        for a in 1..=3 {
            replace_detections(c, a, "m", &[det("dog", near(3, a as usize))])?;
        }
        recluster(c, &T, 1)?;
        let pet = list_pets(c)?[0].id;
        name_pet(c, pet, "Milo")?;
        Ok(pet)
    }

    #[test]
    fn not_name_on_one_box_leaves_it_unknown_refused_and_undo_puts_it_back() {
        let db = library();
        db.with_conn(|c| {
            let pet = milo(c)?;
            let box1 = asset_pets(c, 1)?[0].id;
            let undo = untag_detection(c, box1)?;
            assert_eq!(asset_pets(c, 1)?[0].pet_id, None, "an unknown pet now");
            assert_eq!(list_pets(c)?[0].photo_count, 2);
            recluster(c, &T, 2)?;
            assert_eq!(asset_pets(c, 1)?[0].pet_id, None, "Milo refused it: grouping never puts it back");
            restore_places(c, &[undo])?;
            assert_eq!(asset_pets(c, 1)?[0].pet_id, Some(pet));
            assert_eq!(list_pets(c)?[0].photo_count, 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn not_a_pet_on_one_box_rejects_only_that_box_and_undo_restores_it() {
        let db = library();
        db.with_conn(|c| {
            let pet = milo(c)?;
            let box2 = asset_pets(c, 2)?[0].id;
            let undo = reject_detection(c, box2)?;
            assert!(asset_pets(c, 2)?.is_empty(), "no pet in that photo any more");
            assert_eq!(list_pets(c)?[0].photo_count, 2, "Milo keeps the others");
            recluster(c, &T, 2)?;
            assert!(asset_pets(c, 2)?.is_empty(), "a rejected box is never grouped");
            undo_reject_detection(c, &undo)?;
            assert_eq!(asset_pets(c, 2)?[0].pet_id, Some(pet));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_new_pet_from_one_box_is_named_holds_it_and_undo_removes_it() {
        let db = library();
        db.with_conn(|c| {
            let milo = milo(c)?;
            let box3 = asset_pets(c, 3)?[0].id;
            let (rex, undo) = create_pet_with_detection(c, box3, "  Rex ", 5)?;
            let pets = list_pets(c)?;
            let r = pets.iter().find(|p| p.id == rex).unwrap();
            assert_eq!((r.name.as_deref(), r.species.as_str(), r.photo_count), (Some("Rex"), "dog", 1));
            assert_eq!(asset_pets(c, 3)?[0].pet_id, Some(rex));
            assert!(create_pet_with_detection(c, box3, " ", 5).is_err(), "a new pet needs a name");
            undo_create_pet(c, rex, &undo)?;
            assert!(list_pets(c)?.iter().all(|p| p.id != rex));
            assert_eq!(asset_pets(c, 3)?[0].pet_id, Some(milo));
            Ok(())
        })
        .unwrap();
    }
}
