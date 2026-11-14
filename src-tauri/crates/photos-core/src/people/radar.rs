//! The face radar (Round 9A, reworked in Round 10).
//!
//! When the user corrects a face, the photos they are looking at are checked again for the
//! people the correction touched: the grid the viewer was opened from, with its filter, nearest
//! to the open photo first (`Scope::View`). Without a view (a correction on the People page)
//! it checks the photos taken around the correction instead, outward by date in rings (±1 day,
//! ±7, ±30, ±365), stopping at a ring that changed nothing (`Scope::Around`).
//!
//! Both ways for each person:
//! - **gain**: a face the person now wins by People's join and margin moves to them (a face on
//!   nobody joins the same way);
//! - **lose** (Round 10): a face Lumen put on the person that is more like a face the user
//!   refused for them than like them goes to whoever else clearly wins it, or to nobody. Before,
//!   *Not Ruta* on one photo left every other photo of that look-alike on Ruta.
//!
//! Only faces Lumen placed itself move: never one the user pinned, never one refused for the
//! person, never one judged not a face. Stored vectors only: nothing is decoded, and every read
//! is its own short query.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{against_rows, candidates, clears_for, evidence_rows, fix_covers, known_from, refusal_rows, refusals_from, unblob, vetoed, with_against, Ev, FacePlace, Known, Likeness, Refused, Thresholds, LIKENESS_FLOOR};
use crate::{Db, Result};

/// The rings' outer edges, seconds from the nearest correction.
pub const RINGS_S: [i64; 4] = [86_400, 7 * 86_400, 30 * 86_400, 365 * 86_400];
/// At most this many photos of a view are checked, the nearest to the open photo.
pub const VIEW_MAX: usize = 5_000;
/// Photos read per query.
const CHUNK: usize = 400;

/// Which photos a sweep checks.
#[derive(Debug, Clone, Copy)]
pub enum Scope<'a> {
    /// The photos the user is looking at, in the order shown; nearest to `at` first.
    View { ids: &'a [i64], at: Option<i64> },
    /// Photos captured around these times (the corrections'), in rings.
    Around(&'a [i64]),
}

/// One sweep's moves, for Undo (`restore_faces`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RadarUndo {
    #[ts(type = "number")]
    pub person: i64,
    pub moves: Vec<FacePlace>,
    /// The photos the moved faces are in.
    #[ts(type = "number[]")]
    pub assets: Vec<i64>,
    /// Faces the person gained, and faces taken off them.
    pub added: u32,
    pub removed: u32,
}

impl RadarUndo {
    fn new(person: i64) -> Self {
        Self { person, moves: Vec::new(), assets: Vec::new(), added: 0, removed: 0 }
    }

    /// How many photos changed: *Fixed 7 more photos of Anna*.
    pub fn photos(&self) -> usize {
        self.assets.iter().collect::<HashSet<_>>().len()
    }
}

/// A face as the radar reads it.
struct Seen {
    id: i64,
    asset: i64,
    person: Option<i64>,
    pinned: bool,
    second_low: bool,
    at: Option<i64>,
    v: Ev,
}

const SEEN: &str = "SELECT f.id, f.asset_id, f.person_id, f.pinned, f.vec, f.source = 'second' AND COALESCE(f.quality, f.score) < ?1, a.captured_at
     FROM assets a JOIN faces f ON f.asset_id = a.id
     WHERE a.state = 0 AND f.rejected IS NULL";

fn seen(r: &rusqlite::Row) -> rusqlite::Result<Seen> {
    Ok(Seen {
        id: r.get(0)?,
        asset: r.get(1)?,
        person: r.get(2)?,
        pinned: r.get::<_, i64>(3)? != 0,
        v: Ev::new(unblob(&r.get::<_, Vec<u8>>(4)?)),
        second_low: r.get(5)?,
        at: r.get(6)?,
    })
}

/// Every face not judged a non-face in the photos captured in `lo..=hi` (not in the trash).
fn ring_rows(conn: &Connection, lo: i64, hi: i64, place_quality: f32) -> Result<Vec<Seen>> {
    let mut stmt = conn.prepare_cached(&format!("{SEEN} AND a.captured_at BETWEEN ?2 AND ?3"))?;
    let rows = stmt.query_map(params![place_quality as f64, lo, hi], seen)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every face not judged a non-face in these photos (not in the trash).
fn faces_in(conn: &Connection, assets: &[i64], place_quality: f32) -> Result<Vec<Seen>> {
    if assets.is_empty() {
        return Ok(Vec::new());
    }
    let list = assets.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(",");
    let mut stmt = conn.prepare(&format!("{SEEN} AND a.id IN ({list})"))?;
    let rows = stmt.query_map(params![place_quality as f64], seen)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// `ids` from the one at `at` outward, alternating after and before: what the user sees
/// around the open photo comes first. At most `max`.
pub fn nearest_first(ids: &[i64], at: Option<i64>, max: usize) -> Vec<i64> {
    let centre = at.and_then(|a| ids.iter().position(|&i| i == a)).unwrap_or(0);
    let mut out = Vec::with_capacity(ids.len().min(max));
    let mut step = 0usize;
    while out.len() < max && (centre + step < ids.len() || step <= centre) {
        if let Some(&i) = ids.get(centre + step) {
            out.push(i);
        }
        if step > 0 && step <= centre && out.len() < max {
            out.push(ids[centre - step]);
        }
        step += 1;
    }
    out
}

/// A face's likeness to one person, its own vector left out: a face Lumen put on someone is
/// among their evidence, and would otherwise vouch for itself.
fn likeness(f: &Ev, person: &Known, suggest: f32) -> Likeness {
    let mut l = Likeness::new();
    for e in person.evidence.iter().filter(|e| e.v != f.v) {
        l.add(f.cos(e), suggest);
    }
    l
}

/// What People knows, read once per sweep.
struct Judge<'a> {
    person: i64,
    target: usize,
    known: &'a [Known],
    refused: &'a Refused,
    t: &'a Thresholds,
}

impl Judge<'_> {
    /// Who clearly wins `f` among everyone but `skip`, with the refusals as evidence.
    fn winner(&self, f: &Seen, skip: Option<usize>, blocked: &HashSet<i64>, mine: Option<Likeness>) -> Option<usize> {
        let none = HashSet::new();
        let likes: Vec<(usize, Likeness)> = self
            .known
            .iter()
            .enumerate()
            .filter(|(k, _)| Some(*k) != skip)
            .map(|(k, p)| (k, if k == self.target { mine.unwrap_or_else(|| likeness(&f.v, p, self.t.suggest)) } else { likeness(&f.v, p, self.t.suggest) }))
            .filter(|(_, l)| l.top[0] >= LIKENESS_FLOOR)
            .collect();
        let c = candidates(&likes, self.known, self.refused.get(&f.id).unwrap_or(&none), blocked);
        clears_for(&f.v, c, self.known, self.t, 0.0).map(|(k, _)| k)
    }

    /// Where `f` goes, when it moves: `Some(None)` is off the person, to nobody.
    fn decide(&self, f: &Seen, in_photo: &HashMap<i64, HashSet<i64>>) -> Option<Option<i64>> {
        if f.pinned {
            return None;
        }
        let blocked: HashSet<i64> = in_photo.get(&f.asset).map(|s| s.iter().copied().filter(|p| Some(*p) != f.person).collect()).unwrap_or_default();
        let target = &self.known[self.target];
        if f.person == Some(self.person) {
            // Lose: more like a face refused for them than like them.
            if !vetoed(&f.v, target, self.t) {
                return None;
            }
            return Some(self.winner(f, Some(self.target), &blocked, None).map(|k| self.known[k].id));
        }
        // Gain: the person now wins it by the join and the margin.
        if f.second_low || blocked.contains(&self.person) || self.refused.get(&f.id).is_some_and(|r| r.contains(&self.person)) {
            return None;
        }
        // Most faces are nowhere near the person: one comparison settles them.
        let mine = likeness(&f.v, target, self.t.suggest);
        if mine.score() < self.t.join {
            return None;
        }
        (self.winner(f, None, &blocked, Some(mine)) == Some(self.target)).then_some(Some(self.person))
    }

    /// The moves among the `rows` that `may` move, one after another: every face blocks its
    /// photo's people, and a face placed blocks its person there for the next. (face, photo,
    /// from, to)
    fn moves(&self, rows: &[Seen], may: impl Fn(&Seen) -> bool) -> Vec<(i64, i64, Option<i64>, Option<i64>)> {
        let mut in_photo: HashMap<i64, HashSet<i64>> = HashMap::new();
        for f in rows {
            if let Some(p) = f.person {
                in_photo.entry(f.asset).or_default().insert(p);
            }
        }
        let mut out = Vec::new();
        for f in rows.iter().filter(|f| may(f)) {
            let Some(to) = self.decide(f, &in_photo) else { continue };
            if let Some(p) = f.person {
                if let Some(s) = in_photo.get_mut(&f.asset) {
                    s.remove(&p);
                }
            }
            if let Some(p) = to {
                in_photo.entry(f.asset).or_default().insert(p);
            }
            out.push((f.id, f.asset, f.person, to));
        }
        out
    }
}

/// Checks `scope` again for `person`, both ways, and moves what it may. Returns every move,
/// for one Undo.
pub fn sweep(db: &Db, person: i64, scope: Scope<'_>, t: &Thresholds) -> Result<RadarUndo> {
    let mut undo = RadarUndo::new(person);
    let (evidence, refusals, against) = db.with_conn(|c| Ok((evidence_rows(c)?, refusal_rows(c)?, against_rows(c)?)))?;
    let known = with_against(known_from(evidence), against);
    let refused = refusals_from(refusals);
    let Some(target) = known.iter().position(|k| k.id == person) else { return Ok(undo) };
    let judge = Judge { person, target, known: &known, refused: &refused, t };
    let mut take = |done: Vec<(FacePlace, i64, Option<i64>)>| {
        for (place, asset, to) in done {
            if to == Some(person) {
                undo.added += 1;
            } else {
                undo.removed += 1;
            }
            undo.moves.push(place);
            undo.assets.push(asset);
        }
    };
    match scope {
        Scope::View { ids, at } => {
            for chunk in nearest_first(ids, at, VIEW_MAX).chunks(CHUNK) {
                let rows = db.with_conn(|c| faces_in(c, chunk, t.place_quality))?;
                let moves = judge.moves(&rows, |_| true);
                take(db.with_conn(|c| apply(c, &moves))?);
            }
        }
        Scope::Around(seeds) => {
            let (Some(&first), Some(&last)) = (seeds.iter().min(), seeds.iter().max()) else { return Ok(undo) };
            let distance = |at: Option<i64>| at.map(|at| seeds.iter().map(|s| (at - s).abs()).min().unwrap_or(i64::MAX)).unwrap_or(i64::MAX);
            let mut inner = -1;
            for outer in RINGS_S {
                let rows = db.with_conn(|c| ring_rows(c, first - outer, last + outer, t.place_quality))?;
                // Only the new band moves.
                let moves = judge.moves(&rows, |f| {
                    let d = distance(f.at);
                    d > inner && d <= outer
                });
                let done = db.with_conn(|c| apply(c, &moves))?;
                if done.is_empty() {
                    break;
                }
                take(done);
                inner = outer;
            }
        }
    }
    Ok(undo)
}

/// Moves `(face, photo, from, to)`, unpinned, in one short transaction. A face the user
/// touched since it was read (now pinned, elsewhere, judged) is left alone.
fn apply(conn: &Connection, moves: &[(i64, i64, Option<i64>, Option<i64>)]) -> Result<Vec<(FacePlace, i64, Option<i64>)>> {
    if moves.is_empty() {
        return Ok(Vec::new());
    }
    let tx = conn.unchecked_transaction()?;
    let mut done = Vec::new();
    let mut people: Vec<Option<i64>> = Vec::new();
    for &(face, asset, from, to) in moves {
        let n = tx.execute(
            "UPDATE faces SET person_id = ?2 WHERE id = ?1 AND pinned = 0 AND rejected IS NULL AND person_id IS ?3",
            params![face, to, from],
        )?;
        if n == 1 {
            done.push((FacePlace { face, person: from, pinned: false }, asset, to));
            people.extend([from, to]);
        }
    }
    people.sort();
    people.dedup();
    fix_covers(&tx, &people)?;
    let assets: Vec<i64> = done.iter().map(|(_, a, _)| *a).collect();
    crate::fts::index_ids(&tx, &assets)?;
    tx.commit()?;
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{quantize, Vector, DIM};
    use crate::people::{blob, move_face, restore_faces, tests::lookalike, untag_face};
    use crate::Db;

    const T: Thresholds = Thresholds { join: 0.35, margin: 0.05, suggest: 0.3, cluster: 0.35, place_quality: 0.3 };
    const DAY: i64 = 86_400;
    const ANNA: i64 = 1;
    const BEA: i64 = 2;

    /// A unit vector near `axis`, jittered by `k`.
    fn near(axis: usize, k: usize) -> Vector {
        let mut v = vec![0f32; DIM];
        v[axis] = 1.0;
        v[(axis + 7 + k) % DIM] = 0.15;
        quantize(&v)
    }

    fn library() -> Db {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute("INSERT INTO people (id, name, created_at, status) VALUES (1, 'Anna', 0, 'listed'), (2, 'Bea', 0, 'listed')", [])?;
            crate::fts::rebuild(c)?;
            Ok(())
        })
        .unwrap();
        db
    }

    /// A photo taken at `at` holding one face: on `person`, pinned or not. Returns the face.
    fn put(db: &Db, asset: i64, at: i64, person: Option<i64>, pinned: bool, v: Vector) -> i64 {
        db.with_conn(|c| {
            c.execute(
                "INSERT OR IGNORE INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at)
                 VALUES (?1, ?2, 'p', ?3, 'jpg', 0, 1, 1, 10, ?4)",
                params![asset, format!("h{asset}"), format!("{asset}.jpg"), at],
            )?;
            c.execute(
                "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id, pinned, quality) VALUES (?1, 0.4, 0.3, 0.1, 0.15, 0.9, ?2, 'm', ?3, ?4, 0.9)",
                params![asset, blob(&v), person, pinned],
            )?;
            Ok(c.last_insert_rowid())
        })
        .unwrap()
    }

    fn place(db: &Db, face: i64) -> (Option<i64>, bool) {
        db.with_conn(|c| Ok(c.query_row("SELECT person_id, pinned FROM faces WHERE id = ?1", [face], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap()
    }

    /// Anna's faces around day 0 (two named by hand), Bea's far away at day 1000.
    fn anna_and_bea(db: &Db) {
        for k in 0..4 {
            put(db, 1 + k, k * 3600, Some(ANNA), k < 2, near(0, k as usize));
            put(db, 11 + k, 1000 * DAY + k, Some(BEA), false, near(100, k as usize));
        }
    }

    fn around(seeds: &[i64]) -> Scope<'_> {
        Scope::Around(seeds)
    }

    #[test]
    fn a_correction_fixes_the_faces_lumen_placed_around_it_and_undo_restores_them() {
        let db = library();
        anna_and_bea(&db);
        // Lumen put this face of Anna on Bea, and left this one on nobody.
        let wrong = put(&db, 20, 2 * 3600, Some(BEA), false, near(0, 20));
        let open = put(&db, 21, DAY / 2, None, false, near(0, 21));
        // Julian put this one on Bea himself, and said this one is not Anna.
        let pinned = put(&db, 22, 3 * 3600, Some(BEA), true, near(0, 22));
        let refused = put(&db, 23, 4 * 3600, None, false, near(0, 23));
        db.with_conn(|c| Ok(c.execute("INSERT INTO face_rejections (face_id, person_id, created_at) VALUES (?1, ?2, 0)", params![refused, ANNA])?)).unwrap();
        // A face Lumen marked as not a face at all.
        let not_a_face = put(&db, 24, 5 * 3600, None, false, near(0, 24));
        db.with_conn(|c| Ok(c.execute("UPDATE faces SET rejected = 'tiny' WHERE id = ?1", [not_a_face])?)).unwrap();
        // Far away in time: the radar never gets there.
        let far = put(&db, 25, 800 * DAY, None, false, near(0, 25));

        // Julian moves a face of Anna's taken at day 0 onto her.
        let seed = put(&db, 30, 0, Some(BEA), false, near(0, 30));
        db.with_conn(|c| move_face(c, seed, Some(ANNA))).unwrap();
        let undo = sweep(&db, ANNA, around(&[0]), &T).unwrap();

        assert_eq!(place(&db, wrong), (Some(ANNA), false), "fixed, still Lumen's to move again");
        assert_eq!(place(&db, open), (Some(ANNA), false));
        assert_eq!(place(&db, pinned), (Some(BEA), true), "what Julian placed stays");
        assert_eq!(place(&db, refused), (None, false));
        assert_eq!(place(&db, not_a_face), (None, false));
        assert_eq!(place(&db, far), (None, false));
        assert_eq!(undo.person, ANNA);
        let mut moved: Vec<i64> = undo.moves.iter().map(|m| m.face).collect();
        moved.sort();
        assert_eq!(moved, vec![wrong, open]);
        assert_eq!((undo.photos(), undo.added, undo.removed), (2, 2, 0));

        db.with_conn(|c| restore_faces(c, &undo.moves)).unwrap();
        assert_eq!(place(&db, wrong), (Some(BEA), false));
        assert_eq!(place(&db, open), (None, false));
        assert_eq!(place(&db, seed), (Some(ANNA), true), "Undo of the sweep leaves the correction itself");
    }

    #[test]
    fn not_her_takes_her_off_the_look_alikes_lumen_placed() {
        let db = library();
        anna_and_bea(&db);
        // Lumen put four photos of a look-alike on Anna (they vouch for each other: only her
        // faces placed by hand count against a refusal); Julian put one on Bea himself.
        let lookalikes: Vec<i64> = (0..4).map(|k| put(&db, 40 + k, (5 + k) * 3600, Some(ANNA), false, lookalike(0, k as usize))).collect();
        let his = put(&db, 45, 10 * 3600, Some(BEA), true, lookalike(0, 9));
        // And one of Bea's look-alikes too, which Bea clearly wins.
        let beas = put(&db, 46, 11 * 3600, Some(ANNA), false, {
            let mut v = vec![0f32; DIM];
            v[100] = 1.0;
            v[0] = 0.25;
            quantize(&v)
        });
        // He says the first is not Anna.
        db.with_conn(|c| untag_face(c, lookalikes[0])).unwrap();

        let ids: Vec<i64> = (1..=60).collect();
        let undo = sweep(&db, ANNA, Scope::View { ids: &ids, at: Some(40) }, &T).unwrap();
        for &f in &lookalikes[1..] {
            assert_eq!(place(&db, f), (None, false), "off Anna, to nobody: no one else wins it");
        }
        assert_eq!(place(&db, his), (Some(BEA), true), "what he placed stays");
        assert_eq!(place(&db, beas).0, Some(ANNA), "not like the refused face: it stays where it is");
        let annas: Vec<i64> = db.with_conn(|c| Ok(c.prepare("SELECT id FROM faces WHERE asset_id BETWEEN 1 AND 4")?.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)).unwrap();
        for f in annas {
            assert_eq!(place(&db, f).0, Some(ANNA), "Anna's own photos stay hers");
        }
        assert_eq!((undo.added, undo.removed), (0, 3));
        db.with_conn(|c| restore_faces(c, &undo.moves)).unwrap();
        assert!(lookalikes[1..].iter().all(|&f| place(&db, f) == (Some(ANNA), false)));
    }

    #[test]
    fn a_view_checks_the_photos_on_screen_and_nothing_else() {
        let db = library();
        anna_and_bea(&db);
        // In the view, two years away from any correction; and next to it in time, but filtered out.
        let shown_far = put(&db, 20, 700 * DAY, None, false, near(0, 20));
        let hidden_near = put(&db, 21, 3600, None, false, near(0, 21));
        let ids = [1, 2, 3, 4, 20];
        let undo = sweep(&db, ANNA, Scope::View { ids: &ids, at: Some(1) }, &T).unwrap();
        assert_eq!(place(&db, shown_far).0, Some(ANNA));
        assert_eq!(place(&db, hidden_near).0, None);
        assert_eq!(undo.moves.len(), 1);
    }

    #[test]
    fn the_view_is_checked_from_the_open_photo_outward() {
        let ids: Vec<i64> = (10..20).collect();
        assert_eq!(nearest_first(&ids, Some(13), 100), vec![13, 14, 12, 15, 11, 16, 10, 17, 18, 19]);
        assert_eq!(nearest_first(&ids, Some(13), 3), vec![13, 14, 12]);
        assert_eq!(nearest_first(&ids, None, 3), vec![10, 11, 12], "no open photo: from the top");
        assert_eq!(nearest_first(&ids, Some(19), 4), vec![19, 18, 17, 16]);
        assert!(nearest_first(&[], Some(1), 4).is_empty());
    }

    #[test]
    fn the_radar_stops_at_a_ring_that_changed_nothing() {
        let db = library();
        anna_and_bea(&db);
        // One to fix within a day; nothing between 1 and 7 days; one at 20 days.
        let near_one = put(&db, 20, 3600, None, false, near(0, 20));
        let later = put(&db, 21, 20 * DAY, None, false, near(0, 21));
        let undo = sweep(&db, ANNA, around(&[0]), &T).unwrap();
        assert_eq!(place(&db, near_one).0, Some(ANNA));
        assert_eq!(place(&db, later).0, None, "the ±7-day ring changed nothing: it stopped there");
        assert_eq!(undo.moves.len(), 1);
    }

    #[test]
    fn several_corrections_sweep_once_around_each() {
        let db = library();
        anna_and_bea(&db);
        let a = put(&db, 20, 3600, None, false, near(0, 20));
        let b = put(&db, 21, 400 * DAY + 3600, None, false, near(0, 21));
        let undo = sweep(&db, ANNA, around(&[0, 400 * DAY]), &T).unwrap();
        assert_eq!((place(&db, a).0, place(&db, b).0), (Some(ANNA), Some(ANNA)));
        assert_eq!(undo.moves.len(), 2, "one Undo for both");
    }

    #[test]
    fn a_face_is_not_moved_onto_someone_already_in_its_photo() {
        let db = library();
        anna_and_bea(&db);
        // Photo 1 holds Anna already; a second Anna-like face there stays where it is.
        let twin = db
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id, pinned, quality) VALUES (1, 0.8, 0.3, 0.1, 0.15, 0.9, ?1, 'm', ?2, 0, 0.9)",
                    params![blob(&near(0, 40)), BEA],
                )?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();
        sweep(&db, ANNA, around(&[0]), &T).unwrap();
        assert_eq!(place(&db, twin), (Some(BEA), false));
    }

    #[test]
    fn a_face_that_still_looks_more_like_its_person_stays() {
        let db = library();
        anna_and_bea(&db);
        let bea_near_anna_in_time = put(&db, 20, 3600, Some(BEA), false, near(100, 20));
        let undo = sweep(&db, ANNA, around(&[0]), &T).unwrap();
        assert_eq!(place(&db, bea_near_anna_in_time).0, Some(BEA));
        assert!(undo.moves.is_empty());
    }

    /// The plan's replay on a COPY of a real library.db: 20 of the user's placements (pinned
    /// faces, spread over the library), each swept as if just made, then undone; and 20 of
    /// their refusals, swept over the whole library as a view. Prints what each would move,
    /// for the user to check a sample.
    ///
    /// LUMEN_DB=<copy> cargo test --release -p photos-core --lib radar_on_a_copy -- --ignored --nocapture
    #[test]
    #[ignore = "needs a copy of a real library.db"]
    fn radar_on_a_copy() {
        let db = Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
        let rows = |sql: &str| -> Vec<(i64, i64, i64)> {
            db.with_conn(|c| {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .unwrap()
        };
        let pinned = rows(
            "SELECT f.id, f.person_id, a.captured_at FROM faces f JOIN assets a ON a.id = f.asset_id
             WHERE f.pinned = 1 AND f.person_id IS NOT NULL AND a.captured_at IS NOT NULL AND a.state = 0 ORDER BY f.id",
        );
        let refusals = rows(
            "SELECT r.face_id, r.person_id, f.asset_id FROM face_rejections r JOIN faces f ON f.id = r.face_id JOIN people p ON p.id = r.person_id
             WHERE p.name IS NOT NULL AND p.status = 'listed' ORDER BY r.created_at DESC",
        );
        let timeline: Vec<i64> = db
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT id FROM assets WHERE state = 0 ORDER BY COALESCE(captured_at, imported_at) DESC, id DESC")?;
                let ids = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
                Ok(ids)
            })
            .unwrap();
        println!("{} pinned faces with a date, {} refusals of named people, {} photos", pinned.len(), refusals.len(), timeline.len());
        let name = |p: Option<i64>| -> String {
            p.and_then(|p| db.with_conn(|c| Ok(c.query_row("SELECT COALESCE(name, '#' || id) FROM people WHERE id = ?1", [p], |r| r.get::<_, String>(0))?)).ok())
                .unwrap_or_else(|| "nobody".into())
        };
        let show = |undo: &RadarUndo| {
            for (m, asset) in undo.moves.iter().zip(&undo.assets).take(8) {
                let file: String = db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE id = ?1", [asset], |r| r.get(0))?)).unwrap();
                println!("    face {} ({}) from {}", m.face, file, name(m.person));
            }
        };
        let (mut total, mut ms) = (0, 0u128);
        let step = (pinned.len() / 20).max(1);
        for &(face, person, at) in pinned.iter().step_by(step).take(20) {
            let started = std::time::Instant::now();
            let undo = sweep(&db, person, Scope::Around(&[at]), &Thresholds::MEASURED).unwrap();
            ms = ms.max(started.elapsed().as_millis());
            total += undo.moves.len();
            println!("pinned face {face} → {} at {at}: +{} −{} in {} photos", name(Some(person)), undo.added, undo.removed, undo.photos());
            show(&undo);
            db.with_conn(|c| restore_faces(c, &undo.moves)).unwrap();
        }
        let mut seen = HashSet::new();
        for &(face, person, asset) in refusals.iter().filter(|r| seen.insert(r.1)).take(20) {
            let started = std::time::Instant::now();
            let undo = sweep(&db, person, Scope::View { ids: &timeline, at: Some(asset) }, &Thresholds::MEASURED).unwrap();
            ms = ms.max(started.elapsed().as_millis());
            total += undo.moves.len();
            println!("refused face {face} for {}: +{} −{} in {} photos", name(Some(person)), undo.added, undo.removed, undo.photos());
            show(&undo);
            db.with_conn(|c| restore_faces(c, &undo.moves)).unwrap();
        }
        println!("in all: {total} faces; slowest sweep {ms} ms");
    }

    /// The plan's budget: 10k faces near the seed in well under 200 ms (release build).
    /// `cargo test --release -p photos-core --lib radar_speed -- --ignored --nocapture`
    #[test]
    #[ignore = "a measurement"]
    fn radar_speed() {
        let db = library();
        anna_and_bea(&db);
        db.with_conn(|c| {
            let tx = c.unchecked_transaction()?;
            for i in 0..10_000i64 {
                tx.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at)
                     VALUES (?1, ?2, 'p', 'x.jpg', 'jpg', 0, 1, 1, 10, ?3)",
                    params![1000 + i, format!("s{i}"), (i % 300) * DAY],
                )?;
                tx.execute(
                    "INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id, pinned, quality) VALUES (?1, 0.4, 0.3, 0.1, 0.15, 0.9, ?2, 'm', ?3, 0, 0.9)",
                    params![1000 + i, blob(&near(200 + (i % 50) as usize, i as usize)), if i % 2 == 0 { Some(BEA) } else { None }],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let ids: Vec<i64> = (1000..11_000).collect();
        let started = std::time::Instant::now();
        let undo = sweep(&db, ANNA, Scope::View { ids: &ids, at: Some(5000) }, &T).unwrap();
        println!("radar over a view of 10k photos: {} ms, moved {}", started.elapsed().as_millis(), undo.moves.len());
    }
}
