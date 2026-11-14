//! People (Phase 7b): faces grouped into people you can name. `photos-ml` finds and
//! describes the faces; this decides who is who.
//!
//! Clustering works in two stages so that it scales and so that names are stable. A new face
//! first joins an existing person when it is close enough to that person's centroid — a
//! pass of faces × people. Only what is left over is clustered from scratch, by Chinese
//! Whispers over the graph of faces at least `same` apart (dlib's method: no cluster count,
//! and one stray edge cannot merge two people the way it would with connected components).
//! A person's faces are never re-clustered away, so a name, once given, stays with them.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use ts_rs::TS;

use crate::embedding::{cosine, Vector};
use crate::Result;

pub mod radar;

/// A group smaller than this is not yet a person — too easily a stranger in the background.
pub const MIN_PERSON_FACES: usize = 3;
/// How many neighbours each face keeps in the whisper graph.
const NEIGHBOURS: usize = 20;
const ITERATIONS: usize = 20;

/// Chinese Whispers over `faces` (id, vector): edges join faces at cosine ≥ `same` (each
/// face's nearest `NEIGHBOURS`); every face starts as its own label and, in id order, takes
/// the label its neighbours weigh most. Groups of fewer than `MIN_PERSON_FACES` are dropped.
/// Deterministic: the order is the ids', and ties go to the smaller label.
pub fn cluster(faces: &[(i64, Vector)], same: f32) -> Vec<Vec<i64>> {
    let mut sorted: Vec<&(i64, Vector)> = faces.iter().collect();
    sorted.sort_by_key(|f| f.0);
    let n = sorted.len();
    // Every pair is compared (7,800 open faces on Julian's library: 61 M cosines), each row
    // on its own, so the rows run in parallel and the result is the same.
    use rayon::prelude::*;
    let edges: Vec<Vec<(usize, f32)>> = (0..n)
        .into_par_iter()
        .map(|i| {
            let mut near: Vec<(usize, f32)> = (0..n)
                .filter(|j| *j != i)
                .map(|j| (j, cosine(&sorted[i].1, &sorted[j].1)))
                .filter(|(_, c)| *c >= same)
                .collect();
            near.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            near.truncate(NEIGHBOURS);
            near
        })
        .collect();
    let mut label: Vec<usize> = (0..n).collect();
    for _ in 0..ITERATIONS {
        let mut changed = false;
        for i in 0..n {
            let mut weight: BTreeMap<usize, f32> = BTreeMap::new();
            for (j, c) in &edges[i] {
                *weight.entry(label[*j]).or_default() += c;
            }
            if let Some((best, _)) = weight.iter().max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(a.0))) {
                if *best != label[i] {
                    label[i] = *best;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut groups: BTreeMap<usize, Vec<i64>> = BTreeMap::new();
    for (i, l) in label.iter().enumerate() {
        groups.entry(*l).or_default().push(sorted[i].0);
    }
    groups.into_values().filter(|g| g.len() >= MIN_PERSON_FACES).collect()
}

fn blob(v: &[i8]) -> Vec<u8> {
    v.iter().map(|x| *x as u8).collect()
}

fn unblob(b: &[u8]) -> Vector {
    b.iter().map(|x| *x as i8).collect()
}

/// A face as the detector reported it, placed in 0..1 of the displayed photo.
#[derive(Debug, Clone, PartialEq)]
pub struct NewFace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub vec: Vector,
    /// `face_filter::quality`: ranks exemplars and covers.
    pub quality: f32,
    /// `face_filter::judge`'s reason when this is not a person's face (Round 4B).
    pub rejected: Option<String>,
}

/// Stores an asset's faces and marks it scanned by `model`: `replace_faces_keeping_identity`.
pub fn store_faces(conn: &Connection, asset_id: i64, model: &str, faces: &[NewFace]) -> Result<()> {
    replace_faces_keeping_identity(conn, asset_id, model, faces)
}

/// A detection overlapping a stored face by at least this much is the same face found again.
const SAME_FACE_IOU: f32 = 0.5;

fn iou(a: [f32; 4], b: [f32; 4]) -> f32 {
    let ix = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let iy = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let inter = ix * iy;
    inter / (a[2] * a[3] + b[2] * b[3] - inter)
}

/// Replaces an asset's faces with a new scan's (Round 4B, Task 4) without losing what the
/// user decided. Each new detection is matched to a stored face by box overlap (IoU ≥ 0.5,
/// best pairs first); a match updates that row in place, so its id — and with it the
/// person, the pin, *Not a person*, rejections and covers — survives. A machine judgement
/// (a screen, a print) is taken afresh unless the user pinned or rejected the face. Stored
/// faces with no match go unless the user pinned or rejected them; the rest are new.
pub fn replace_faces_keeping_identity(conn: &Connection, asset_id: i64, model: &str, faces: &[NewFace]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    // People who may lose a face here: their covers are checked, and the photo re-indexed.
    let people: Vec<Option<i64>> = {
        let mut stmt = tx.prepare("SELECT DISTINCT person_id FROM faces WHERE asset_id = ?1 AND person_id IS NOT NULL")?;
        let rows = stmt.query_map([asset_id], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    // (id, box, kept, judged by the user). A face the user touched is kept even when the
    // scan does not find it again: placed on someone (pinned), refused by someone (Round 4I:
    // no longer pinned), Not a person, or restored. The user's judgement of what a face is
    // (Not a person, *This is a real face*) is never redone by the filter.
    let old: Vec<(i64, [f32; 4], bool, bool)> = {
        let mut stmt = tx.prepare(
            "SELECT id, x, y, w, h,
                    pinned OR rejected IS 'user' OR source IS 'restored' OR EXISTS (SELECT 1 FROM face_rejections r WHERE r.face_id = faces.id),
                    pinned OR rejected IS 'user' OR source IS 'restored'
             FROM faces WHERE asset_id = ?1",
        )?;
        let rows = stmt.query_map([asset_id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                [r.get::<_, f64>(1)? as f32, r.get::<_, f64>(2)? as f32, r.get::<_, f64>(3)? as f32, r.get::<_, f64>(4)? as f32],
                r.get::<_, i64>(5)? != 0,
                r.get::<_, i64>(6)? != 0,
            ))
        })?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
    for (i, f) in faces.iter().enumerate() {
        for (j, o) in old.iter().enumerate() {
            let v = iou([f.x, f.y, f.w, f.h], o.1);
            if v >= SAME_FACE_IOU {
                pairs.push((v, i, j));
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (mut new_of, mut old_used) = (vec![None; faces.len()], vec![false; old.len()]);
    for (_, i, j) in pairs {
        if new_of[i].is_none() && !old_used[j] {
            new_of[i] = Some(j);
            old_used[j] = true;
        }
    }
    for (j, (id, _, kept, _)) in old.iter().enumerate() {
        if !old_used[j] && !kept {
            tx.execute("DELETE FROM face_rejections WHERE face_id = ?1", [id])?;
            tx.execute("DELETE FROM faces WHERE id = ?1", [id])?;
        }
    }
    {
        let mut ins = tx.prepare(
            "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, quality, rejected) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        let mut upd = tx.prepare("UPDATE faces SET x = ?2, y = ?3, w = ?4, h = ?5, score = ?6, vec = ?7, model = ?8, quality = ?9 WHERE id = ?1")?;
        // A machine judgement is redone; a rejected face has no person.
        let mut judge = tx.prepare("UPDATE faces SET rejected = ?2, person_id = CASE WHEN ?2 IS NULL THEN person_id ELSE NULL END WHERE id = ?1")?;
        for (i, f) in faces.iter().enumerate() {
            match new_of[i] {
                Some(j) => {
                    let (id, _, _, judged) = old[j];
                    upd.execute(params![id, f.x as f64, f.y as f64, f.w as f64, f.h as f64, f.score as f64, blob(&f.vec), model, f.quality as f64])?;
                    if !judged {
                        judge.execute(params![id, f.rejected])?;
                    }
                }
                None => {
                    ins.execute(params![asset_id, f.x as f64, f.y as f64, f.w as f64, f.h as f64, f.score as f64, blob(&f.vec), model, f.quality as f64, f.rejected])?;
                }
            }
        }
    }
    tx.execute(
        "INSERT INTO face_scan (asset_id, model) VALUES (?1, ?2) ON CONFLICT(asset_id) DO UPDATE SET model = excluded.model",
        params![asset_id, model],
    )?;
    fix_covers(&tx, &people)?;
    if !people.is_empty() {
        crate::fts::index_ids(&tx, &[asset_id])?;
    }
    tx.commit()?;
    Ok(())
}

/// Records `asset_id` as scanned by `model` without touching its faces: for a photo the scan
/// could not read, which keeps what an earlier scan found.
pub fn mark_scanned(conn: &Connection, asset_id: i64, model: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO face_scan (asset_id, model) VALUES (?1, ?2) ON CONFLICT(asset_id) DO UPDATE SET model = excluded.model",
        params![asset_id, model],
    )?;
    Ok(())
}

/// Photos not yet scanned by `model` (a second look, `second_chance_key`, counts as
/// scanned): `(asset_id, content_hash)`.
pub fn unscanned(conn: &Connection, model: &str, limit: usize) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id, a.content_hash FROM assets a LEFT JOIN face_scan s ON s.asset_id = a.id
         WHERE a.state = 0 AND a.media_type != 1 AND NOT {} AND (s.asset_id IS NULL OR (s.model != ?1 AND s.model != ?1 || 'i'))
         ORDER BY a.id DESC LIMIT ?2",
        crate::assets::MOVIE_HALF
    ))?;
    let rows = stmt.query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// How many photos the first pass has left (the activity ring's "… left").
pub fn unscanned_count(conn: &Connection, model: &str) -> Result<usize> {
    let n: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM assets a LEFT JOIN face_scan s ON s.asset_id = a.id
             WHERE a.state = 0 AND a.media_type != 1 AND NOT {} AND (s.asset_id IS NULL OR (s.model != ?1 AND s.model != ?1 || 'i'))",
            crate::assets::MOVIE_HALF
        ),
        [model],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Round 4I: scene tags (CLIP, `embedding::scene_tags`) that say a photo has people in it.
/// Such a photo with no face found is looked at once more, larger and in tiles.
pub const PEOPLE_SCENES: [&str; 7] = ["portrait", "selfie", "group of people", "baby", "child", "party", "wedding"];

/// What `face_scan.model` records once a photo's second look is done: the first pass's key
/// and an `i`, which the first pass counts as its own.
pub fn second_chance_key(model: &str) -> String {
    format!("{model}i")
}

/// Stores what a photo's second look found, as `replace_faces_keeping_identity` does, and
/// marks the new faces `source = 'second'` (`Thresholds::place_quality`).
pub fn store_second_look(conn: &Connection, asset_id: i64, model: &str, faces: &[NewFace]) -> Result<()> {
    replace_faces_keeping_identity(conn, asset_id, model, faces)?;
    conn.execute("UPDATE faces SET source = 'second' WHERE asset_id = ?1 AND model = ?2 AND source = 'scan'", params![asset_id, model])?;
    Ok(())
}

/// Photos the first pass (`model`) found no face in whose scene reads as people
/// (`PEOPLE_SCENES`), not yet given their second look. Newest first.
pub fn second_chance(conn: &Connection, model: &str, limit: usize) -> Result<Vec<i64>> {
    let scenes = PEOPLE_SCENES.iter().map(|t| format!("' ' || a.scene_tags || ' ' LIKE '% {t} %'")).collect::<Vec<_>>().join(" OR ");
    let mut stmt = conn.prepare(&format!(
        "SELECT a.id FROM assets a JOIN face_scan s ON s.asset_id = a.id
         WHERE s.model = ?1 AND a.state = 0 AND a.media_type != 1 AND ({scenes})
           AND NOT EXISTS (SELECT 1 FROM faces f WHERE f.asset_id = a.id)
         ORDER BY a.id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![model, limit as i64], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// The grouping thresholds (SFace cosines). Round 4B measured them against Julian's
/// corrections; Round 4I compares a face with a person's **consensus** (the mean of its best
/// `TOP_K` matches among the person's faces) rather than their single closest exemplar, so
/// the join can sit lower without letting a lookalike of one odd face in. Between `suggest`
/// and a join a face is asked about, or placed when its moment already holds the person.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    pub join: f32,
    pub margin: f32,
    pub suggest: f32,
    /// New people form from what is left over at this similarity.
    pub cluster: f32,
    /// A face the second look found (`source = 'second'`) of lower quality than this
    /// (`face_filter::quality`: tiny, ~50 px or less at 2560, smaller than the first pass ever
    /// stores) is never placed on anyone nor made into a new person without the user: it can
    /// only be suggested or tagged by hand.
    pub place_quality: f32,
}

impl Thresholds {
    /// Round 4I on Julian's copy (`grouping_quality_on_a_real_library`, half of every named
    /// person taken out and his removals lifted): precision 98.3 % (4B's rule: 97.4 %) and
    /// 56 % of the faces he placed by hand found again (4B: 50 %). A join of 0.42 found 64 %
    /// but at 95.9 % precision; the wider margin is what took precision over 98 %.
    ///
    /// `place_quality`: Round 4I's second look found class photos of 30–40 faces of ~40 px
    /// and put 366 of them on 31 named people (up to 29 in one photo). Applied to every face
    /// it cost 7 points of recall for 0.4 of precision, so it holds for the second look only.
    pub const MEASURED: Thresholds = Thresholds { join: 0.45, margin: 0.07, suggest: 0.363, cluster: 0.45, place_quality: 0.3 };

    /// One similarity for everything, no margin: the 7b rule (for tests).
    pub fn uniform(same: f32) -> Self {
        Self { join: same, margin: 0.0, suggest: same, cluster: same, place_quality: 0.0 }
    }

    /// A group of left-over faces whose mean likeness to their best existing person is below
    /// this is a new person; above it, it is asked about as that person (a second Anna is
    /// worse than a question about Anna). Halfway between a doubt and a join.
    fn new_person_ceiling(&self) -> f32 {
        (self.suggest + self.join) / 2.0
    }
}

/// How many of a person's best faces stand for them (Round 4I: 24, was 12).
pub const EXEMPLARS: usize = 24;
/// Faces this alike are one exemplar, not two.
const EXEMPLAR_DISTINCT: f32 = 0.8;
/// An exemplar has at least this share of the person's best quality: a blurry face does not
/// stand for them. (Faces the user placed count anyway; the consensus keeps one odd face
/// from deciding.)
const EXEMPLAR_QUALITY: f32 = 0.6;
/// A face's likeness to a person is the mean of its `TOP_K` best matches among the person's
/// faces: several of them must agree, so one lookalike that slipped in cannot pull strangers.
const TOP_K: usize = 3;
/// ...and at least this many of those faces must be doubtful-close (≥ `suggest`) on their own.
const VOTES: usize = 2;
/// A person with fewer than `TOP_K` faces to compare with is trusted a little less.
const SMALL_PENALTY: f32 = 0.03;
/// A doubtful face this close to a join is placed without asking when its moment (the
/// memories' 12-hour session) already holds `AUTO_MOMENT_FACES` of the person's faces:
/// the photo next to two of Anna at the same party is Anna.
const AUTO_SLACK: f32 = 0.03;
const AUTO_MOMENT_FACES: usize = 2;
const MOMENT_GAP_S: i64 = 12 * 3600;
/// A face that joins this far over `join` counts among the person's faces in the next pass.
const EVIDENCE_LEAD: f32 = 0.05;
/// Passes stop when nothing moves, or after this many.
const MAX_PASSES: usize = 5;
/// A group of left-over faces this much claimed by known people (a face with one of theirs at
/// a join) is theirs, not a new person; below it, the new person is made of the rest. On
/// Julian's copy, second copies of named people were 50–90 % claimed, new people 0–10 %.
const CLAIMED_SHARE: f32 = 1.0 / 3.0;
/// A person whose every face is further than this from a face is not worth remembering for it.
const LIKENESS_FLOOR: f32 = 0.2;

/// A stored vector with its length, so a cosine is one integer dot product.
struct Ev {
    v: Vector,
    norm: f32,
}

impl Ev {
    fn new(v: Vector) -> Self {
        let norm = (dot(&v, &v) as f32).sqrt();
        Self { v, norm }
    }

    fn cos(&self, other: &Ev) -> f32 {
        if self.norm == 0.0 || other.norm == 0.0 {
            return 0.0;
        }
        dot(&self.v, &other.v) as f32 / (self.norm * other.norm)
    }
}

fn dot(a: &[i8], b: &[i8]) -> i32 {
    a.iter().zip(b).map(|(x, y)| *x as i32 * *y as i32).sum()
}

/// A person as grouping sees them: the faces that stand for them.
struct Known {
    id: i64,
    evidence: Vec<Ev>,
    /// Faces the user said are not this person (Round 10): a face more like one of them than
    /// like the person is not theirs either.
    against: Vec<Ev>,
    /// Whether each of `evidence` was placed by hand.
    by_hand: Vec<bool>,
    /// The `evidence` a veto weighs the refused faces against (all when `None`): not a face
    /// Lumen placed that is more like a refused face than like any the user placed, or the
    /// look-alikes Lumen put on someone would vouch for each other.
    vouch: Option<Vec<usize>>,
}

type Refused = std::collections::HashMap<i64, std::collections::HashSet<i64>>;

/// Every person's evidence: their best faces (distinct, of good quality, best first) plus
/// every face the user placed on them. A face with no quality (a box the user drew with no
/// face the detector could find) never stands for anyone.
fn known_people(conn: &Connection) -> Result<Vec<Known>> {
    Ok(known_from(evidence_rows(conn)?))
}

/// The rows `known_people` is built from, read with nothing else done while the connection is
/// held (Round 7).
fn evidence_rows(conn: &Connection) -> Result<Vec<(i64, Vector, f32, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT person_id, vec, COALESCE(quality, score), pinned FROM faces
         WHERE person_id IS NOT NULL AND rejected IS NULL AND COALESCE(quality, score) > 0
         ORDER BY person_id, COALESCE(quality, score) DESC, id",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, unblob(&r.get::<_, Vec<u8>>(1)?), r.get::<_, f64>(2)? as f32, r.get::<_, i64>(3)? != 0)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Every group's evidence from its members: (group, vector, quality, placed by hand), ordered by
/// group, then best quality first. Members with no quality must already be left out.
fn known_from(rows: Vec<(i64, Vector, f32, bool)>) -> Vec<Known> {
    let mut by: BTreeMap<i64, Vec<(Ev, f32, bool)>> = BTreeMap::new();
    for (p, v, q, pinned) in rows {
        by.entry(p).or_default().push((Ev::new(v), q, pinned));
    }
    by
        .into_iter()
        .map(|(id, faces)| {
            let best = faces.first().map(|f| f.1).unwrap_or(0.0);
            let mut evidence: Vec<Ev> = Vec::new();
            let mut by_hand: Vec<bool> = Vec::new();
            let mut placed: Vec<Ev> = Vec::new();
            let mut spare: Vec<Ev> = Vec::new();
            for (v, q, pinned) in faces {
                let exemplar_room = evidence.len() < EXEMPLARS && q >= best * EXEMPLAR_QUALITY;
                if exemplar_room && evidence.iter().all(|e| e.cos(&v) < EXEMPLAR_DISTINCT) {
                    evidence.push(v);
                    by_hand.push(pinned);
                } else if pinned {
                    placed.push(v);
                } else if spare.len() < TOP_K {
                    spare.push(v);
                }
            }
            // A person photographed in one burst has near-identical faces: they still stand
            // for them, or a consensus of `TOP_K` could never form.
            let short = TOP_K.saturating_sub(evidence.len() + placed.len());
            let spare: Vec<Ev> = spare.into_iter().take(short).collect();
            by_hand.extend(std::iter::repeat_n(false, spare.len()));
            by_hand.extend(std::iter::repeat_n(true, placed.len()));
            evidence.extend(spare);
            evidence.extend(placed);
            Known { id, evidence, against: Vec::new(), by_hand, vouch: None }
        })
        .collect()
}

/// How alike a face is to one person: its `TOP_K` best cosines (best first), how many of the
/// person's faces it was compared with, and how many of those were at least `suggest`.
#[derive(Clone, Copy)]
struct Likeness {
    top: [f32; TOP_K],
    seen: usize,
    votes: usize,
}

impl Likeness {
    fn new() -> Self {
        Self { top: [f32::MIN; TOP_K], seen: 0, votes: 0 }
    }

    fn add(&mut self, c: f32, suggest: f32) {
        self.seen += 1;
        if c >= suggest {
            self.votes += 1;
        }
        if c > self.top[TOP_K - 1] {
            self.top[TOP_K - 1] = c;
            self.top.sort_by(|a, b| b.total_cmp(a));
        }
    }

    fn score(&self) -> f32 {
        let n = self.seen.min(TOP_K);
        if n == 0 {
            return f32::MIN;
        }
        let mean = self.top[..n].iter().sum::<f32>() / n as f32;
        if self.seen < TOP_K {
            mean - SMALL_PENALTY
        } else {
            mean
        }
    }
}

/// A face's likeness to every person worth remembering (index into `known`).
fn likeness_to_all(f: &Ev, known: &[Known], suggest: f32) -> Vec<(usize, Likeness)> {
    let mut out = Vec::new();
    for (k, person) in known.iter().enumerate() {
        let mut l = Likeness::new();
        for e in &person.evidence {
            l.add(f.cos(e), suggest);
        }
        if l.top[0] >= LIKENESS_FLOOR {
            out.push((k, l));
        }
    }
    out
}

/// The people a face may be (not refused, not already in its photo), best first:
/// (index into `known`, consensus score, votes).
fn candidates(likes: &[(usize, Likeness)], known: &[Known], refused: &std::collections::HashSet<i64>, blocked: &std::collections::HashSet<i64>) -> Vec<(usize, f32, usize)> {
    let mut out: Vec<(usize, f32, usize)> = likes
        .iter()
        .filter(|(k, _)| !refused.contains(&known[*k].id) && !blocked.contains(&known[*k].id))
        .map(|(k, l)| (*k, l.score(), l.votes))
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(known[a.0].id.cmp(&known[b.0].id)));
    out
}

/// Whether the best candidate clears a join by `slack` less than `join`, with its margin and
/// votes: `Some(index into known, score)`.
fn clears(c: &[(usize, f32, usize)], t: &Thresholds, slack: f32) -> Option<(usize, f32)> {
    let &(best, s, votes) = c.first()?;
    let second = c.get(1).map(|x| x.1).unwrap_or(f32::MIN);
    (s >= t.join - slack && s - second >= t.margin && votes >= VOTES).then_some((best, s))
}

/// How much closer a face must be to one refused for a person than to the closest of the
/// person's own faces to be vetoed for them. Round 10 on Julian's copy (`veto_on_a_copy`,
/// his hand-placed faces as the truth): 2 of 306 faces he placed vetoed for their own person
/// (0.65 %), 10 % of near-misses on someone else vetoed. Against the person's consensus instead
/// of their closest face, 0.07 vetoed 7.5 % of his.
const VETO_MARGIN: f32 = 0.1;

/// The user's "not them" as evidence (Round 10): `f` is closer to a face refused for this
/// person than to any of their own faces, by `VETO_MARGIN`, and at least doubtful-close to
/// it. Refusing one face of a look-alike used to leave every other photo of them on the
/// person. A tie stays with the person, as before.
fn vetoed(f: &Ev, person: &Known, t: &Thresholds) -> bool {
    if person.against.is_empty() {
        return false;
    }
    let refused = person.against.iter().filter(|a| a.v != f.v).map(|a| f.cos(a)).fold(f32::MIN, f32::max);
    if refused < t.suggest {
        return false;
    }
    let own = |i: &usize| person.evidence.get(*i).filter(|e| e.v != f.v).map(|e| f.cos(e));
    let closest = match &person.vouch {
        Some(vouch) => vouch.iter().filter_map(own).fold(f32::MIN, f32::max),
        None => (0..person.evidence.len()).filter_map(|i| own(&i)).fold(f32::MIN, f32::max),
    };
    refused - closest >= VETO_MARGIN
}

/// [`clears`] with the refusals as evidence: a winner `f` is vetoed for is out, and the rest are
/// judged again. Only winners are checked, so the cost is a few comparisons per face.
fn clears_for(f: &Ev, mut c: Vec<(usize, f32, usize)>, known: &[Known], t: &Thresholds, slack: f32) -> Option<(usize, f32)> {
    loop {
        let (k, s) = clears(&c, t, slack)?;
        if !vetoed(f, &known[k], t) {
            return Some((k, s));
        }
        c.retain(|x| x.0 != k);
    }
}

/// Each group's refused members' vectors, from rows already read: (group, vector).
fn against_of(members: &[MemberRow], refusals: &[(i64, i64)]) -> Vec<(i64, Vector)> {
    let by_id: std::collections::HashMap<i64, &Vector> = members.iter().map(|m| (m.0, &m.4)).collect();
    refusals.iter().filter_map(|(face, group)| by_id.get(face).map(|v| (*group, (*v).clone()))).collect()
}

/// The refused faces' vectors from the table: (person, vector), faces judged not a face left out.
fn against_rows(conn: &Connection) -> Result<Vec<(i64, Vector)>> {
    let mut stmt = conn.prepare("SELECT r.person_id, f.vec FROM face_rejections r JOIN faces f ON f.id = r.face_id WHERE f.rejected IS NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, unblob(&r.get::<_, Vec<u8>>(1)?))))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Adds each person's refused faces to what grouping knows of them, and which of their faces
/// may vouch for them against those (`Known::vouch`).
fn with_against(mut known: Vec<Known>, rows: Vec<(i64, Vector)>) -> Vec<Known> {
    let index: std::collections::HashMap<i64, usize> = known.iter().enumerate().map(|(i, k)| (k.id, i)).collect();
    for (p, v) in rows {
        if let Some(&i) = index.get(&p) {
            known[i].against.push(Ev::new(v));
        }
    }
    for k in known.iter_mut().filter(|k| !k.against.is_empty() && k.by_hand.iter().any(|h| *h)) {
        let closest = |e: &Ev, faces: &mut dyn Iterator<Item = &Ev>| faces.filter(|a| a.v != e.v).map(|a| e.cos(a)).fold(f32::MIN, f32::max);
        let vouch = (0..k.evidence.len())
            .filter(|&i| {
                if k.by_hand[i] {
                    return true;
                }
                let e = &k.evidence[i];
                let hand = closest(e, &mut k.evidence.iter().zip(&k.by_hand).filter(|(_, h)| **h).map(|(e, _)| e));
                closest(e, &mut k.against.iter()) - hand < VETO_MARGIN
            })
            .collect();
        k.vouch = Some(vouch);
    }
    known
}

/// Faces the user said are not these people: face → people.
fn refusals(conn: &Connection) -> Result<Refused> {
    Ok(refusals_from(refusal_rows(conn)?))
}

fn refusal_rows(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare("SELECT face_id, person_id FROM face_rejections")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn refusals_from(rows: Vec<(i64, i64)>) -> Refused {
    let mut out = Refused::default();
    for (f, p) in rows {
        out.entry(f).or_default().insert(p);
    }
    out
}

/// Each photo's moment: runs of photos with faces, split where more than `MOMENT_GAP_S`
/// passes between two (the memories' sessions). Photos without a date have none.
fn dated_rows(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.captured_at FROM assets a
         WHERE a.captured_at IS NOT NULL AND EXISTS (SELECT 1 FROM faces f WHERE f.asset_id = a.id)
         ORDER BY a.captured_at, a.id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Each photo's moment from (photo, captured at) in time order.
fn moments_from(rows: Vec<(i64, i64)>) -> std::collections::HashMap<i64, usize> {
    let mut out = std::collections::HashMap::new();
    let (mut moment, mut last) = (0usize, None::<i64>);
    for (asset, at) in rows {
        if last.is_some_and(|l| at - l > MOMENT_GAP_S) {
            moment += 1;
        }
        last = Some(at);
        out.insert(asset, moment);
    }
    out
}

/// Faces without a person find one, and what is left forms new people (Round 4I):
///
/// 1. Each unplaced face is compared with every person's consensus (`known_people`,
///    `Likeness`). It joins the best when that clears `join` with a `margin` over the
///    runner-up and `VOTES` of the person's faces agree; or, a little under a join, when its
///    moment already holds `AUTO_MOMENT_FACES` of the person.
/// 2. A person is in a photo once: faces are placed best first, and a person already in the
///    photo is off the list for the next face there, which may go to its runner-up.
/// 3. Faces that join confidently count among the person's faces, and the pass repeats until
///    nothing moves (`MAX_PASSES`).
/// 4. What is left, doubtful faces included, is clustered (Chinese Whispers at `cluster`). A
///    group that is not nearly someone already (`new_person_ceiling`) becomes a new unnamed
///    person, once per photo; the rest stay suggestions.
///
/// People removed from People take their lookalikes like anyone else, so they never come back
/// as a new person. Pinned and rejected faces never move. Returns how many faces found a
/// person. People left with no faces and no name are removed.
///
/// In three steps, so the library is never locked while it thinks (Round 5A: one write
/// transaction around the whole of it held every other write — a Develop save among them —
/// for 6–22 s on Julian's library): [`ReclusterInput::read`] on any connection,
/// [`ReclusterInput::plan`] with none, then [`ReclusterPlan::apply`] in one short transaction.
pub fn recluster(conn: &Connection, t: &Thresholds, now: i64) -> Result<usize> {
    ReclusterInput::read(conn)?.plan(t).apply(conn, now)
}

// Rejected detections (a screen, a statue, Not a person) are never grouped.
// (face, asset, person, pinned, vector, quality, from the second look)
pub type MemberRow = (i64, i64, Option<i64>, bool, Vector, f32, bool);

/// Everything grouping looks at, read in one go.
pub struct ReclusterInput {
    all: Vec<MemberRow>,
    known: Vec<Known>,
    refused: Refused,
    moment_of: std::collections::HashMap<i64, usize>,
}

/// What grouping decided: faces to place on people who exist, and new people.
#[derive(Debug, Default, PartialEq)]
pub struct ReclusterPlan {
    /// (face, person, photo)
    place: Vec<(i64, i64, i64)>,
    new_people: Vec<NewGroup>,
}

/// A new unnamed person: their faces (face, photo) and the face that covers them.
#[derive(Debug, PartialEq)]
struct NewGroup {
    cover: i64,
    faces: Vec<(i64, i64)>,
}

/// What grouping needs, as rows, for any kind of member (faces, pets).
#[derive(Debug, Default)]
pub struct GroupingRows {
    /// Every member not rejected: (id, photo, group, placed by hand, vector, quality, from a
    /// second look).
    pub members: Vec<MemberRow>,
    /// Each group's members with a quality over 0: (group, vector, quality, placed by hand),
    /// by group, best quality first.
    pub evidence: Vec<(i64, Vector, f32, bool)>,
    /// (member, group it must never join).
    pub refusals: Vec<(i64, i64)>,
    /// (photo, captured at) of the photos with members, in time order.
    pub dated: Vec<(i64, i64)>,
}

impl ReclusterPlan {
    /// (member, existing group, photo) to place.
    pub fn places(&self) -> &[(i64, i64, i64)] {
        &self.place
    }

    /// New groups: their cover member and (member, photo) list.
    pub fn new_groups(&self) -> impl Iterator<Item = (i64, &[(i64, i64)])> {
        self.new_people.iter().map(|g| (g.cover, g.faces.as_slice()))
    }
}

impl ReclusterInput {
    pub fn read(conn: &Connection) -> Result<Self> {
        Ok(Self::from_rows(GroupingRows::read_faces(conn)?))
    }
}

impl GroupingRows {
    /// The faces' rows, only read: building each person's evidence from them takes longer
    /// than reading them, so the app does it after handing the connection back (Round 7).
    pub fn read_faces(conn: &Connection) -> Result<Self> {
        let mut stmt = conn.prepare("SELECT id, asset_id, person_id, pinned, vec, COALESCE(quality, score), source = 'second' FROM faces WHERE rejected IS NULL")?;
        let all = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, i64>(3)? != 0,
                    unblob(&r.get::<_, Vec<u8>>(4)?),
                    r.get::<_, f64>(5)? as f32,
                    r.get::<_, bool>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self { evidence: evidence_of(&all), members: all, refusals: refusal_rows(conn)?, dated: dated_rows(conn)? })
    }
}

/// `evidence_rows` from faces already read: the placed ones with a quality, by person, best
/// first. Reading them again from the table cost twice the read of every face (Round 7).
fn evidence_of(members: &[MemberRow]) -> Vec<(i64, Vector, f32, bool)> {
    let mut rows: Vec<(i64, i64, Vector, f32, bool)> = members
        .iter()
        .filter_map(|(id, _, person, pinned, v, q, _)| person.filter(|_| *q > 0.0).map(|p| (p, *id, v.clone(), *q, *pinned)))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(b.3.total_cmp(&a.3)).then(a.1.cmp(&b.1)));
    rows.into_iter().map(|(p, _, v, q, pinned)| (p, v, q, pinned)).collect()
}

impl ReclusterInput {
    /// The same input from rows another kind of group reads from its own tables (pets).
    pub fn from_rows(rows: GroupingRows) -> Self {
        let against = against_of(&rows.members, &rows.refusals);
        Self { known: with_against(known_from(rows.evidence), against), all: rows.members, refused: refusals_from(rows.refusals), moment_of: moments_from(rows.dated) }
    }

    /// The decisions, with no connection held.
    pub fn plan(self, t: &Thresholds) -> ReclusterPlan {
        use std::collections::{HashMap, HashSet};
        let Self { all, mut known, refused, moment_of } = self;
        let none = HashSet::new();
        let mut plan = ReclusterPlan::default();

        // Who is already in each photo, and how many of each person's faces each moment holds.
        let mut in_photo: HashMap<i64, HashSet<i64>> = HashMap::new();
        let mut in_moment: HashMap<(i64, usize), usize> = HashMap::new();
        for (_, asset, person, ..) in &all {
            if let Some(p) = person {
                in_photo.entry(*asset).or_default().insert(*p);
                if let Some(m) = moment_of.get(asset) {
                    *in_moment.entry((*p, *m)).or_default() += 1;
                }
            }
        }

        // The faces to place (a face with a person stays; one the user placed stays too).
        struct Open {
            id: i64,
            asset: i64,
            ev: Ev,
            likes: Vec<(usize, Likeness)>,
            placed: Option<usize>,
            /// Under `place_quality`: only ever suggested.
            tiny: bool,
        }
        use rayon::prelude::*;
        let mut open: Vec<Open> = all
            .par_iter()
            .filter(|(_, _, person, pinned, ..)| person.is_none() && !pinned)
            .map(|(id, asset, _, _, v, q, second)| {
                let ev = Ev::new(v.clone());
                let likes = likeness_to_all(&ev, &known, t.suggest);
                Open { id: *id, asset: *asset, ev, likes, placed: None, tiny: *second && *q < t.place_quality }
            })
            .collect();
        let quality_of: HashMap<i64, f32> = all.iter().map(|(f, .., q, _)| (*f, *q)).collect();

        {
            let decide = |o: &Open, in_photo: &HashMap<i64, HashSet<i64>>, in_moment: &HashMap<(i64, usize), usize>, known: &[Known]| -> Option<(usize, f32)> {
                if o.tiny {
                    return None;
                }
                let blocked = in_photo.get(&o.asset).unwrap_or(&none);
                let c = candidates(&o.likes, known, refused.get(&o.id).unwrap_or(&none), blocked);
                if let Some(hit) = clears_for(&o.ev, c.clone(), known, t, 0.0) {
                    return Some(hit);
                }
                let (k, s) = clears_for(&o.ev, c, known, t, AUTO_SLACK)?;
                let m = moment_of.get(&o.asset)?;
                (in_moment.get(&(known[k].id, *m)).copied().unwrap_or(0) >= AUTO_MOMENT_FACES).then_some((k, s))
            };
            for _pass in 0..MAX_PASSES {
                let mut order: Vec<(f32, usize)> = open
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| o.placed.is_none())
                    .filter_map(|(i, o)| decide(o, &in_photo, &in_moment, &known).map(|(_, s)| (s, i)))
                    .collect();
                order.sort_by(|a, b| b.0.total_cmp(&a.0).then(open[a.1].id.cmp(&open[b.1].id)));
                let mut fresh: Vec<(usize, usize)> = Vec::new(); // (person index, open index)
                for (_, i) in order {
                    // Decided again: a face placed just before may have taken this photo's person.
                    let Some((k, s)) = decide(&open[i], &in_photo, &in_moment, &known) else { continue };
                    let person = known[k].id;
                    plan.place.push((open[i].id, person, open[i].asset));
                    open[i].placed = Some(k);
                    in_photo.entry(open[i].asset).or_default().insert(person);
                    if let Some(m) = moment_of.get(&open[i].asset) {
                        *in_moment.entry((person, *m)).or_default() += 1;
                    }
                    if s >= t.join + EVIDENCE_LEAD {
                        fresh.push((k, i));
                    }
                }
                if fresh.is_empty() {
                    break;
                }
                // The confident newcomers count for the next pass: each open face is compared
                // with them alone, not with everyone again.
                for (k, i) in &fresh {
                    let newcomer = Ev { v: open[*i].ev.v.clone(), norm: open[*i].ev.norm };
                    for o in open.iter_mut().filter(|o| o.placed.is_none()) {
                        let c = o.ev.cos(&newcomer);
                        match o.likes.iter_mut().find(|(kk, _)| *kk == *k) {
                            Some((_, l)) => l.add(c, t.suggest),
                            None if c >= LIKENESS_FLOOR => {
                                let mut l = Likeness::new();
                                for e in &known[*k].evidence {
                                    l.add(o.ev.cos(e), t.suggest);
                                }
                                l.add(c, t.suggest);
                                o.likes.push((*k, l));
                            }
                            None => {}
                        }
                    }
                    known[*k].evidence.push(newcomer);
                }
            }

            // What is left becomes new people, when it is not nearly someone already.
            let left: Vec<usize> = (0..open.len()).filter(|i| open[*i].placed.is_none() && !open[*i].tiny).collect();
            let pool: Vec<(i64, Vector)> = left.iter().map(|i| (open[*i].id, open[*i].ev.v.clone())).collect();
            let index_of: HashMap<i64, usize> = left.iter().map(|i| (open[*i].id, *i)).collect();
            for group in cluster(&pool, t.cluster) {
                let members: Vec<usize> = group.iter().map(|id| index_of[id]).collect();
                let nearest = |i: usize| {
                    let o = &open[i];
                    candidates(&o.likes, &known, refused.get(&o.id).unwrap_or(&none), &none).first().map(|c| c.1).unwrap_or(0.0)
                };
                let mean = members.iter().map(|i| nearest(*i)).sum::<f32>() / members.len() as f32;
                if mean >= t.new_person_ceiling() {
                    continue;
                }
                // New photos of someone known are doubtful as a group (their own look is not yet
                // among the person's faces) but mostly have one of the person's faces at a join or
                // closer; strangers seldom do. A face with such a pair is the person's to claim.
                let claimed = |i: usize| {
                    let refused_here = refused.get(&open[i].id).unwrap_or(&none);
                    open[i].likes.iter().any(|(k, l)| l.top[0] >= t.join && !refused_here.contains(&known[*k].id))
                };
                let (claimed, members): (Vec<usize>, Vec<usize>) = members.into_iter().partition(|i| claimed(*i));
                if claimed.len() as f32 >= (claimed.len() + members.len()) as f32 * CLAIMED_SHARE {
                    continue;
                }
                // Once per photo: the face most like the rest of the group stays.
                let fit = |i: usize| members.iter().filter(|j| **j != i).map(|j| open[i].ev.cos(&open[*j].ev)).sum::<f32>();
                let mut by_photo: HashMap<i64, usize> = HashMap::new();
                for &i in &members {
                    let keep = match by_photo.get(&open[i].asset) {
                        Some(&j) => fit(i) > fit(j),
                        None => true,
                    };
                    if keep {
                        by_photo.insert(open[i].asset, i);
                    }
                }
                let mut chosen: Vec<usize> = by_photo.into_values().collect();
                if chosen.len() < MIN_PERSON_FACES {
                    continue;
                }
                chosen.sort_by_key(|i| open[*i].id);
                let cover = chosen
                    .iter()
                    .map(|i| open[*i].id)
                    .max_by(|a, b| quality_of[a].total_cmp(&quality_of[b]).then(b.cmp(a)))
                    .unwrap_or(open[chosen[0]].id);
                plan.new_people.push(NewGroup { cover, faces: chosen.iter().map(|i| (open[*i].id, open[*i].asset)).collect() });
            }
        }
        plan
    }
}

impl ReclusterPlan {
    /// Writes the decisions in one short transaction. The library may have moved on since it
    /// was read: a face is placed only while it is still open (no person, not pinned, not
    /// rejected) and only on a person who still exists, so nothing the user did meanwhile is
    /// undone. Returns how many faces found a person.
    pub fn apply(&self, conn: &Connection, now: i64) -> Result<usize> {
        self.apply_counting(conn, now).map(|(assigned, _)| assigned)
    }

    /// `apply`, also returning how many people with no faces left were removed: when both are
    /// zero nothing the user can see changed, and the app need not say the library did.
    pub fn apply_counting(&self, conn: &Connection, now: i64) -> Result<(usize, usize)> {
        let tx = conn.unchecked_transaction()?;
        let mut assigned = 0;
        // Photos whose people changed: their names must reach the search index.
        let mut touched: Vec<i64> = Vec::new();
        {
            let mut set = tx.prepare(
                "UPDATE faces SET person_id = ?2
                 WHERE id = ?1 AND person_id IS NULL AND pinned = 0 AND rejected IS NULL
                   AND EXISTS (SELECT 1 FROM people WHERE id = ?2)",
            )?;
            for &(face, person, asset) in &self.place {
                if set.execute(params![face, person])? > 0 {
                    touched.push(asset);
                    assigned += 1;
                }
            }
            for group in &self.new_people {
                tx.execute("INSERT INTO people (created_at, cover_face_id) VALUES (?1, ?2)", params![now, group.cover])?;
                let person = tx.last_insert_rowid();
                for &(face, asset) in &group.faces {
                    if set.execute(params![face, person])? > 0 {
                        touched.push(asset);
                        assigned += 1;
                    }
                }
            }
        }
        let removed = tx.execute(
            "DELETE FROM people WHERE name IS NULL AND NOT EXISTS (SELECT 1 FROM faces f WHERE f.person_id = people.id)",
            [],
        )?;
        touched.sort_unstable();
        touched.dedup();
        crate::fts::index_ids(&tx, &touched)?;
        tx.commit()?;
        Ok((assigned, removed))
    }
}

/// Round 4I: faces pinned to nobody come back into play. Before Round 4B, taking a face
/// out of a person pinned it to nobody and recorded nothing else, so 611 of Julian's faces
/// were out of every person and every suggestion for good. Julian chose "block the
/// likeliest": a face with no rejection is refused by the person it looks most like (at
/// least `t.suggest`, or it could never have been theirs), which is almost always the one
/// it was taken from; then every such face is unpinned. Faces the user rejected (*Not a
/// person*) are left alone. Idempotent: after Round 4I nothing pins a face to nobody.
/// Returns how many faces were unpinned.
pub fn repair_orphans(conn: &Connection, t: &Thresholds, now: i64) -> Result<usize> {
    let orphans: Vec<(i64, Vector)> = {
        let mut stmt = conn.prepare("SELECT id, vec FROM faces WHERE person_id IS NULL AND pinned = 1 AND rejected IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, unblob(&r.get::<_, Vec<u8>>(1)?))))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    if orphans.is_empty() {
        return Ok(0);
    }
    let known = known_people(conn)?;
    let refused = refusals(conn)?;
    let tx = conn.unchecked_transaction()?;
    {
        let mut refuse = tx.prepare("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)")?;
        let mut unpin = tx.prepare("UPDATE faces SET pinned = 0 WHERE id = ?1")?;
        for (id, v) in &orphans {
            // A face refused by someone already says whom it was taken from.
            if !refused.contains_key(id) {
                let likes = likeness_to_all(&Ev::new(v.clone()), &known, t.suggest);
                // "Looks most like": the single closest face, as 4B's exemplars judged when
                // the face was taken out.
                let best = likes.iter().max_by(|a, b| a.1.top[0].total_cmp(&b.1.top[0]));
                if let Some((k, l)) = best {
                    if l.top[0] >= t.suggest {
                        refuse.execute(params![id, known[*k].id, now])?;
                    }
                }
            }
            unpin.execute([id])?;
        }
    }
    tx.commit()?;
    Ok(orphans.len())
}

/// A face that may be this person, asked about on their page ("Is this Anna?").
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Suggestion {
    #[ts(type = "number")]
    pub face_id: i64,
    #[ts(type = "number")]
    pub asset_id: i64,
    pub similarity: f32,
}

/// Faces with no person whose best match is `person` but not clearly enough to place them:
/// a consensus of at least `suggest`, below a join or without its margin or votes, or any
/// match for a second-look face too small to place (`place_quality`). Never a
/// face the user said is not this person. Best first.
pub fn suggestions(conn: &Connection, person: i64, t: &Thresholds, limit: usize) -> Result<Vec<Suggestion>> {
    let (evidence, refusals, open, against) = suggestion_rows(conn)?;
    Ok(suggest(&with_against(known_from(evidence), against), &refusals_from(refusals), open, person, t, limit))
}

/// `suggestions` for the app: the rows in one hold, the comparing with no connection (Round 7).
pub fn suggestions_on(db: &crate::Db, person: i64, t: &Thresholds, limit: usize) -> Result<Vec<Suggestion>> {
    let (evidence, refusals, open, against) = db.with_conn(suggestion_rows)?;
    Ok(suggest(&with_against(known_from(evidence), against), &refusals_from(refusals), open, person, t, limit))
}

type SuggestionRows = (Vec<(i64, Vector, f32, bool)>, Vec<(i64, i64)>, Vec<(i64, i64, Vector, f32)>, Vec<(i64, Vector)>);

/// The evidence, the refusals and the open faces (unplaced, unpinned, not rejected: id, photo,
/// vector, and the quality where a second look may only be suggested, else 1), from one read
/// of the faces.
fn suggestion_rows(conn: &Connection) -> Result<SuggestionRows> {
    let rows = GroupingRows::read_faces(conn)?;
    let against = against_of(&rows.members, &rows.refusals);
    let open = rows
        .members
        .into_iter()
        .filter(|(_, _, person, pinned, ..)| person.is_none() && !pinned)
        .map(|(id, asset, _, _, v, q, second)| (id, asset, v, if second { q } else { 1.0 }))
        .collect();
    Ok((rows.evidence, rows.refusals, open, against))
}

/// [`suggestions`] from rows another kind of group reads from its own tables (pets):
/// `evidence` as [`GroupingRows::evidence`], `refusals` as there, and `open` the unplaced,
/// unpinned, unrejected members as (id, photo, vector, quality where only a suggestion is allowed
/// below `place_quality`, else 1).
pub fn suggestions_from(evidence: Vec<(i64, Vector, f32, bool)>, refusals: Vec<(i64, i64)>, open: Vec<(i64, i64, Vector, f32)>, group: i64, t: &Thresholds, limit: usize) -> Vec<Suggestion> {
    suggest(&known_from(evidence), &refusals_from(refusals), open, group, t, limit)
}

fn suggest(known: &[Known], refused: &Refused, rows: Vec<(i64, i64, Vector, f32)>, person: i64, t: &Thresholds, limit: usize) -> Vec<Suggestion> {
    let none = std::collections::HashSet::new();
    let Some(mine) = known.iter().find(|k| k.id == person) else { return Vec::new() };
    use rayon::prelude::*;
    let mut out: Vec<Suggestion> = rows
        .into_par_iter()
        .filter_map(|(face_id, asset_id, v, quality)| {
            let refused_here = refused.get(&face_id).unwrap_or(&none);
            if refused_here.contains(&person) {
                return None;
            }
            let ev = Ev::new(v);
            // Only a face at least `suggest` like this person can be suggested for them, and
            // most are nowhere near: those are dropped before being compared with everyone
            // (Round 5A, real library: 1.9 s → 0.13 s, `people_screen_timing_on_a_real_library`).
            let mut own = Likeness::new();
            for e in &mine.evidence {
                own.add(ev.cos(e), t.suggest);
            }
            if own.top[0] < LIKENESS_FLOOR || own.score() < t.suggest {
                return None;
            }
            let likes = likeness_to_all(&ev, known, t.suggest);
            let c = candidates(&likes, known, refused_here, &none);
            let &(best, s, _) = c.first()?;
            // More like a face refused for them than like them: not asked about.
            if vetoed(&ev, &known[best], t) {
                return None;
            }
            // A tiny face is never placed, so even a clear match is only asked about.
            (known[best].id == person && s >= t.suggest && (quality < t.place_quality || clears(&c, t, 0.0).is_none())).then_some(Suggestion { face_id, asset_id, similarity: s })
        })
        .collect();
    out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity).then(a.face_id.cmp(&b.face_id)));
    out.truncate(limit);
    out
}

/// An answer to a suggestion, for Undo.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SuggestionUndo {
    #[ts(type = "number")]
    pub face: i64,
    #[ts(type = "number")]
    pub person: i64,
    pub yes: bool,
    /// Where the face was before a Yes.
    pub before: FacePlace,
}

/// Yes: the face is this person, placed and pinned. No: it is not, and is never offered to
/// or joined with them again.
pub fn answer_suggestion(conn: &Connection, face: i64, person: i64, yes: bool, now: i64) -> Result<SuggestionUndo> {
    let (asset, was, pinned): (i64, Option<i64>, bool) =
        conn.query_row("SELECT asset_id, person_id, pinned FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    if yes {
        let tx = conn.unchecked_transaction()?;
        tx.execute("UPDATE faces SET person_id = ?2, pinned = 1 WHERE id = ?1", params![face, person])?;
        crate::fts::index_ids(&tx, &[asset])?;
        tx.commit()?;
    } else {
        conn.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![face, person, now])?;
    }
    Ok(SuggestionUndo { face, person, yes, before: FacePlace { face, person: was, pinned } })
}

/// Undoes `answer_suggestion`.
pub fn undo_suggestion(conn: &Connection, undo: &SuggestionUndo) -> Result<()> {
    if undo.yes {
        restore_faces(conn, std::slice::from_ref(&undo.before))
    } else {
        conn.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", params![undo.face, undo.person])?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: Option<String>,
    #[ts(type = "number")]
    pub face_count: i64,
    #[ts(type = "number")]
    pub photo_count: i64,
    #[ts(type = "number | null")]
    pub cover_face_id: Option<i64>,
}

fn people_with_status(conn: &Connection, status: &str) -> Result<Vec<PersonSummary>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, COUNT(f.id), COUNT(DISTINCT f.asset_id), p.cover_face_id
         FROM people p JOIN faces f ON f.person_id = p.id JOIN assets a ON a.id = f.asset_id AND a.state = 0 AND a.concealed = 0
         WHERE p.status = ?1
         GROUP BY p.id
         ORDER BY p.name IS NULL, p.name COLLATE NOCASE, COUNT(f.id) DESC, p.id",
    )?;
    let rows = stmt.query_map([status], |r| {
        Ok(PersonSummary { id: r.get(0)?, name: r.get(1)?, face_count: r.get(2)?, photo_count: r.get(3)?, cover_face_id: r.get(4)? })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Everyone on the People page, named first (by name), then by how often they appear.
pub fn list_people(conn: &Connection) -> Result<Vec<PersonSummary>> {
    people_with_status(conn, "listed")
}

/// The people removed from People (Round 4B), for Settings → People's *Restore*.
pub fn removed_people(conn: &Connection) -> Result<Vec<PersonSummary>> {
    people_with_status(conn, "removed")
}

fn assets_of(conn: &Connection, person: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT DISTINCT asset_id FROM faces WHERE person_id = ?1")?;
    let rows = stmt.query_map([person], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// Names a person (empty clears the name) and re-indexes their photos so the name finds them.
pub fn name_person(conn: &Connection, person: i64, name: &str) -> Result<()> {
    let name = name.trim();
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE people SET name = ?2 WHERE id = ?1", params![person, (!name.is_empty()).then_some(name)])?;
    crate::fts::index_ids(&tx, &assets_of(&tx, person)?)?;
    tx.commit()?;
    Ok(())
}

/// What a merge moved, so Undo can put it back.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MergeUndo {
    #[ts(type = "number[]")]
    pub faces: Vec<i64>,
    pub name: Option<String>,
    /// Whether `into` took the name from the merged person (and so gives it back).
    pub took_name: bool,
    #[ts(type = "number")]
    pub into: i64,
    /// Faces said not to be the merged person: they are not `into` either (Round 4B).
    #[serde(default)]
    #[ts(type = "number[]")]
    pub rejections: Vec<i64>,
    /// Of those, the ones `into` did not already refuse (Undo takes them off `into`).
    #[serde(default)]
    #[ts(type = "number[]")]
    pub moved_rejections: Vec<i64>,
    /// Faces of the merged person that `into` refused: the merge says they are `into`.
    #[serde(default)]
    #[ts(type = "number[]")]
    pub cleared: Vec<i64>,
}

/// `from`'s faces become `into`'s; `into` keeps its name, or takes `from`'s if it had none.
pub fn merge_people(conn: &Connection, into: i64, from: i64) -> Result<MergeUndo> {
    let tx = conn.unchecked_transaction()?;
    let name_of = |id: i64| -> Result<Option<String>> { Ok(tx.query_row("SELECT name FROM people WHERE id = ?1", [id], |r| r.get(0)).optional()?.flatten()) };
    let (from_name, into_name) = (name_of(from)?, name_of(into)?);
    let faces: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT id FROM faces WHERE person_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([from], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let ids = |sql: &str| -> Result<Vec<i64>> {
        let mut stmt = tx.prepare(sql)?;
        let rows = stmt.query_map(params![from, into], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    };
    let rejections = ids("SELECT face_id FROM face_rejections WHERE person_id = ?1 AND ?2 IS NOT NULL ORDER BY face_id")?;
    let moved_rejections = ids(
        "SELECT face_id FROM face_rejections r WHERE person_id = ?1
         AND NOT EXISTS (SELECT 1 FROM face_rejections o WHERE o.face_id = r.face_id AND o.person_id = ?2) ORDER BY face_id",
    )?;
    let cleared = ids("SELECT face_id FROM face_rejections WHERE person_id = ?2 AND face_id IN (SELECT id FROM faces WHERE person_id = ?1) ORDER BY face_id")?;
    let undo = MergeUndo { faces, took_name: into_name.is_none() && from_name.is_some(), name: from_name.clone(), into, rejections, moved_rejections, cleared };
    if into == from {
        return Ok(MergeUndo { faces: Vec::new(), took_name: false, rejections: Vec::new(), moved_rejections: Vec::new(), cleared: Vec::new(), ..undo });
    }
    for f in &undo.moved_rejections {
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![f, into, crate::now_unix()])?;
    }
    tx.execute("DELETE FROM face_rejections WHERE person_id = ?1", [from])?;
    for f in &undo.cleared {
        tx.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", params![f, into])?;
    }
    tx.execute("UPDATE faces SET person_id = ?1 WHERE person_id = ?2", params![into, from])?;
    tx.execute("UPDATE people SET name = COALESCE(name, ?2) WHERE id = ?1", params![into, from_name])?;
    tx.execute("DELETE FROM people WHERE id = ?1", [from])?;
    crate::fts::index_ids(&tx, &assets_of(conn, into)?)?;
    tx.commit()?;
    Ok(undo)
}

/// Undoes a merge: the moved faces become their own person again, with their old name.
pub fn restore_person(conn: &Connection, undo: &MergeUndo, now: i64) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("INSERT INTO people (name, cover_face_id, created_at) VALUES (?1, ?2, ?3)", params![undo.name, undo.faces.first(), now])?;
    let person = tx.last_insert_rowid();
    for f in &undo.faces {
        tx.execute("UPDATE faces SET person_id = ?1 WHERE id = ?2", params![person, f])?;
    }
    if undo.took_name {
        tx.execute("UPDATE people SET name = NULL WHERE id = ?1", [undo.into])?;
    }
    for f in &undo.rejections {
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![f, person, now])?;
    }
    for f in &undo.moved_rejections {
        tx.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", params![f, undo.into])?;
    }
    for f in &undo.cleared {
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![f, undo.into, now])?;
    }
    let mut touched = assets_of(&tx, person)?;
    touched.extend(assets_of(&tx, undo.into)?);
    crate::fts::index_ids(&tx, &touched)?;
    tx.commit()?;
    Ok(person)
}

/// The user says this face is `person`, or not the person it was with (`None`).
///
/// Placed on someone, the face is pinned: clustering leaves it there. Taken out of someone,
/// it is only refused by them (Round 4I): pinning it to nobody, as before, kept it out of
/// every person and every suggestion for good, although the user said only who it is not.
pub fn move_face(conn: &Connection, face: i64, person: Option<i64>) -> Result<()> {
    let (asset, was): (i64, Option<i64>) = conn.query_row("SELECT asset_id, person_id FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?)))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE faces SET person_id = ?2, pinned = (?2 IS NOT NULL) WHERE id = ?1", params![face, person])?;
    // Taken out of a person: it is not them, and is never offered back (Round 4B).
    if let Some(was) = was.filter(|w| Some(*w) != person) {
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![face, was, crate::now_unix()])?;
    }
    crate::fts::index_ids(&tx, &[asset])?;
    tx.commit()?;
    Ok(())
}

/// Who a face is on now.
pub fn face_person(conn: &Connection, face: i64) -> Result<Option<i64>> {
    Ok(conn.query_row("SELECT person_id FROM faces WHERE id = ?1", [face], |r| r.get(0)).optional()?.flatten())
}

/// Where a face was before the user moved it, for Undo.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FacePlace {
    #[ts(type = "number")]
    pub face: i64,
    #[ts(type = "number | null")]
    pub person: Option<i64>,
    pub pinned: bool,
}

/// The user says `person` is not in these photos: their faces there go to `to` (another
/// person, pinned) or back into play refused by `person` (Round 4I). A photo with two faces
/// of the person loses both. Returns where each face was, for Undo (`restore_faces`).
pub fn move_photos(conn: &Connection, person: i64, assets: &[i64], to: Option<i64>) -> Result<Vec<FacePlace>> {
    let tx = conn.unchecked_transaction()?;
    let mut moved = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT id, pinned FROM faces WHERE person_id = ?1 AND asset_id = ?2 ORDER BY id")?;
        for &asset in assets {
            let rows = stmt.query_map(params![person, asset], |r| Ok(FacePlace { face: r.get(0)?, person: Some(person), pinned: r.get(1)? }))?;
            for f in rows {
                moved.push(f?);
            }
        }
    }
    for f in &moved {
        tx.execute("UPDATE faces SET person_id = ?2, pinned = (?2 IS NOT NULL) WHERE id = ?1", params![f.face, to])?;
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![f.face, person, crate::now_unix()])?;
    }
    fix_covers(&tx, &[Some(person), to])?;
    crate::fts::index_ids(&tx, assets)?;
    tx.commit()?;
    Ok(moved)
}

/// Undoes `move_photos`: each face goes back to where it was.
pub fn restore_faces(conn: &Connection, places: &[FacePlace]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let mut touched = Vec::new();
    let mut people = Vec::new();
    for f in places {
        let now: Option<(i64, Option<i64>)> = tx.query_row("SELECT asset_id, person_id FROM faces WHERE id = ?1", [f.face], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
        let Some((asset, current)) = now else { continue };
        tx.execute("UPDATE faces SET person_id = ?2, pinned = ?3 WHERE id = ?1", params![f.face, f.person, f.pinned])?;
        // Back where it was: the "not this person" its move recorded no longer holds.
        if let Some(p) = f.person {
            tx.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", params![f.face, p])?;
        }
        touched.push(asset);
        people.extend([current, f.person]);
    }
    fix_covers(&tx, &people)?;
    crate::fts::index_ids(&tx, &touched)?;
    tx.commit()?;
    Ok(())
}

/// A person whose cover face is no longer theirs gets one of their own faces as cover.
fn fix_covers(conn: &Connection, people: &[Option<i64>]) -> Result<()> {
    for p in people.iter().flatten() {
        conn.execute(
            "UPDATE people SET cover_face_id = (SELECT id FROM faces WHERE person_id = ?1 ORDER BY score DESC, id LIMIT 1)
             WHERE id = ?1 AND (cover_face_id IS NULL OR cover_face_id NOT IN (SELECT id FROM faces WHERE person_id = ?1))",
            [p],
        )?;
    }
    Ok(())
}

/// *Remove from People* (Round 4B): a real person the user does not want. They leave the
/// People page, the viewer's names, search and suggestions; their faces stay theirs, so
/// their lookalikes join them quietly instead of becoming a new person. `restore_removed`
/// brings them back.
pub fn remove_person(conn: &Connection, person: i64) -> Result<()> {
    set_status(conn, person, "removed")
}

/// Undo of `remove_person`, and Settings → People's *Restore*.
pub fn restore_removed(conn: &Connection, person: i64) -> Result<()> {
    set_status(conn, person, "listed")
}

fn set_status(conn: &Connection, person: i64, status: &str) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE people SET status = ?2 WHERE id = ?1", params![person, status])?;
    crate::fts::index_ids(&tx, &assets_of(&tx, person)?)?;
    tx.commit()?;
    Ok(())
}

/// What *Not a person* took away, for Undo.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PersonUndo {
    #[ts(type = "number")]
    pub id: i64,
    pub name: Option<String>,
    #[ts(type = "number | null")]
    pub cover_face_id: Option<i64>,
    #[ts(type = "number")]
    pub created_at: i64,
    pub status: String,
    pub faces: Vec<FacePlace>,
    /// Faces said not to be this person.
    #[ts(type = "number[]")]
    pub rejections: Vec<i64>,
}

/// *Not a person* (Round 4B): none of this person's faces are faces — a poster, a statue, a
/// pattern. Each becomes a rejected row ('user'), so a rescan does not find it again as
/// new and nothing groups it, and the person goes.
pub fn reject_person(conn: &Connection, person: i64) -> Result<PersonUndo> {
    let tx = conn.unchecked_transaction()?;
    let (name, cover_face_id, created_at, status) =
        tx.query_row("SELECT name, cover_face_id, created_at, status FROM people WHERE id = ?1", [person], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let faces: Vec<FacePlace> = {
        let mut stmt = tx.prepare("SELECT id, pinned FROM faces WHERE person_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([person], |r| Ok(FacePlace { face: r.get(0)?, person: Some(person), pinned: r.get(1)? }))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let rejections: Vec<i64> = {
        let mut stmt = tx.prepare("SELECT face_id FROM face_rejections WHERE person_id = ?1 ORDER BY face_id")?;
        let rows = stmt.query_map([person], |r| r.get(0))?;
        rows.collect::<std::result::Result<_, _>>()?
    };
    let touched = assets_of(&tx, person)?;
    tx.execute("UPDATE faces SET rejected = 'user', person_id = NULL WHERE person_id = ?1", [person])?;
    tx.execute("DELETE FROM face_rejections WHERE person_id = ?1", [person])?;
    tx.execute("DELETE FROM people WHERE id = ?1", [person])?;
    crate::fts::index_ids(&tx, &touched)?;
    tx.commit()?;
    Ok(PersonUndo { id: person, name, cover_face_id, created_at, status, faces, rejections })
}

/// Undo of `reject_person`: the same person, with the same id, faces and pins.
pub fn undo_reject_person(conn: &Connection, undo: &PersonUndo) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO people (id, name, cover_face_id, created_at, status) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![undo.id, undo.name, undo.cover_face_id, undo.created_at, undo.status],
    )?;
    for f in &undo.faces {
        tx.execute("UPDATE faces SET rejected = NULL, person_id = ?2, pinned = ?3 WHERE id = ?1", params![f.face, f.person, f.pinned])?;
    }
    for f in &undo.rejections {
        tx.execute("INSERT OR IGNORE INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, ?3)", params![f, undo.id, crate::now_unix()])?;
    }
    crate::fts::index_ids(&tx, &assets_of(conn, undo.id)?)?;
    tx.commit()?;
    Ok(())
}

/// *Use as main photo*: `face` becomes the person's cover. It must be one of their faces.
/// Returns the cover it replaced, for Undo.
pub fn set_cover(conn: &Connection, person: i64, face: i64) -> Result<Option<i64>> {
    let theirs: bool = conn.query_row("SELECT EXISTS (SELECT 1 FROM faces WHERE id = ?1 AND person_id = ?2 AND rejected IS NULL)", params![face, person], |r| r.get(0))?;
    if !theirs {
        return Err(crate::CoreError::Invalid("That face is not this person's.".into()));
    }
    let before: Option<i64> = conn.query_row("SELECT cover_face_id FROM people WHERE id = ?1", [person], |r| r.get(0))?;
    conn.execute("UPDATE people SET cover_face_id = ?2 WHERE id = ?1", params![person, face])?;
    Ok(before)
}

/// A face's place in its photo (0..1) and the photo it is in, for the face crop route.
pub fn face_rect(conn: &Connection, face: i64) -> Result<Option<(i64, [f32; 4])>> {
    Ok(conn
        .query_row("SELECT asset_id, x, y, w, h FROM faces WHERE id = ?1", [face], |r| {
            Ok((r.get::<_, i64>(0)?, [r.get::<_, f64>(1)? as f32, r.get::<_, f64>(2)? as f32, r.get::<_, f64>(3)? as f32, r.get::<_, f64>(4)? as f32]))
        })
        .optional()?)
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AssetFace {
    #[ts(type = "number")]
    pub id: i64,
    #[ts(type = "number | null")]
    pub person_id: Option<i64>,
    pub name: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// A box the user drew (Round 4I).
    pub manual: bool,
}

/// The faces in one photo, left to right, with who they are.
pub fn faces_for_asset(conn: &Connection, asset: i64) -> Result<Vec<AssetFace>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, p.id, p.name, f.x, f.y, f.w, f.h, f.source = 'manual' FROM faces f LEFT JOIN people p ON p.id = f.person_id AND p.status = 'listed'
         WHERE f.asset_id = ?1 AND f.rejected IS NULL ORDER BY f.x",
    )?;
    let rows = stmt.query_map([asset], |r| {
        Ok(AssetFace {
            id: r.get(0)?,
            person_id: r.get(1)?,
            name: r.get(2)?,
            x: r.get::<_, f64>(3)? as f32,
            y: r.get::<_, f64>(4)? as f32,
            w: r.get::<_, f64>(5)? as f32,
            h: r.get::<_, f64>(6)? as f32,
            manual: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// A detection that is not a person's face, for review in Settings → People.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RejectedFace {
    #[ts(type = "number")]
    pub face_id: i64,
    #[ts(type = "number")]
    pub asset_id: i64,
    /// 'screen', 'drawing', 'print', 'statue' (the filter) or 'user' (*Not a person*).
    pub reason: String,
}

/// Rejected faces in visible photos, newest first, a page at a time: those with an id below
/// `before` (keyset).
pub fn rejected_faces(conn: &Connection, before: Option<i64>, limit: usize) -> Result<Vec<RejectedFace>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.asset_id, f.rejected FROM faces f JOIN assets a ON a.id = f.asset_id AND a.state = 0 AND a.concealed = 0
         WHERE f.rejected IS NOT NULL AND f.id < ?1 ORDER BY f.id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![before.unwrap_or(i64::MAX), limit as i64], |r| Ok(RejectedFace { face_id: r.get(0)?, asset_id: r.get(1)?, reason: r.get(2)? }))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// "This is a real face": a rejected detection comes back as `restored`, which no rescan
/// judges again, and is grouped like any face (Round 4I; it used to be pinned to nobody,
/// which kept it out of every person).
pub fn restore_face(conn: &Connection, face: i64) -> Result<()> {
    conn.execute("UPDATE faces SET rejected = NULL, pinned = 0, source = 'restored' WHERE id = ?1", [face])?;
    Ok(())
}

/// *Not a face* (Round 4G, a face box's menu in the viewer): the detection is rejected by the
/// user as Settings → People lists it, leaves its person and is never grouped again. Returns
/// where it was, for `undo_reject_face`.
pub fn reject_face(conn: &Connection, face: i64) -> Result<FacePlace> {
    let (person, pinned, asset): (Option<i64>, bool, i64) =
        conn.query_row("SELECT person_id, pinned, asset_id FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE faces SET rejected = 'user', person_id = NULL, pinned = 0 WHERE id = ?1", [face])?;
    tx.execute("UPDATE people SET cover_face_id = NULL WHERE cover_face_id = ?1", [face])?;
    fix_covers(&tx, &[person])?;
    crate::fts::index_ids(&tx, &[asset])?;
    tx.commit()?;
    Ok(FacePlace { face, person, pinned })
}

/// Undo of `reject_face`: the face is a face again, back where it was.
pub fn undo_reject_face(conn: &Connection, place: &FacePlace) -> Result<()> {
    conn.execute("UPDATE faces SET rejected = NULL WHERE id = ?1", [place.face])?;
    restore_faces(conn, std::slice::from_ref(place))
}

/// What `face_scan`-independent `faces.model` records for a box the user drew.
pub const MANUAL_MODEL: &str = "manual";

/// A box the user drew around someone (Round 4I, Task 6), embedded by the binary: with the
/// detector's landmarks inside it the embedding is aligned and `quality` is the detector's;
/// without, it is the box's centre embedded as it is and `quality` is 0. Such a face counts for
/// the person's photos, search and the viewer, but never stands for them in grouping
/// (`known_people` skips quality 0), so it cannot pull strangers in.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ManualFace {
    #[ts(type = "number")]
    pub asset_id: i64,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub score: f32,
    pub quality: f32,
    #[ts(type = "number[]")]
    pub vec: Vector,
}

/// Stores a drawn face on `person`, pinned, and returns its id. `id` reuses a deleted face's
/// id (Undo of `untag_face`).
pub fn add_manual_face(conn: &Connection, face: &ManualFace, person: i64, id: Option<i64>) -> Result<i64> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO faces (id, asset_id, x, y, w, h, score, vec, model, quality, person_id, pinned, source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 'manual')",
        params![id, face.asset_id, face.x as f64, face.y as f64, face.w as f64, face.h as f64, face.score as f64, blob(&face.vec), MANUAL_MODEL, face.quality as f64, person],
    )?;
    let id = tx.last_insert_rowid();
    fix_covers(&tx, &[Some(person)])?;
    crate::fts::index_ids(&tx, &[face.asset_id])?;
    tx.commit()?;
    Ok(id)
}

/// *Tag…* on a face: it is `person` (pinned). Returns where it was, for Undo (`restore_faces`).
pub fn tag_face(conn: &Connection, face: i64, person: i64) -> Result<FacePlace> {
    let (was, pinned): (Option<i64>, bool) = conn.query_row("SELECT person_id, pinned FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?)))?;
    move_face(conn, face, Some(person))?;
    // The user says who it is: whoever refused it before was right, and that stays recorded;
    // the person it now is cannot be among them.
    conn.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", params![face, person])?;
    let tx = conn.unchecked_transaction()?;
    fix_covers(&tx, &[Some(person), was])?;
    tx.commit()?;
    Ok(FacePlace { face, person: was, pinned })
}

/// A new named person. Returns their id.
pub fn create_person(conn: &Connection, name: &str, now: i64) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::CoreError::Invalid("A person needs a name.".into()));
    }
    conn.execute("INSERT INTO people (name, created_at, status) VALUES (?1, ?2, 'listed')", params![name, now])?;
    Ok(conn.last_insert_rowid())
}

/// *New person…* on a face: a new person called `name`, holding that face (pinned and their
/// cover). Returns the person and where the face was, for Undo (`undo_create_person`).
pub fn create_person_with_face(conn: &Connection, face: i64, name: &str, now: i64) -> Result<(i64, FacePlace)> {
    let person = create_person(conn, name, now)?;
    let place = tag_face(conn, face, person)?;
    conn.execute("UPDATE people SET cover_face_id = ?2 WHERE id = ?1", params![person, face])?;
    Ok((person, place))
}

/// Undo of `create_person_with_face`: the face goes back, and the person goes if nothing else
/// was put on them since.
pub fn undo_create_person(conn: &Connection, person: i64, place: &FacePlace) -> Result<()> {
    restore_faces(conn, std::slice::from_ref(place))?;
    let empty: bool = conn.query_row("SELECT NOT EXISTS (SELECT 1 FROM faces WHERE person_id = ?1)", [person], |r| r.get(0))?;
    if empty {
        conn.execute("DELETE FROM face_rejections WHERE person_id = ?1", [person])?;
        conn.execute("DELETE FROM people WHERE id = ?1", [person])?;
    }
    Ok(())
}

/// What `untag_face` undid, for its Undo (`undo_untag_face`).
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UntagUndo {
    pub place: FacePlace,
    /// A drawn face is deleted; this brings it back with the same id.
    pub manual: Option<ManualFace>,
}

/// *Not {name}* on a face. A scanned face goes back into play refused by that person (Round
/// 4I's rule); a drawn face is deleted, since it was only ever the user's word.
pub fn untag_face(conn: &Connection, face: i64) -> Result<UntagUndo> {
    let (person, pinned, source): (Option<i64>, bool, String) = conn.query_row("SELECT person_id, pinned, source FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let place = FacePlace { face, person, pinned };
    if source != "manual" {
        move_face(conn, face, None)?;
        let tx = conn.unchecked_transaction()?;
        fix_covers(&tx, &[person])?;
        tx.commit()?;
        return Ok(UntagUndo { place, manual: None });
    }
    let manual = conn.query_row("SELECT asset_id, x, y, w, h, score, quality, vec FROM faces WHERE id = ?1", [face], |r| {
        Ok(ManualFace {
            asset_id: r.get(0)?,
            x: r.get::<_, f64>(1)? as f32,
            y: r.get::<_, f64>(2)? as f32,
            w: r.get::<_, f64>(3)? as f32,
            h: r.get::<_, f64>(4)? as f32,
            score: r.get::<_, f64>(5)? as f32,
            quality: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0) as f32,
            vec: unblob(&r.get::<_, Vec<u8>>(7)?),
        })
    })?;
    let asset_id = manual.asset_id;
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM face_rejections WHERE face_id = ?1", [face])?;
    tx.execute("UPDATE people SET cover_face_id = NULL WHERE cover_face_id = ?1", [face])?;
    tx.execute("DELETE FROM faces WHERE id = ?1", [face])?;
    fix_covers(&tx, &[person])?;
    crate::fts::index_ids(&tx, &[asset_id])?;
    tx.commit()?;
    Ok(UntagUndo { place, manual: Some(manual) })
}

/// Undo of `untag_face`.
pub fn undo_untag_face(conn: &Connection, undo: &UntagUndo) -> Result<()> {
    match (&undo.manual, undo.place.person) {
        (Some(m), Some(person)) => add_manual_face(conn, m, person, Some(undo.place.face)).map(|_| ()),
        (Some(_), None) => Ok(()),
        (None, _) => restore_faces(conn, std::slice::from_ref(&undo.place)),
    }
}

/// How many faces and people there are, for Settings.
pub fn counts(conn: &Connection) -> Result<(i64, i64)> {
    Ok(conn.query_row(
        "SELECT (SELECT COUNT(*) FROM faces WHERE rejected IS NULL), (SELECT COUNT(*) FROM people WHERE status = 'listed')",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

/// For tests and the FTS row query: the names in a photo.
pub fn names_in(conn: &Connection, asset: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT DISTINCT p.name FROM faces f JOIN people p ON p.id = f.person_id WHERE f.asset_id = ?1 AND p.name IS NOT NULL AND p.status = 'listed'")?;
    let rows = stmt.query_map([asset], |r| r.get(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{quantize, DIM};
    use crate::Db;

    /// Unit vectors near `axis`, jittered by `k` so no two are equal.
    fn near(axis: usize, k: usize) -> Vector {
        let mut v = vec![0f32; DIM];
        v[axis] = 1.0;
        v[(axis + 7 + k) % DIM] = 0.15;
        quantize(&v)
    }

    #[test]
    fn two_tight_groups_and_a_stray_make_two_people() {
        let mut faces: Vec<(i64, Vector)> = (0..3).map(|k| (k as i64 + 1, near(0, k))).collect();
        faces.extend((0..4).map(|k| (k as i64 + 10, near(100, k))));
        faces.push((99, near(300, 0)));
        let groups = cluster(&faces, 0.5);
        assert_eq!(groups, vec![vec![1, 2, 3], vec![10, 11, 12, 13]]);
    }

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for i in 1..=8 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', ?3, 'jpg', 0, 1, 1, 10)",
                    params![i, format!("h{i}"), format!("{i}.jpg")],
                )?;
            }
            crate::fts::rebuild(c)?;
            Ok(())
        })
        .unwrap();
        db
    }

    fn face(v: Vector) -> NewFace {
        NewFace { x: 0.4, y: 0.3, w: 0.1, h: 0.15, score: 0.9, vec: v, quality: 0.9, rejected: None }
    }

    #[test]
    fn grouping_never_undoes_what_the_user_did_while_it_was_thinking() {
        let db = library();
        db.with_conn(|c| {
            for (asset, k) in [(1, 0), (2, 1), (3, 2)] {
                store_faces(c, asset, "sface@1", &[face(near(0, k))])?;
            }
            recluster(c, &Thresholds::uniform(0.5), 1)?;
            let anna = list_people(c)?[0].id;
            store_faces(c, 4, "sface@1", &[face(near(0, 3))])?;
            store_faces(c, 5, "sface@1", &[face(near(0, 4))])?;
            let late: Vec<i64> = c.prepare("SELECT id FROM faces WHERE asset_id IN (4, 5) ORDER BY asset_id")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;

            // Read and decided; meanwhile the user says photo 4's face is nobody's.
            let plan = ReclusterInput::read(c)?.plan(&Thresholds::uniform(0.5));
            c.execute("UPDATE faces SET pinned = 1 WHERE id = ?1", [late[0]])?;
            assert_eq!(plan.apply(c, 2)?, 1, "only the face still open is placed");
            let person_of = |f: i64| c.query_row("SELECT person_id FROM faces WHERE id = ?1", [f], |r| r.get::<_, Option<i64>>(0));
            assert_eq!(person_of(late[0])?, None);
            assert_eq!(person_of(late[1])?, Some(anna));

            // A person merged away meanwhile takes nobody.
            store_faces(c, 6, "sface@1", &[face(near(0, 5))])?;
            let plan = ReclusterInput::read(c)?.plan(&Thresholds::uniform(0.5));
            c.execute("UPDATE faces SET person_id = NULL, pinned = 1 WHERE person_id = ?1", [anna])?;
            c.execute("DELETE FROM people WHERE id = ?1", [anna])?;
            assert_eq!(plan.apply(c, 3)?, 0);
            Ok(())
        })
        .unwrap();
    }

    /// Round 10's check of the refusals as evidence on a COPY of a real library: every face
    /// the user placed by hand is the truth. A veto of one on its own person would be a wrong
    /// one (the user said it is them); a veto of one on a person it is not would be right.
    ///
    /// LUMEN_DB=<copy> cargo test --release -p photos-core --lib veto_on_a_copy -- --ignored --nocapture
    #[test]
    #[ignore = "needs a copy of a real library.db"]
    fn veto_on_a_copy() {
        let db = Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        let (evidence, against, pinned) = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT id, person_id, vec FROM faces WHERE pinned = 1 AND person_id IS NOT NULL AND rejected IS NULL")?;
                let pinned = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, unblob(&r.get::<_, Vec<u8>>(2)?))))?.collect::<std::result::Result<Vec<_>, _>>()?;
                Ok((evidence_rows(c)?, against_rows(c)?, pinned))
            })
            .unwrap();
        let known = with_against(known_from(evidence), against);
        let t = Thresholds::MEASURED;
        {
            let (mut own, mut own_vetoed, mut other, mut other_vetoed) = (0, 0, 0, 0);
            for (_, person, v) in &pinned {
                let f = Ev::new(v.clone());
                for k in known.iter().filter(|k| !k.against.is_empty()) {
                    let mut l = Likeness::new();
                    for e in k.evidence.iter().filter(|e| e.v != f.v) {
                        l.add(f.cos(e), t.suggest);
                    }
                    // Only faces the person would otherwise take matter.
                    if l.score() < t.join - 0.05 {
                        continue;
                    }
                    let hit = vetoed(&f, k, &t);
                    if k.id == *person {
                        own += 1;
                        own_vetoed += hit as usize;
                    } else {
                        other += 1;
                        other_vetoed += hit as usize;
                    }
                }
            }
            println!(
                "wrong vetoes {own_vetoed} of {own} hand-placed faces on their own person ({:.2} %); right vetoes {other_vetoed} of {other} near-misses on someone else ({:.1} %)",
                100.0 * own_vetoed as f64 / own.max(1) as f64,
                100.0 * other_vetoed as f64 / other.max(1) as f64
            );
        }
    }

    /// A look-alike of `axis`'s person: cosine ≈ 0.8 to them, ≈ 0.99 to each other.
    pub(crate) fn lookalike(axis: usize, k: usize) -> Vector {
        let mut v = vec![0f32; DIM];
        v[axis] = 0.8;
        v[(axis + 50) % DIM] = 0.6;
        v[(axis + 57 + k) % DIM] = 0.1;
        quantize(&v)
    }

    #[test]
    fn a_face_like_one_refused_for_someone_never_joins_them() {
        let db = library();
        db.with_conn(|c| {
            for (asset, k) in [(1, 0), (2, 1), (3, 2)] {
                store_faces(c, asset, "sface@1", &[face(near(0, k))])?;
            }
            store_faces(c, 4, "sface@1", &[face(lookalike(0, 0))])?;
            recluster(c, &Thresholds::uniform(0.5), 1)?;
            let anna = list_people(c)?[0].id;
            let person_of = |a: i64| c.query_row("SELECT person_id FROM faces WHERE asset_id = ?1", [a], |r| r.get::<_, Option<i64>>(0));
            assert_eq!(person_of(4)?, Some(anna), "the look-alike joined Anna");
            // The user says the look-alike is not Anna.
            let refused: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 4", [], |r| r.get(0))?;
            untag_face(c, refused)?;
            // Another photo of the look-alike, and one of Anna.
            store_faces(c, 5, "sface@1", &[face(lookalike(0, 1))])?;
            store_faces(c, 6, "sface@1", &[face(near(0, 3))])?;
            recluster(c, &Thresholds::uniform(0.5), 2)?;
            assert_eq!(person_of(5)?, None, "more like the face refused for Anna than like Anna");
            assert_eq!(person_of(6)?, Some(anna), "Anna's own photos still join her");
            assert!(suggestions(c, anna, &Thresholds::uniform(0.5), 10)?.iter().all(|s| s.asset_id != 5), "and it is not asked about");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_named_person_keeps_their_name_and_gains_new_faces() {
        let db = library();
        db.with_conn(|c| {
            for (asset, k) in [(1, 0), (2, 1), (3, 2)] {
                store_faces(c, asset, "sface@1", &[face(near(0, k))])?;
            }
            assert_eq!(recluster(c, &Thresholds::uniform(0.5), 1)?, 3);
            let anna = list_people(c)?[0].id;
            name_person(c, anna, "Anna")?;
            store_faces(c, 4, "sface@1", &[face(near(0, 3))])?;
            recluster(c, &Thresholds::uniform(0.5), 2)?;
            let people = list_people(c)?;
            assert_eq!(people.len(), 1);
            assert_eq!(people[0].name.as_deref(), Some("Anna"));
            assert_eq!(people[0].photo_count, 4);
            // The name is searchable.
            let q = crate::query::AssetQuery { text: Some("anna".into()), ..Default::default() };
            let scope = crate::scope::LayoutScope { query: Some(q), ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &scope)?, 4);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn photos_move_out_of_a_person_and_back() {
        let db = library();
        db.with_conn(|c| {
            for (asset, axis, k) in [(1, 0, 0), (2, 0, 1), (3, 0, 2), (4, 0, 3), (5, 100, 0), (6, 100, 1), (7, 100, 2)] {
                store_faces(c, asset, "sface@1", &[face(near(axis, k))])?;
            }
            recluster(c, &Thresholds::uniform(0.5), 1)?;
            let people = list_people(c)?;
            let (anna, ben) = (people.iter().find(|p| p.photo_count == 4).unwrap().id, people.iter().find(|p| p.photo_count == 3).unwrap().id);
            let scope = |p: i64| crate::scope::LayoutScope { person_id: Some(p), ..Default::default() };
            // Photo 1 is not Anna at all; photo 2 is Ben.
            let a = move_photos(c, anna, &[1], None)?;
            let b = move_photos(c, anna, &[2], Some(ben))?;
            assert_eq!(crate::layout::count_scoped(c, &scope(anna))?, 2);
            assert_eq!(crate::layout::count_scoped(c, &scope(ben))?, 4);
            // Clustering leaves the user's decision alone.
            recluster(c, &Thresholds::uniform(0.5), 2)?;
            assert_eq!(crate::layout::count_scoped(c, &scope(anna))?, 2);
            // Anna's cover is one of her own faces.
            let cover: i64 = c.query_row("SELECT cover_face_id FROM people WHERE id = ?1", [anna], |r| r.get(0))?;
            let owner: i64 = c.query_row("SELECT person_id FROM faces WHERE id = ?1", [cover], |r| r.get(0))?;
            assert_eq!(owner, anna);
            restore_faces(c, &b)?;
            restore_faces(c, &a)?;
            assert_eq!(crate::layout::count_scoped(c, &scope(anna))?, 4);
            assert_eq!(crate::layout::count_scoped(c, &scope(ben))?, 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn merging_and_moving_faces() {
        let db = library();
        db.with_conn(|c| {
            for (asset, axis, k) in [(1, 0, 0), (2, 0, 1), (3, 0, 2), (4, 100, 0), (5, 100, 1), (6, 100, 2)] {
                store_faces(c, asset, "sface@1", &[face(near(axis, k))])?;
            }
            recluster(c, &Thresholds::uniform(0.5), 1)?;
            let people = list_people(c)?;
            assert_eq!(people.len(), 2);
            name_person(c, people[1].id, "Ben")?;
            // Merge and undo: two people again, the name back where it was.
            let undo = merge_people(c, people[0].id, people[1].id)?;
            assert_eq!(undo.faces.len(), 3);
            restore_person(c, &undo, 2)?;
            let again = list_people(c)?;
            assert_eq!(again.len(), 2);
            assert_eq!(again.iter().filter(|p| p.name.as_deref() == Some("Ben")).count(), 1);
            let ben = again.iter().find(|p| p.name.as_deref() == Some("Ben")).unwrap().id;
            let other = again.iter().find(|p| p.id != ben).unwrap().id;
            merge_people(c, other, ben)?;
            let merged = list_people(c)?;
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].face_count, 6);
            assert_eq!(merged[0].name.as_deref(), Some("Ben"), "the unnamed side takes the other's name");
            // A face the user moves out stays out.
            let f = faces_for_asset(c, 6)?[0].id;
            move_face(c, f, None)?;
            recluster(c, &Thresholds::uniform(0.5), 2)?;
            assert_eq!(faces_for_asset(c, 6)?[0].person_id, None);
            // The grid scoped to a person.
            let scope = crate::scope::LayoutScope { person_id: Some(merged[0].id), ..Default::default() };
            assert_eq!(crate::layout::count_scoped(c, &scope)?, 5);
            Ok(())
        })
        .unwrap();
    }

    /// How many photos a search for `word` finds.
    fn found(c: &Connection, word: &str) -> Result<i64> {
        Ok(c.query_row("SELECT COUNT(*) FROM assets_fts WHERE assets_fts MATCH ?1", [word], |r| r.get(0))?)
    }

    /// Round 4B, Task 5: *Not a person* — none of these are faces. They stay as rejected
    /// rows (a rescan never finds them again as new), nothing regroups them, and Undo puts
    /// the person back exactly.
    #[test]
    fn not_a_person_rejects_every_face_and_undo_brings_them_back() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let pinned = faces_for_asset(c, 2)?[0].id;
            move_face(c, pinned, Some(anna))?;
            let undo = reject_person(c, anna)?;
            assert_eq!(list_people(c)?.len(), 1);
            assert!(faces_for_asset(c, 1)?.is_empty());
            let reasons: Vec<Option<String>> = c.prepare("SELECT rejected FROM faces WHERE asset_id IN (1, 2, 3)")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            assert!(reasons.iter().all(|r| r.as_deref() == Some("user")));
            recluster(c, &Thresholds::uniform(0.5), 2)?;
            assert_eq!(list_people(c)?.len(), 1, "never grouped again");
            assert!(found(c, "Anna")? == 0);

            undo_reject_person(c, &undo)?;
            let back = list_people(c)?;
            assert!(back.iter().any(|p| p.id == anna && p.name.as_deref() == Some("Anna") && p.photo_count == 3));
            let (person, is_pinned): (Option<i64>, i64) = c.query_row("SELECT person_id, pinned FROM faces WHERE id = ?1", [pinned], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((person, is_pinned), (Some(anna), 1));
            assert_eq!(found(c, "Anna")?, 3);
            Ok(())
        })
        .unwrap();
    }

    /// *Remove from People*: a real person not wanted — gone from People, the viewer's
    /// names and search, and back with *Restore*.
    #[test]
    fn a_removed_person_leaves_people_names_and_search() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            remove_person(c, anna)?;
            assert!(list_people(c)?.iter().all(|p| p.id != anna));
            assert_eq!(removed_people(c)?.iter().map(|p| p.id).collect::<Vec<_>>(), vec![anna]);
            let f = &faces_for_asset(c, 1)?[0];
            assert_eq!((f.person_id, f.name.as_deref()), (None, None), "the viewer names nobody");
            assert!(names_in(c, 1)?.is_empty());
            assert!(found(c, "Anna")? == 0);
            assert_eq!(counts(c)?.1, 1);
            restore_removed(c, anna)?;
            assert!(list_people(c)?.iter().any(|p| p.id == anna));
            assert_eq!(found(c, "Anna")?, 3);
            Ok(())
        })
        .unwrap();
    }

    /// *Use as main photo*: only one of the person's own faces, and Undo gets the old one.
    #[test]
    fn the_main_photo_is_one_of_the_persons_own_faces() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let before = list_people(c)?.into_iter().find(|p| p.id == anna).unwrap().cover_face_id;
            let third = faces_for_asset(c, 3)?[0].id;
            assert_eq!(set_cover(c, anna, third)?, before);
            assert_eq!(list_people(c)?.into_iter().find(|p| p.id == anna).unwrap().cover_face_id, Some(third));
            let bens = faces_for_asset(c, 4)?[0].id;
            assert!(matches!(set_cover(c, anna, bens), Err(crate::CoreError::Invalid(_))));
            Ok(())
        })
        .unwrap();
    }

    /// Review focus 4: "not Ben" on a face still holds once Ben is merged into Anna (the
    /// same person now), and Undo of the merge puts it back on Ben.
    #[test]
    fn a_merge_carries_the_rejections_and_undo_returns_them() {
        let db = library();
        db.with_conn(|c| {
            let (anna, ben) = anna_and_ben(c)?;
            store_faces(c, 7, "m", &[face(near(300, 0))])?;
            let stranger = faces_for_asset(c, 7)?[0].id;
            answer_suggestion(c, stranger, ben, false, 1)?;
            let refused = |p: i64| -> Result<bool> { Ok(c.query_row("SELECT COUNT(*) FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", [stranger, p], |r| r.get::<_, i64>(0))? == 1) };
            let undo = merge_people(c, anna, ben)?;
            assert!(refused(anna)?, "not Ben is not Anna, now they are one");
            let ben_again = restore_person(c, &undo, 2)?;
            assert!(refused(ben_again)?);
            assert!(!refused(anna)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_rescan_replaces_faces_but_keeps_pinned_ones() {
        let db = library();
        db.with_conn(|c| {
            store_faces(c, 1, "sface@1", &[face(near(0, 0)), face(near(5, 0))])?;
            let pinned = faces_for_asset(c, 1)?[0].id;
            // Placed on someone by the user (Round 4I: only a placement pins).
            c.execute("INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 0)", [])?;
            move_face(c, pinned, Some(1))?;
            store_faces(c, 1, "sface@1", &[NewFace { x: 0.8, ..face(near(9, 0)) }])?;
            let now: Vec<i64> = faces_for_asset(c, 1)?.iter().map(|f| f.id).collect();
            assert_eq!(now.len(), 2);
            assert!(now.contains(&pinned));
            assert!(unscanned(c, "sface@1", 10)?.iter().all(|(id, _)| *id != 1));
            assert!(unscanned(c, "sface@2", 10)?.iter().any(|(id, _)| *id == 1), "a new detector rescans");
            Ok(())
        })
        .unwrap();
    }

    /// Review fix: a photo the rescan cannot read keeps the faces it has, and is not asked
    /// about again.
    #[test]
    fn an_unreadable_photo_keeps_its_faces() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            mark_scanned(c, 1, "new")?;
            assert_eq!(faces_for_asset(c, 1)?[0].person_id, Some(anna));
            assert!(unscanned(c, "new", 100)?.iter().all(|(id, _)| *id != 1));
            Ok(())
        })
        .unwrap();
    }

    /// Review fix: a rescan that loses a person's face updates search and the cover.
    #[test]
    fn a_rescan_that_loses_a_face_updates_search_and_the_cover() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let first = faces_for_asset(c, 1)?[0].id;
            set_cover(c, anna, first)?;
            assert_eq!(found(c, "Anna")?, 3);
            // Found nowhere near the old box: the old face goes.
            replace_faces_keeping_identity(c, 1, "new", &[NewFace { x: 0.9, y: 0.9, w: 0.05, h: 0.05, ..face(near(300, 0)) }])?;
            assert_eq!(found(c, "Anna")?, 2, "photo 1 no longer has Anna");
            let cover: i64 = c.query_row("SELECT cover_face_id FROM people WHERE id = ?1", [anna], |r| r.get(0))?;
            assert_ne!(cover, first);
            assert_eq!(c.query_row("SELECT person_id FROM faces WHERE id = ?1", [cover], |r| r.get::<_, i64>(0))?, anna);
            Ok(())
        })
        .unwrap();
    }

    /// Review fix (focus 2): what the filter and *Not a person* took away can be looked
    /// through, newest first, and restored.
    #[test]
    fn rejected_faces_are_listed_for_review() {
        let db = library();
        db.with_conn(|c| {
            store_faces(c, 1, "m", &[NewFace { rejected: Some("print".into()), ..face(near(0, 0)) }])?;
            store_faces(c, 2, "m", &[face(near(0, 1))])?;
            store_faces(c, 3, "m", &[NewFace { rejected: Some("screen".into()), ..face(near(0, 2)) }])?;
            let page = rejected_faces(c, None, 1)?;
            assert_eq!(page.len(), 1);
            assert_eq!((page[0].asset_id, page[0].reason.as_str()), (3, "screen"));
            let next = rejected_faces(c, Some(page[0].face_id), 10)?;
            assert_eq!(next.iter().map(|f| f.asset_id).collect::<Vec<_>>(), vec![1]);
            restore_face(c, next[0].face_id)?;
            assert_eq!(rejected_faces(c, None, 10)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    /// The rescan reaches the photos Julian took last first.
    #[test]
    fn the_newest_photos_are_scanned_first() {
        let db = library();
        db.with_conn(|c| {
            let ids: Vec<i64> = unscanned(c, "m", 3)?.into_iter().map(|(id, _)| id).collect();
            assert_eq!(ids, vec![8, 7, 6]);
            Ok(())
        })
        .unwrap();
    }

    /// Round 4B, Task 4: the rescan at a larger size finds the same faces again in slightly
    /// different boxes. Each keeps its row (its id, person, pin and rejection) when the new
    /// box overlaps the old one; only machine-made faces with no match go.
    #[test]
    fn a_rescan_keeps_every_pinned_face() {
        let db = library();
        db.with_conn(|c| {
            let at = |x: f32, y: f32, v: Vector| NewFace { x, y, w: 0.1, h: 0.1, ..face(v) };
            store_faces(c, 1, "old", &[at(0.10, 0.10, near(0, 0)), at(0.50, 0.10, near(100, 0)), at(0.10, 0.60, near(200, 0)), at(0.70, 0.70, near(300, 0))])?;
            let id_at = |x: f64, y: f64| -> Result<i64> {
                Ok(c.query_row("SELECT id FROM faces WHERE asset_id = 1 AND abs(x - ?1) < 0.001 AND abs(y - ?2) < 0.001", [x, y], |r| r.get(0))?)
            };
            let (a, b, cc) = (id_at(0.10, 0.10)?, id_at(0.50, 0.10)?, id_at(0.10, 0.60)?);
            // E (0.70, 0.70): automatic, unassigned, not found again — it goes.
            c.execute("INSERT INTO people (id, name, created_at, cover_face_id) VALUES (1, 'Anna', 0, ?1), (2, 'Ben', 0, ?2)", [a, b])?;
            c.execute("UPDATE faces SET person_id = 1, pinned = 1 WHERE id = ?1", [a])?;
            c.execute("UPDATE faces SET person_id = 2 WHERE id = ?1", [b])?;
            c.execute("UPDATE faces SET rejected = 'user' WHERE id = ?1", [cc])?;

            // A′ overlaps A (IoU ≈ 0.8), B′ overlaps B (≈ 0.7), C found again, D is new.
            let fresh = [at(0.105, 0.105, near(0, 1)), at(0.51, 0.115, near(100, 1)), at(0.10, 0.60, near(200, 1)), at(0.30, 0.30, near(400, 0))];
            replace_faces_keeping_identity(c, 1, "new", &fresh)?;

            let row = |id: i64| -> Result<(Option<i64>, bool, Option<String>, f64)> {
                Ok(c.query_row("SELECT person_id, pinned, rejected, x FROM faces WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0, r.get(2)?, r.get(3)?)))?)
            };
            let (p, pinned, rej, x) = row(a)?;
            assert_eq!((p, pinned, rej), (Some(1), true, None), "A′ is still Anna, pinned");
            assert!((x - 0.105).abs() < 1e-6, "in its new box");
            assert_eq!(row(b)?.0, Some(2), "B′ is still Ben");
            assert_eq!(row(cc)?.2.as_deref(), Some("user"), "C stays Not a person");
            let total: i64 = c.query_row("SELECT COUNT(*) FROM faces WHERE asset_id = 1", [], |r| r.get(0))?;
            assert_eq!(total, 4, "A′, B′, C and the new D; E is gone");
            let d: (Option<i64>, bool) = c.query_row("SELECT person_id, pinned FROM faces WHERE asset_id = 1 AND abs(x - 0.30) < 0.001", [], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))?;
            assert_eq!(d, (None, false));
            let cover: i64 = c.query_row("SELECT cover_face_id FROM people WHERE id = 1", [], |r| r.get(0))?;
            assert_eq!(cover, a, "the cover still points at Anna's face");
            assert!(unscanned(c, "new", 10)?.iter().all(|(id, _)| *id != 1));
            Ok(())
        })
        .unwrap();
    }

    /// Round 4B: a face on a screen or a statue is stored with its reason, so a rescan does
    /// not find it again as new, but it is never grouped, counted or listed.
    #[test]
    fn a_rejected_face_is_kept_but_never_grouped_counted_or_listed() {
        let db = library();
        db.with_conn(|c| {
            store_faces(c, 1, "m", &[NewFace { rejected: Some("screen".into()), ..face(near(0, 0)) }])?;
            for k in 0..3 {
                store_faces(c, 2 + k as i64, "m", &[face(near(0, k + 1))])?;
            }
            recluster(c, &Thresholds::uniform(0.5), 0)?;
            assert!(faces_for_asset(c, 1)?.is_empty());
            let (reason, person): (Option<String>, Option<i64>) = c.query_row("SELECT rejected, person_id FROM faces WHERE asset_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((reason.as_deref(), person), (Some("screen"), None));
            assert_eq!(counts(c)?.0, 3);
            // "This is a real face": back, and grouped like any face (Round 4I: not pinned to
            // nobody, which kept it out of every person for good).
            let id: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 1", [], |r| r.get(0))?;
            restore_face(c, id)?;
            assert_eq!(faces_for_asset(c, 1)?.len(), 1);
            let (pinned, source): (i64, String) = c.query_row("SELECT pinned, source FROM faces WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((pinned, source.as_str()), (0, "restored"));
            recluster(c, &Thresholds::uniform(0.5), 1)?;
            let joined: Option<i64> = c.query_row("SELECT person_id FROM faces WHERE id = ?1", [id], |r| r.get(0))?;
            assert!(joined.is_some(), "a restored face joins the person it looks like");
            Ok(())
        })
        .unwrap();
    }

    /// A unit vector made of these axis weights (normalised).
    fn mix(parts: &[(usize, f32)]) -> Vector {
        let mut v = vec![0f32; DIM];
        for (axis, w) in parts {
            v[*axis] += *w;
        }
        quantize(&v)
    }

    const T: Thresholds = Thresholds { join: 0.35, margin: 0.05, suggest: 0.3, cluster: 0.35, place_quality: 0.0 };

    /// Anna (axis 0) on photos 1–3, Ben (axis 100) on 4–6, both named.
    fn anna_and_ben(c: &Connection) -> Result<(i64, i64)> {
        for k in 0..3 {
            store_faces(c, 1 + k as i64, "m", &[face(near(0, k))])?;
            store_faces(c, 4 + k as i64, "m", &[face(near(100, k))])?;
        }
        recluster(c, &T, 0)?;
        let who = |asset: i64| -> Result<i64> { Ok(c.query_row("SELECT person_id FROM faces WHERE asset_id = ?1", [asset], |r| r.get(0))?) };
        let (anna, ben) = (who(1)?, who(4)?);
        name_person(c, anna, "Anna")?;
        name_person(c, ben, "Ben")?;
        Ok((anna, ben))
    }

    fn person_of(c: &Connection, asset: i64) -> Option<i64> {
        c.query_row("SELECT person_id FROM faces WHERE asset_id = ?1 AND rejected IS NULL", [asset], |r| r.get(0)).unwrap()
    }

    #[test]
    fn a_face_without_a_clear_lead_is_asked_about_not_joined() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            // About 0.40 to Anna and 0.38 to Ben: over the join, but no clear lead.
            store_faces(c, 7, "m", &[face(mix(&[(0, 0.40), (100, 0.38), (300, 0.83)]))])?;
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 7), None);
            let asked = suggestions(c, anna, &T, 10)?;
            assert_eq!(asked.iter().map(|s| s.asset_id).collect::<Vec<_>>(), vec![7]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_face_said_not_to_be_anna_never_joins_or_is_offered_to_her() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            store_faces(c, 7, "m", &[face(near(0, 9))])?;
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 7", [], |r| r.get(0))?;
            answer_suggestion(c, f, anna, false, 1)?;
            recluster(c, &T, 2)?;
            assert_eq!(person_of(c, 7), None);
            assert!(suggestions(c, anna, &T, 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn yes_and_no_to_a_suggestion_are_undone_exactly() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            store_faces(c, 7, "m", &[face(mix(&[(0, 0.40), (100, 0.38), (300, 0.83)]))])?;
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 7", [], |r| r.get(0))?;
            let yes = answer_suggestion(c, f, anna, true, 1)?;
            assert_eq!(person_of(c, 7), Some(anna));
            undo_suggestion(c, &yes)?;
            assert_eq!(person_of(c, 7), None);
            let pinned: i64 = c.query_row("SELECT pinned FROM faces WHERE id = ?1", [f], |r| r.get(0))?;
            assert_eq!(pinned, 0);
            let no = answer_suggestion(c, f, anna, false, 2)?;
            assert!(suggestions(c, anna, &T, 10)?.is_empty());
            undo_suggestion(c, &no)?;
            assert_eq!(suggestions(c, anna, &T, 10)?.len(), 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_blurry_face_pinned_into_anna_does_not_pull_strangers_in() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            // The user put a low-quality face (axis 200) into Anna.
            store_faces(c, 7, "m", &[NewFace { quality: 0.1, ..face(near(200, 0)) }])?;
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 7", [], |r| r.get(0))?;
            move_face(c, f, Some(anna))?;
            // A stranger who looks like that face only.
            store_faces(c, 8, "m", &[face(near(200, 3))])?;
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 8), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_face_moved_out_of_a_person_is_never_offered_back() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 1", [], |r| r.get(0))?;
            move_face(c, f, None)?;
            recluster(c, &T, 1)?;
            assert!(suggestions(c, anna, &T, 10)?.iter().all(|s| s.face_id != f));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn removed_people_stay_removed() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            remove_person(c, anna)?;
            let before: i64 = c.query_row("SELECT COUNT(*) FROM people", [], |r| r.get(0))?;
            // New photos of her: they go to her, hidden, and never make a new unnamed person.
            for k in 0..2 {
                store_faces(c, 7 + k as i64, "m", &[face(near(0, 10 + k))])?;
            }
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 7), Some(anna));
            let after: i64 = c.query_row("SELECT COUNT(*) FROM people", [], |r| r.get(0))?;
            assert_eq!(before, after);
            Ok(())
        })
        .unwrap();
    }

    fn face_on(c: &Connection, asset: i64) -> i64 {
        c.query_row("SELECT id FROM faces WHERE asset_id = ?1", [asset], |r| r.get(0)).unwrap()
    }

    fn pin_of(c: &Connection, face: i64) -> (Option<i64>, bool) {
        c.query_row("SELECT person_id, pinned FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0))).unwrap()
    }

    fn refused_by(c: &Connection, face: i64) -> Vec<i64> {
        c.prepare("SELECT person_id FROM face_rejections WHERE face_id = ?1 ORDER BY person_id")
            .unwrap()
            .query_map([face], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap()
    }

    /// Round 4I, report 3: a face wrongly in Anna and taken out of her was pinned to nobody,
    /// so it was never grouped or suggested again. Now it is only refused by Anna.
    #[test]
    fn a_face_taken_out_of_someone_is_still_in_play() {
        let db = library();
        db.with_conn(|c| {
            let (anna, ben) = anna_and_ben(c)?;
            // Ben's face, wrongly placed in Anna, then taken out of her.
            store_faces(c, 7, "m", &[face(near(100, 5))])?;
            let f = face_on(c, 7);
            move_face(c, f, Some(anna))?;
            move_face(c, f, None)?;
            assert_eq!(pin_of(c, f), (None, false));
            assert_eq!(refused_by(c, f), vec![anna]);
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 7), Some(ben));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_face_taken_out_and_doubtful_is_suggested_to_someone_else() {
        let db = library();
        db.with_conn(|c| {
            let (anna, ben) = anna_and_ben(c)?;
            // About 0.33 to Ben: doubtful for him (between T's suggest and join).
            store_faces(c, 7, "m", &[face(mix(&[(100, 0.33), (300, 0.94)]))])?;
            let f = face_on(c, 7);
            move_face(c, f, Some(anna))?;
            move_photos(c, anna, &[7], None)?;
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 7), None);
            assert!(suggestions(c, ben, &T, 10)?.iter().any(|s| s.face_id == f));
            assert!(suggestions(c, anna, &T, 10)?.iter().all(|s| s.face_id != f));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn placing_a_face_on_someone_still_pins_it() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let f = face_on(c, 4);
            move_face(c, f, Some(anna))?;
            assert_eq!(pin_of(c, f), (Some(anna), true));
            recluster(c, &T, 1)?;
            assert_eq!(person_of(c, 4), Some(anna));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn undo_of_a_removal_restores_the_old_pin() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let f = face_on(c, 1);
            let undo = move_photos(c, anna, &[1], None)?;
            assert_eq!(pin_of(c, f), (None, false));
            restore_faces(c, &undo)?;
            assert_eq!(pin_of(c, f), (Some(anna), false));
            Ok(())
        })
        .unwrap();
    }

    /// Round 4I: 611 faces taken out of people before Round 4B are pinned to nobody with no
    /// record of whom they left. Julian chose "block the likeliest": the person they look
    /// most like is refused, then they are unpinned.
    #[test]
    fn old_removals_refuse_the_likeliest_person_and_come_back_into_play() {
        let db = library();
        db.with_conn(|c| {
            let (anna, ben) = anna_and_ben(c)?;
            // Photo 7: taken out of Anna before 4B (no rejection row). Photo 8: taken out of
            // Ben after 4B (its rejection is kept as it is). Photo 9: Not a person.
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (9, 'h9', 'p', '9.jpg', 'jpg', 0, 1, 1, 10)",
                [],
            )?;
            store_faces(c, 7, "m", &[face(near(0, 7))])?;
            store_faces(c, 8, "m", &[face(near(100, 8))])?;
            store_faces(c, 9, "m", &[face(near(0, 9))])?;
            let (f7, f8, f9) = (face_on(c, 7), face_on(c, 8), face_on(c, 9));
            c.execute("UPDATE faces SET pinned = 1 WHERE id IN (?1, ?2)", [f7, f8])?;
            c.execute("INSERT INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, 0)", [f8, ben])?;
            c.execute("UPDATE faces SET pinned = 1, rejected = 'user' WHERE id = ?1", [f9])?;

            assert_eq!(repair_orphans(c, &T, 5)?, 2);
            assert_eq!((pin_of(c, f7), refused_by(c, f7)), ((None, false), vec![anna]));
            assert_eq!((pin_of(c, f8), refused_by(c, f8)), ((None, false), vec![ben]));
            let f9_state: (bool, Option<String>) = c.query_row("SELECT pinned, rejected FROM faces WHERE id = ?1", [f9], |r| Ok((r.get::<_, i64>(0)? != 0, r.get(1)?)))?;
            assert_eq!(f9_state, (true, Some("user".into())), "Not a person is untouched");
            // Idempotent: nothing is left to repair.
            assert_eq!(repair_orphans(c, &T, 6)?, 0);
            // Neither goes back where it was taken from.
            recluster(c, &T, 7)?;
            assert_ne!(person_of(c, 7), Some(anna));
            assert_ne!(person_of(c, 8), Some(ben));
            Ok(())
        })
        .unwrap();
    }

    /// A face the user took out of someone (a rejection row, no pin) survives a rescan that
    /// does not find it again, so the rejection is not lost with it.
    #[test]
    fn a_rescan_keeps_a_face_with_a_rejection() {
        let db = library();
        db.with_conn(|c| {
            let (anna, _) = anna_and_ben(c)?;
            let f = face_on(c, 1);
            move_face(c, f, None)?;
            replace_faces_keeping_identity(c, 1, "new", &[])?;
            assert_eq!(refused_by(c, f), vec![anna]);
            Ok(())
        })
        .unwrap();
    }

    /// "This is a real face" holds through a rescan: the filter never takes it again.
    #[test]
    fn a_rescan_never_rejudges_a_restored_face() {
        let db = library();
        db.with_conn(|c| {
            store_faces(c, 1, "m", &[NewFace { rejected: Some("screen".into()), ..face(near(0, 0)) }])?;
            let f = face_on(c, 1);
            restore_face(c, f)?;
            replace_faces_keeping_identity(c, 1, "new", &[NewFace { rejected: Some("screen".into()), ..face(near(0, 1)) }])?;
            let rejected: Option<String> = c.query_row("SELECT rejected FROM faces WHERE id = ?1", [f], |r| r.get(0))?;
            assert_eq!(rejected, None);
            Ok(())
        })
        .unwrap();
    }

    /// The tests' thresholds: `MEASURED`'s shape with round numbers, so vectors built by hand
    /// stay readable (the measured values are judged on real faces, not here).
    const M: Thresholds = Thresholds { join: 0.42, margin: 0.05, suggest: 0.363, cluster: 0.45, place_quality: 0.0 };

    /// Adds photos `ids` (the test library has 1–8), each captured at `at(id)`.
    fn photos(c: &Connection, ids: std::ops::RangeInclusive<i64>, at: impl Fn(i64) -> Option<i64>) {
        for i in ids {
            c.execute(
                "INSERT OR IGNORE INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                 VALUES (?1, ?2, 'p', ?3, 'jpg', 0, 1, 1, 10)",
                params![i, format!("h{i}"), format!("{i}.jpg")],
            )
            .unwrap();
            c.execute("UPDATE assets SET captured_at = ?2 WHERE id = ?1", params![i, at(i)]).unwrap();
        }
    }

    /// Faces on photos: (photo, vector), one each.
    fn faces_on(c: &Connection, list: &[(i64, Vector)]) {
        for (asset, v) in list {
            store_faces(c, *asset, "m", &[face(v.clone())]).unwrap();
        }
    }

    /// Anna: `n` faces around axis 0 on photos 1.., placed and named.
    fn anna_with(c: &Connection, n: i64) -> i64 {
        photos(c, 1..=n, |_| None);
        faces_on(c, &(1..=n).map(|i| (i, near(0, i as usize))).collect::<Vec<_>>());
        recluster(c, &M, 0).unwrap();
        let anna = person_of(c, 1).unwrap();
        name_person(c, anna, "Anna").unwrap();
        anna
    }

    /// Round 4I, report 1: a person with one odd exemplar (a lookalike that slipped in) no
    /// longer pulls in strangers who resemble only that face: several of the person's faces
    /// have to agree.
    #[test]
    fn a_single_lookalike_exemplar_cannot_pull_a_stranger() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 6);
            photos(c, 7..=9, |_| None);
            faces_on(c, &[(7, near(200, 0))]);
            move_face(c, face_on(c, 7), Some(anna))?;
            // A stranger who looks like that face only.
            faces_on(c, &[(8, near(200, 3))]);
            recluster(c, &M, 1)?;
            assert_eq!(person_of(c, 8), None);
            Ok(())
        })
        .unwrap();
    }

    /// Several of Julian's faces at about 0.43: under 4B's 0.45 it waited to be accepted.
    #[test]
    fn a_face_close_to_several_of_a_persons_faces_joins() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 4);
            photos(c, 5..=5, |_| None);
            faces_on(c, &[(5, mix(&[(0, 0.44), (400, 0.9)]))]);
            recluster(c, &M, 1)?;
            assert_eq!(person_of(c, 5), Some(anna));
            Ok(())
        })
        .unwrap();
    }

    /// Report 1: a person is in a photo once. Two faces that both look like Anna (a mirror,
    /// a sibling) do not both become her.
    #[test]
    fn one_face_per_person_per_photo() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 4);
            photos(c, 5..=5, |_| None);
            let at = |x: f32, v: Vector| NewFace { x, ..face(v) };
            store_faces(c, 5, "m", &[at(0.1, near(0, 20)), at(0.6, near(0, 21))])?;
            recluster(c, &M, 1)?;
            let in_anna: i64 = c.query_row("SELECT COUNT(*) FROM faces WHERE asset_id = 5 AND person_id = ?1", [anna], |r| r.get(0))?;
            assert_eq!(in_anna, 1);
            Ok(())
        })
        .unwrap();
    }

    /// Report 5: new people in new photos sit in the doubtful band of someone already known
    /// (SFace puts many strangers there). A tight group of them is a new person, not a pile
    /// of suggestions for the wrong one.
    #[test]
    fn new_people_form_from_the_doubtful_band() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 4);
            photos(c, 5..=9, |_| None);
            // About 0.37 to Anna, about 0.98 to each other.
            faces_on(c, &(5..=9).map(|i| (i, mix(&[(0, 0.38), (200, 0.92), (300 + i as usize, 0.1)]))).collect::<Vec<_>>());
            recluster(c, &M, 1)?;
            let who: std::collections::HashSet<Option<i64>> = (5..=9).map(|i| person_of(c, i)).collect();
            assert_eq!(who.len(), 1, "one person");
            let new = who.into_iter().next().unwrap();
            assert!(new.is_some() && new != Some(anna));
            assert!(suggestions(c, anna, &M, 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    /// Round 4I, Task 4: a photo scanned with no face whose scene reads as people is looked at
    /// once more; its second-chance key counts as scanned, so the first pass never takes it
    /// back, and a photo with faces or with another scene is never on the list.
    #[test]
    fn photos_of_people_with_no_face_get_one_second_chance() {
        let db = library();
        db.with_conn(|c| {
            photos(c, 1..=5, |_| None);
            for (id, tags) in [(1, "party drinks"), (2, "beach"), (3, "group of people"), (4, "selfie"), (5, "portrait")] {
                crate::embedding::set_scene_tags(c, id, &tags.split(' ').map(String::from).collect::<Vec<_>>())?;
            }
            for id in 1..=4 {
                mark_scanned(c, id, "d+r4")?;
            }
            store_faces(c, 4, "d+r4", &[face(near(0, 0))])?;
            // 5 (and the library's 6–8) are not scanned yet: the first pass has them.
            assert_eq!(second_chance(c, "d+r4", 10)?, vec![3, 1]);
            assert_eq!(unscanned_count(c, "d+r4")?, 4);
            mark_scanned(c, 3, &second_chance_key("d+r4"))?;
            assert_eq!(second_chance(c, "d+r4", 10)?, vec![1]);
            assert_eq!(unscanned(c, "d+r4", 10)?.iter().map(|r| r.0).collect::<Vec<_>>(), vec![8, 7, 6, 5], "a second look counts as scanned");
            assert_eq!(unscanned_count(c, "d+r4")?, 4);
            Ok(())
        })
        .unwrap();
    }

    /// On Julian's copy, new photos of a named person grouped into a second one: as a group they
    /// were doubtful to the person (half of their photos being new), but nearly every face had
    /// one of the person's faces at a join or closer. Such a group is that person's, not new.
    #[test]
    fn a_group_of_annas_new_photos_is_not_a_second_anna() {
        let db = library();
        db.with_conn(|c| {
            photos(c, 1..=9, |_| None);
            // Anna's four faces are alike (0.61) but not near-identical.
            faces_on(c, &(1..=4).map(|i| (i, mix(&[(0, 1.0), (50 + i as usize, 0.8)]))).collect::<Vec<_>>());
            recluster(c, &M, 0)?;
            let anna = person_of(c, 1).unwrap();
            name_person(c, anna, "Anna")?;
            // Five new ones, like each other and each like Anna's second face (0.48), none
            // like the rest of her: a consensus far below a doubt.
            faces_on(c, &(5..=9).map(|i| (i, mix(&[(52, 1.2), (200, 1.0), (300 + i as usize, 0.2)]))).collect::<Vec<_>>());
            recluster(c, &M, 1)?;
            let made: i64 = c.query_row("SELECT COUNT(*) FROM people WHERE id != ?1", [anna], |r| r.get(0))?;
            assert_eq!(made, 0, "no second Anna");
            Ok(())
        })
        .unwrap();
    }

    /// A group that is nearly Anna (0.41) is asked about as Anna, not made a second Anna.
    #[test]
    fn a_doubtful_group_that_is_nearly_anna_stays_suggested() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 4);
            photos(c, 5..=7, |_| None);
            faces_on(c, &(5..=7).map(|i| (i, mix(&[(0, 0.41), (200, 0.9), (300 + i as usize, 0.1)]))).collect::<Vec<_>>());
            recluster(c, &M, 1)?;
            assert!((5..=7).all(|i| person_of(c, i).is_none()));
            assert_eq!(suggestions(c, anna, &M, 10)?.len(), 3);
            Ok(())
        })
        .unwrap();
    }

    /// Report 6: a doubtful face taken at the same party as two of Anna's placed faces is hers
    /// without asking.
    #[test]
    fn doubtful_faces_in_a_moment_with_anna_are_placed() {
        let db = library();
        db.with_conn(|c| {
            let day = 1_750_000_000;
            let anna = anna_with(c, 4);
            // Photos 1 and 2 were taken that evening; 5 an hour later; 6 a month later.
            photos(c, 1..=2, |i| Some(day + i * 600));
            photos(c, 5..=6, |i| Some(if i == 5 { day + 3600 } else { day + 30 * 86_400 }));
            let doubtful = |k: usize| mix(&[(0, 0.40), (200, 0.9), (300 + k, 0.1)]);
            faces_on(c, &[(5, doubtful(5)), (6, doubtful(6))]);
            recluster(c, &M, 1)?;
            assert_eq!(person_of(c, 5), Some(anna), "same evening");
            assert_eq!(person_of(c, 6), None, "a month later: asked about");
            Ok(())
        })
        .unwrap();
    }

    /// Faces that join Anna confidently count as her faces for the next pass, so a photo of
    /// her that looks like the new ones (not the old) joins too.
    #[test]
    fn a_face_close_to_new_faces_of_anna_joins_on_the_next_pass() {
        let db = library();
        db.with_conn(|c| {
            let anna = anna_with(c, 3);
            photos(c, 4..=7, |_| None);
            let newer = |k: usize| mix(&[(0, 0.55), (500, 0.83), (450 + k, 0.1)]);
            faces_on(c, &[(4, newer(4)), (5, newer(5)), (6, newer(6)), (7, mix(&[(0, 0.2), (500, 0.98)]))]);
            recluster(c, &M, 1)?;
            assert!((4..=7).all(|i| person_of(c, i) == Some(anna)));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn user_decisions_survive_the_new_grouping() {
        let db = library();
        db.with_conn(|c| {
            let (anna, ben) = anna_and_ben(c)?;
            // A face of Anna's the user put in Ben, and one of Ben's the user refused.
            let f1 = face_on(c, 1);
            move_face(c, f1, Some(ben))?;
            let f4 = face_on(c, 4);
            move_face(c, f4, None)?;
            recluster(c, &M, 1)?;
            assert_eq!(person_of(c, 1), Some(ben));
            assert_ne!(person_of(c, 4), Some(ben));
            let name: String = c.query_row("SELECT name FROM people WHERE id = ?1", [anna], |r| r.get(0))?;
            assert_eq!(name, "Anna");
            Ok(())
        })
        .unwrap();
    }

    /// The steady state of a big library: 45,000 faces already placed on 450 people, then an
    /// import brings 5,000 new faces (4,500 of those people, 500 of 50 new ones). SFace
    /// vectors are 128-d. The Maintenance lane may take a while, but not minutes. Release.
    #[test]
    #[ignore]
    fn recluster_50k_faces_in_under_ten_seconds() {
        const D: usize = 128;
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let (known, new_people, per) = (450usize, 50usize, 100usize);
            let total = known * per + known * 10 + new_people * 10;
            photos(c, 1..=total as i64, |_| None);
            let mut rng = 0x9e37_79b9_u64;
            let mut noise = move || {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                (rng % 1000) as f32 / 1000.0 - 0.5
            };
            // Each person is a random direction (in 128-d two of them are nearly orthogonal, as
            // SFace's strangers mostly are); each face of them is that plus noise, at about
            // 0.6–0.7 to the others, where real faces of one person sit.
            let centres: Vec<Vec<f32>> = (0..known + new_people).map(|_| (0..D).map(|_| noise()).collect()).collect();
            let mut face_of = |p: usize| {
                let v: Vec<f32> = centres[p].iter().map(|x| x + 0.7 * noise()).collect();
                quantize(&v)
            };
            let tx = c.unchecked_transaction()?;
            let mut asset = 0i64;
            let mut add = |tx: &Connection, v: Vector, person: Option<i64>| -> Result<()> {
                asset += 1;
                tx.execute(
                    "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, quality, person_id) VALUES (?1, 0.4, 0.3, 0.1, 0.1, 0.9, ?2, 'm', 0.9, ?3)",
                    params![asset, blob(&v), person],
                )?;
                Ok(())
            };
            for p in 0..known {
                tx.execute("INSERT INTO people (id, name, created_at) VALUES (?1, ?2, 0)", params![p as i64 + 1, format!("P{p}")])?;
                for _ in 0..per {
                    add(&tx, face_of(p), Some(p as i64 + 1))?;
                }
            }
            for p in 0..known {
                for _ in 0..10 {
                    add(&tx, face_of(p), None)?;
                }
            }
            for p in known..known + new_people {
                for _ in 0..10 {
                    add(&tx, face_of(p), None)?;
                }
            }
            tx.commit()?;
            let t0 = std::time::Instant::now();
            let placed = recluster(c, &M, 0)?;
            let took = t0.elapsed();
            let people: i64 = c.query_row("SELECT COUNT(*) FROM people", [], |r| r.get(0))?;
            println!("50k faces, 5,000 new: placed {placed} in {took:?}; people now {people}");
            assert!(took.as_secs_f32() < 10.0);
            Ok(())
        })
        .unwrap();
    }

    /// Replay on a COPY of a real library.db (`LUMEN_DB`): what the measured thresholds do.
    #[test]
    #[ignore]
    fn recluster_on_a_real_library() {
        let db = Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        db.with_conn(|c| {
            // Round 4I: the orphans first, as `faces::schedule` does.
            let orphans: Vec<i64> = c.prepare("SELECT id FROM faces WHERE person_id IS NULL AND pinned = 1 AND rejected IS NULL")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            let repaired = repair_orphans(c, &Thresholds::MEASURED, 0)?;
            let refused_now: i64 = c.query_row("SELECT COUNT(*) FROM face_rejections WHERE created_at = 0", [], |r| r.get(0))?;
            println!("orphans {} repaired {repaired}, refused the likeliest person for {refused_now}", orphans.len());
            let before: i64 = c.query_row("SELECT COUNT(*) FROM faces WHERE person_id IS NULL AND rejected IS NULL AND pinned = 0", [], |r| r.get(0))?;
            let t0 = std::time::Instant::now();
            let placed = recluster(c, &Thresholds::MEASURED, 0)?;
            let mut went = std::collections::BTreeMap::<&str, usize>::new();
            for id in &orphans {
                let (p, named): (Option<i64>, bool) = c.query_row("SELECT f.person_id, p.name IS NOT NULL FROM faces f LEFT JOIN people p ON p.id = f.person_id WHERE f.id = ?1", [id], |r| Ok((r.get(0)?, r.get::<_, Option<bool>>(1)?.unwrap_or(false))))?;
                *went.entry(match (p, named) { (None, _) => "still unplaced", (Some(_), true) => "joined a named person", (Some(_), false) => "joined an unnamed person" }).or_default() += 1;
            }
            println!("the orphans after recluster: {went:?}");
            let took = t0.elapsed();
            let named: Vec<i64> = c.prepare("SELECT id FROM people WHERE name IS NOT NULL AND status = 'listed'")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            let t1 = std::time::Instant::now();
            let asked: usize = named.iter().map(|p| suggestions(c, *p, &Thresholds::MEASURED, 1000).map(|s| s.len())).sum::<Result<usize>>()?;
            println!("unplaced before {before}; placed {placed} in {took:?}; suggestions over {} named people: {asked} in {:?}", named.len(), t1.elapsed());
            let new_people: i64 = c.query_row("SELECT COUNT(*) FROM people WHERE created_at = 0 AND name IS NULL", [], |r| r.get(0))?;
            println!("new unnamed people formed: {new_people}");
            if let Ok(me) = std::env::var("LUMEN_ME") {
                let photos: i64 = c.query_row("SELECT COUNT(DISTINCT asset_id) FROM faces WHERE person_id = ?1 AND rejected IS NULL", [me.parse::<i64>().unwrap()], |r| r.get(0))?;
                println!("LUMEN_ME's photos after: {photos}");
            }
            Ok(())
        })
        .unwrap();
    }

    /// Round 4I, Task 3: the grouping judged against Julian's own decisions, on a copy of
    /// `LUMEN_DB` made here (the file given is never opened for writing). Half of every named
    /// person's faces (by id) are taken out, hand-placed ones included, and every face he took
    /// out of someone loses that rejection; then `recluster` runs and each is scored:
    /// back on its own person (right), on another named person or back on the person he took
    /// it from (wrong), on an unnamed person, or unplaced. `LUMEN_JOIN` / `LUMEN_MARGIN` override.
    /// Round 5A: how long each step of `recluster` takes on a copy of `LUMEN_DB` made here.
    /// Only `apply` holds the write lock.
    ///
    /// ```sh
    /// LUMEN_DB=<copy> cargo test --release -p photos-core --lib recluster_timing_on_a_real_library -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn recluster_timing_on_a_real_library() {
        let src = std::path::PathBuf::from(std::env::var("LUMEN_DB").unwrap());
        let copy = std::env::temp_dir().join(format!("lumen-recluster-{}.db", std::process::id()));
        std::fs::copy(&src, &copy).unwrap();
        let db = Db::open(&copy).unwrap();
        let t0 = std::time::Instant::now();
        let input = db.with_conn(ReclusterInput::read).unwrap();
        let t1 = std::time::Instant::now();
        let plan = input.plan(&Thresholds::MEASURED);
        let t2 = std::time::Instant::now();
        let n = db.with_conn(|c| plan.apply(c, 0)).unwrap();
        let t3 = std::time::Instant::now();
        println!("read {:?}, plan {:?}, apply {:?} (write lock), {n} placed", t1 - t0, t2 - t1, t3 - t2);
        drop(db);
        let _ = std::fs::remove_file(&copy);
    }

    /// Round 5A: the People screen's slow calls on a copy of `LUMEN_DB` (read only).
    ///
    /// ```sh
    /// LUMEN_DB=<copy> cargo test --release -p photos-core --lib people_screen_timing_on_a_real_library -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn people_screen_timing_on_a_real_library() {
        let db = Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        db.with_conn(|c| {
            let t = std::time::Instant::now();
            let people = list_people(c)?;
            println!("list_people: {:?} ({} people)", t.elapsed(), people.len());
            for p in people.iter().take(3) {
                let t = std::time::Instant::now();
                let s = suggestions(c, p.id, &Thresholds::MEASURED, 24)?;
                println!("suggestions({}): {:?} ({} found)", p.id, t.elapsed(), s.len());
            }
            let t = std::time::Instant::now();
            let sets = crate::duplicates::find_sets_in(c, None)?;
            println!("duplicates::find_sets_in: {:?} ({} sets)", t.elapsed(), sets.len());
            let t = std::time::Instant::now();
            crate::duplicates::count_sets_in(c, None)?;
            println!("duplicates::count_sets_in: {:?}", t.elapsed());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    #[ignore]
    fn grouping_quality_on_a_real_library() {
        let src = std::path::PathBuf::from(std::env::var("LUMEN_DB").unwrap());
        let copy = std::env::temp_dir().join(format!("lumen-quality-{}.db", std::process::id()));
        std::fs::copy(&src, &copy).unwrap();
        let var = |k: &str, d: f32| std::env::var(k).map(|v| v.parse().unwrap()).unwrap_or(d);
        let m = Thresholds::MEASURED;
        let t = Thresholds { join: var("LUMEN_JOIN", m.join), margin: var("LUMEN_MARGIN", m.margin), place_quality: var("LUMEN_PLACE_QUALITY", m.place_quality), ..m };
        let db = Db::open(&copy).unwrap();
        db.with_conn(|c| {
            repair_orphans(c, &Thresholds::MEASURED, 0)?;
            let named: std::collections::HashSet<i64> = c.prepare("SELECT id FROM people WHERE name IS NOT NULL AND status = 'listed'")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            // (face, person, placed by hand)
            let positives: Vec<(i64, i64, bool)> = c
                .prepare("SELECT id, person_id, pinned FROM faces WHERE person_id IS NOT NULL AND rejected IS NULL AND id % 2 = 0")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0)))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(_, p, _)| named.contains(p))
                .collect();
            // (face, person he took it out of), for faces not placed by hand elsewhere. Only his
            // own rejections: the orphan repair (created_at 0) refused the likeliest person, and
            // lifting that would score the repair against itself.
            let negatives: Vec<(i64, i64)> = c
                .prepare("SELECT r.face_id, r.person_id FROM face_rejections r JOIN faces f ON f.id = r.face_id WHERE f.rejected IS NULL AND f.pinned = 0 AND r.created_at > 0")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|(_, p)| named.contains(p))
                .collect();
            for (f, ..) in &positives {
                c.execute("UPDATE faces SET person_id = NULL, pinned = 0 WHERE id = ?1", [f])?;
            }
            for (f, p) in &negatives {
                c.execute("DELETE FROM face_rejections WHERE face_id = ?1 AND person_id = ?2", [f, p])?;
                c.execute("UPDATE faces SET person_id = NULL WHERE id = ?1", [f])?;
            }
            let t0 = std::time::Instant::now();
            recluster(c, &t, 0)?;
            let took = t0.elapsed();
            let person_of = |f: i64| -> Result<Option<i64>> { Ok(c.query_row("SELECT person_id FROM faces WHERE id = ?1", [f], |r| r.get(0))?) };
            let (mut right, mut wrong, mut unnamed, mut left) = ([0usize; 2], 0usize, [0usize; 2], [0usize; 2]);
            for (f, p, by_hand) in &positives {
                match person_of(*f)? {
                    Some(q) if q == *p => right[*by_hand as usize] += 1,
                    Some(q) if named.contains(&q) => wrong += 1,
                    Some(q) => {
                        let fresh: bool = c.query_row("SELECT created_at = 0 FROM people WHERE id = ?1", [q], |r| r.get(0))?;
                        unnamed[fresh as usize] += 1;
                    }
                    None => left[*by_hand as usize] += 1,
                }
            }
            let mut put_back = 0;
            let mut elsewhere = 0;
            for (f, p) in &negatives {
                match person_of(*f)? {
                    Some(q) if q == *p => put_back += 1,
                    Some(q) if named.contains(&q) => elsewhere += 1,
                    _ => {}
                }
            }
            // Each person this recluster made: is it really someone named? (for the ceiling)
            {
                let pos_of: std::collections::HashMap<i64, i64> = positives.iter().map(|(f, p, _)| (*f, *p)).collect();
                let known = known_people(c)?;
                let fresh: Vec<i64> = c.prepare("SELECT id FROM people WHERE created_at = 0 AND name IS NULL")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
                for p in fresh {
                    let members: Vec<(i64, Vector)> = c.prepare("SELECT id, vec FROM faces WHERE person_id = ?1")?.query_map([p], |r| Ok((r.get(0)?, unblob(&r.get::<_, Vec<u8>>(1)?))))?.collect::<std::result::Result<_, _>>()?;
                    let others: Vec<Known> = known.iter().filter(|k| k.id != p && named.contains(&k.id)).map(|k| Known { id: k.id, evidence: k.evidence.iter().map(|e| Ev { v: e.v.clone(), norm: e.norm }).collect(), against: Vec::new(), by_hand: k.by_hand.clone(), vouch: None }).collect();
                    let none = std::collections::HashSet::new();
                    let nearest: Vec<(f32, f32, i64)> = members.iter().map(|(_, v)| {
                        let likes = likeness_to_all(&Ev::new(v.clone()), &others, t.suggest);
                        let c = candidates(&likes, &others, &none, &none);
                        let best = likes.iter().map(|(_, l)| l.top[0]).fold(0.0f32, f32::max);
                        c.first().map(|x| (x.1, best, others[x.0].id)).unwrap_or((0.0, best, 0))
                    }).collect();
                    let mean = nearest.iter().map(|x| x.0).sum::<f32>() / nearest.len() as f32;
                    let top = nearest.iter().map(|x| x.1).fold(0.0f32, f32::max);
                    let n_top = nearest.iter().filter(|x| x.1 >= t.join).count();
                    let really: Vec<i64> = members.iter().filter_map(|(f, _)| pos_of.get(f).copied()).collect();
                    let target = nearest.iter().map(|x| x.2).max_by_key(|id| nearest.iter().filter(|y| y.2 == *id).count()).unwrap_or(0);
                    println!("  new person {p}: {} faces, {} really named ({}), mean {mean:.3}, best pair {top:.3}, members with a pair ≥ join {n_top}, nearest mostly {} ", members.len(), really.len(), if really.iter().all(|r| *r == target) && !really.is_empty() { "same as nearest" } else { "-" }, target);
                }
            }
            let auto = positives.iter().filter(|x| !x.2).count();
            let hand = positives.len() - auto;
            let placed_named = right[0] + right[1] + wrong + put_back;
            println!("join {:.3} margin {:.3}: recluster {took:?}", t.join, t.margin);
            println!("  taken out: {auto} placed by Lumen, {hand} by hand; {} faces he removed", negatives.len());
            println!("  recall: {}/{auto} of Lumen's ({:.1} %), {}/{hand} of his own ({:.1} %)", right[0], 100.0 * right[0] as f32 / auto.max(1) as f32, right[1], 100.0 * right[1] as f32 / hand.max(1) as f32);
            println!("  wrong: {wrong} on another named person, {put_back} back where he removed them; {elsewhere} removed faces on another named person (unjudged)");
            println!("  precision: {:.2} % ({} right of {placed_named} placed on named people)", 100.0 * (right[0] + right[1]) as f32 / placed_named.max(1) as f32, right[0] + right[1]);
            println!("  on an unnamed person: {} already there, {} made by this recluster; unplaced: {} of Lumen's, {} of his", unnamed[0], unnamed[1], left[0], left[1]);
            Ok(())
        })
        .unwrap();
        drop(db);
        for ext in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{ext}", copy.display()));
        }
    }

    // Round 4I, Task 6: tagging by hand.

    fn drawn(asset: i64, v: Vector, quality: f32) -> ManualFace {
        ManualFace { asset_id: asset, x: 0.6, y: 0.2, w: 0.1, h: 0.1, score: 0.0, quality, vec: v }
    }

    fn anna(c: &Connection) -> i64 {
        for (asset, k) in [(1, 0), (2, 1), (3, 2)] {
            store_faces(c, asset, "m", &[face(near(0, k))]).unwrap();
        }
        recluster(c, &Thresholds::uniform(0.6), 0).unwrap();
        let p = person_of(c, 1).unwrap();
        name_person(c, p, "Anna").unwrap();
        p
    }

    fn photos_of(c: &Connection, person: i64) -> Vec<i64> {
        let scope = crate::scope::LayoutScope { person_id: Some(person), ..Default::default() };
        let (from_where, order, params) = scope.sql();
        let mut stmt = c.prepare(&format!("SELECT a.id {from_where} ORDER BY {order}")).unwrap();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| r.get(0)).unwrap();
        let mut ids: Vec<i64> = rows.collect::<std::result::Result<_, _>>().unwrap();
        ids.sort();
        ids
    }

    #[test]
    fn a_drawn_box_is_never_an_exemplar() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            // A box around the back of a head: embedded as it is, it looks like a stranger's
            // face (axis 9). It must never pull that stranger into Anna.
            add_manual_face(c, &drawn(4, near(9, 0), 0.0), a, None)?;
            store_faces(c, 5, "m", &[face(near(9, 1))])?;
            recluster(c, &Thresholds::uniform(0.6), 0)?;
            assert_ne!(person_of(c, 5), Some(a));
            assert!(known_people(c)?.iter().find(|k| k.id == a).unwrap().evidence.len() == 3, "only the three scanned faces stand for Anna");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_manual_face_counts_for_the_persons_photos() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            add_manual_face(c, &drawn(6, near(9, 0), 0.0), a, None)?;
            assert_eq!(photos_of(c, a), vec![1, 2, 3, 6]);
            assert_eq!(names_in(c, 6)?, vec!["Anna".to_string()]);
            // A recluster leaves it where the user put it.
            recluster(c, &Thresholds::uniform(0.6), 0)?;
            assert_eq!(photos_of(c, a), vec![1, 2, 3, 6]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn untag_deletes_a_manual_face_and_rejects_a_scanned_one() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            let drawn_id = add_manual_face(c, &drawn(6, near(9, 0), 0.0), a, None)?;
            let undo = untag_face(c, drawn_id)?;
            assert!(undo.manual.is_some());
            assert_eq!(c.query_row("SELECT COUNT(*) FROM faces WHERE id = ?1", [drawn_id], |r| r.get::<_, i64>(0))?, 0);
            assert_eq!(photos_of(c, a), vec![1, 2, 3]);
            undo_untag_face(c, &undo)?;
            assert_eq!(photos_of(c, a), vec![1, 2, 3, 6], "Undo brings the drawn face back");
            assert_eq!(c.query_row("SELECT source FROM faces WHERE id = ?1", [drawn_id], |r| r.get::<_, String>(0))?, "manual");

            let scanned: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 2", [], |r| r.get(0))?;
            let undo = untag_face(c, scanned)?;
            assert!(undo.manual.is_none());
            let (person, pinned): (Option<i64>, bool) = c.query_row("SELECT person_id, pinned FROM faces WHERE id = ?1", [scanned], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((person, pinned), (None, false), "back in play, not pinned to nobody");
            assert!(refusals(c)?.get(&scanned).is_some_and(|p| p.contains(&a)));
            recluster(c, &Thresholds::uniform(0.6), 0)?;
            assert_ne!(person_of(c, 2), Some(a), "never offered back to Anna");
            undo_untag_face(c, &undo)?;
            assert_eq!(person_of(c, 2), Some(a));
            Ok(())
        })
        .unwrap();
    }

    /// Round 4G: *Not a face* on a face box in the viewer.
    #[test]
    fn not_a_face_rejects_it_and_undo_puts_it_back() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 2", [], |r| r.get(0))?;
            let place = reject_face(c, f)?;
            assert_eq!(place.person, Some(a));
            let (person, rejected): (Option<i64>, Option<String>) = c.query_row("SELECT person_id, rejected FROM faces WHERE id = ?1", [f], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((person, rejected.as_deref()), (None, Some("user")));
            assert_eq!(photos_of(c, a), vec![1, 3]);
            recluster(c, &Thresholds::uniform(0.6), 0)?;
            let regrouped: Option<i64> = c.query_row("SELECT person_id FROM faces WHERE id = ?1", [f], |r| r.get(0))?;
            assert_eq!(regrouped, None, "a rejected face is never grouped");
            undo_reject_face(c, &place)?;
            assert_eq!(person_of(c, 2), Some(a));
            let rejected: Option<String> = c.query_row("SELECT rejected FROM faces WHERE id = ?1", [f], |r| r.get(0))?;
            assert_eq!(rejected, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn create_person_with_face_names_them_and_pins_the_face() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 3", [], |r| r.get(0))?;
            let (ben, place) = create_person_with_face(c, f, "Ben", 5)?;
            assert_eq!(place, FacePlace { face: f, person: Some(a), pinned: false });
            let (name, cover): (String, i64) = c.query_row("SELECT name, cover_face_id FROM people WHERE id = ?1", [ben], |r| Ok((r.get(0)?, r.get(1)?)))?;
            assert_eq!((name.as_str(), cover), ("Ben", f));
            let pinned: bool = c.query_row("SELECT pinned FROM faces WHERE id = ?1", [f], |r| r.get(0))?;
            assert!(pinned);
            assert_eq!(names_in(c, 3)?, vec!["Ben".to_string()]);
            assert!(create_person(c, "  ", 0).is_err(), "a person needs a name");
            undo_create_person(c, ben, &place)?;
            assert_eq!(person_of(c, 3), Some(a));
            assert_eq!(c.query_row("SELECT COUNT(*) FROM people WHERE id = ?1", [ben], |r| r.get::<_, i64>(0))?, 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tagging_a_face_someone_refused_places_it_and_keeps_the_refusal() {
        let db = library();
        db.with_conn(|c| {
            let a = anna(c);
            store_faces(c, 7, "m", &[face(near(9, 0))])?;
            let f: i64 = c.query_row("SELECT id FROM faces WHERE asset_id = 7", [], |r| r.get(0))?;
            let ben = create_person(c, "Ben", 0)?;
            c.execute("INSERT INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, 0)", params![f, ben])?;
            let place = tag_face(c, f, a)?;
            assert_eq!(place.person, None);
            assert_eq!(person_of(c, 7), Some(a));
            assert!(refusals(c)?.get(&f).is_some_and(|p| p.contains(&ben)));
            restore_faces(c, &[place])?;
            assert_eq!(person_of(c, 7), None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_tiny_second_look_face_is_suggested_but_never_placed_or_made_a_person() {
        let db = library();
        let t = Thresholds { place_quality: 0.3, ..Thresholds::uniform(0.6) };
        db.with_conn(|c| {
            for (asset, k) in [(1, 0), (2, 1), (3, 2)] {
                store_faces(c, asset, "m", &[face(near(0, k))])?;
            }
            recluster(c, &t, 0)?;
            let anna = person_of(c, 1).unwrap();
            name_person(c, anna, "Anna")?;
            // Anna, at the size floor, and three tiny strangers who would make a new person.
            let tiny = |v| NewFace { quality: 0.22, ..face(v) };
            store_second_look(c, 4, "mi", &[tiny(near(0, 3))])?;
            for (asset, k) in [(5, 0), (6, 1), (7, 2)] {
                store_second_look(c, asset, "mi", &[tiny(near(40, k))])?;
            }
            // The first pass's tiny faces are placed as before.
            store_faces(c, 8, "m", &[tiny(near(0, 4))])?;
            recluster(c, &t, 0)?;
            for asset in 4..=7 {
                assert_eq!(person_of(c, asset), None, "photo {asset}");
            }
            assert_eq!(person_of(c, 8), Some(anna));
            let s = suggestions(c, anna, &t, 10)?;
            assert_eq!(s.iter().map(|x| x.asset_id).collect::<Vec<_>>(), vec![4], "Anna's tiny face is asked about");
            // Round 7: evidence and open faces now come from one read of the faces; the
            // grouping input built from it is the one the old reads built.
            let old = (known_from(evidence_rows(c)?), refusals(c)?);
            let rows = GroupingRows::read_faces(c)?;
            let new = known_from(rows.evidence);
            assert_eq!(new.iter().map(|k| (k.id, k.evidence.len())).collect::<Vec<_>>(), old.0.iter().map(|k| (k.id, k.evidence.len())).collect::<Vec<_>>());
            assert_eq!(refusals_from(rows.refusals), old.1);
            Ok(())
        })
        .unwrap();
        let anna = db.with_conn(|c| Ok(person_of(c, 1))).unwrap().unwrap();
        assert_eq!(suggestions_on(&db, anna, &t, 10).unwrap(), db.with_conn(|c| suggestions(c, anna, &t, 10)).unwrap());
    }
}
