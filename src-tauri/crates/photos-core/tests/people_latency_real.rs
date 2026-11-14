//! How long the People screen's edits take on a real catalogue, idle and while the
//! background work the app runs beside them is busy (Round 5B): the face scan storing each
//! photo's faces, regrouping, memories, the duplicate finder, and the global rayon pool full.
//! Ignored by default; it copies `LUMEN_DB` (and its `-wal`) to a temp dir first, so the file
//! given is never opened for writing:
//!
//! ```sh
//! LUMEN_DB=<a copy>/library.db cargo test --release -p photos-core --test people_latency_real -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use photos_core::people::{self, NewFace, Thresholds};
use photos_core::{memories, Db};

fn copy_of(src: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let dst = dir.path().join("library.db");
    std::fs::copy(src, &dst).unwrap();
    let wal = format!("{src}-wal");
    if std::path::Path::new(&wal).exists() {
        std::fs::copy(&wal, dir.path().join("library.db-wal")).unwrap();
    }
    (dir, dst)
}

/// (asset, its faces as the scan would store them again)
fn faces_by_asset(db: &Db, limit: usize) -> Vec<(i64, Vec<NewFace>)> {
    db.with_conn(|c| {
        let assets: Vec<i64> = c.prepare("SELECT DISTINCT asset_id FROM faces ORDER BY asset_id LIMIT ?1")?.query_map([limit as i64], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let mut stmt = c.prepare("SELECT x, y, w, h, score, vec, COALESCE(quality, 0), rejected FROM faces WHERE asset_id = ?1")?;
        let mut out = Vec::new();
        for a in assets {
            let faces = stmt
                .query_map([a], |r| {
                    Ok(NewFace {
                        x: r.get::<_, f64>(0)? as f32,
                        y: r.get::<_, f64>(1)? as f32,
                        w: r.get::<_, f64>(2)? as f32,
                        h: r.get::<_, f64>(3)? as f32,
                        score: r.get::<_, f64>(4)? as f32,
                        vec: r.get::<_, Vec<u8>>(5)?.into_iter().map(|b| b as i8).collect(),
                        quality: r.get::<_, f64>(6)? as f32,
                        rejected: r.get(7)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            out.push((a, faces));
        }
        Ok(out)
    })
    .unwrap()
}

#[derive(Default)]
struct Times(Vec<(&'static str, Duration)>);

impl Times {
    fn time<T>(&mut self, what: &'static str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.0.push((what, t.elapsed()));
        out
    }

    fn report(&self, label: &str) {
        let mut names: Vec<&str> = self.0.iter().map(|(n, _)| *n).collect();
        names.dedup();
        let mut seen = std::collections::HashSet::new();
        println!("--- {label}");
        for n in names.into_iter().filter(|n| seen.insert(*n)) {
            let mut ms: Vec<f64> = self.0.iter().filter(|(m, _)| *m == n).map(|(_, d)| d.as_secs_f64() * 1000.0).collect();
            ms.sort_by(f64::total_cmp);
            println!("{n:<22} n={:<3} median {:>8.1} ms  max {:>8.1} ms", ms.len(), ms[ms.len() / 2], ms[ms.len() - 1]);
        }
    }
}

/// One round of what a user does on People: open it, open someone, name them, take photos
/// out of them and put them back, answer a suggestion and undo it.
fn user_round(db: &Db, times: &mut Times, person: i64, name: &str) {
    let t = Thresholds::MEASURED;
    times.time("list_people", || db.with_conn(people::list_people).unwrap());
    let sugg = times.time("person_suggestions", || db.with_conn(|c| people::suggestions(c, person, &t, 24)).unwrap());
    times.time("name_person", || db.with_conn(|c| people::name_person(c, person, name)).unwrap());
    let photos: Vec<i64> = db.with_conn(|c| Ok(c.prepare("SELECT DISTINCT asset_id FROM faces WHERE person_id = ?1 LIMIT 3")?.query_map([person], |r| r.get(0))?.collect::<Result<_, _>>()?)).unwrap();
    let moved = times.time("move_person_photos", || db.with_conn(|c| people::move_photos(c, person, &photos, None)).unwrap());
    times.time("restore_faces", || db.with_conn(|c| people::restore_faces(c, &moved)).unwrap());
    if let Some(s) = sugg.first() {
        let undo = times.time("answer_suggestion", || db.with_conn(|c| people::answer_suggestion(c, s.face_id, person, true, 1)).unwrap());
        times.time("undo_suggestion", || db.with_conn(|c| people::undo_suggestion(c, &undo)).unwrap());
    }
}

#[test]
#[ignore = "needs LUMEN_DB, a copy of a library database"]
fn people_edits_stay_fast_under_background_load() {
    let Ok(src) = std::env::var("LUMEN_DB") else {
        println!("LUMEN_DB not set");
        return;
    };
    let (_dir, path) = copy_of(&src);
    let db = Arc::new(Db::open(&path).unwrap());
    // The named person with the most photos: naming them re-indexes the most.
    let (person, name): (i64, String) = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT p.id, p.name FROM people p JOIN faces f ON f.person_id = p.id WHERE p.name IS NOT NULL AND p.status = 'listed'
                 GROUP BY p.id ORDER BY COUNT(DISTINCT f.asset_id) DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    let photos: i64 = db.with_conn(|c| Ok(c.query_row("SELECT COUNT(DISTINCT asset_id) FROM faces WHERE person_id = ?1", [person], |r| r.get(0))?)).unwrap();
    println!("person {person} has {photos} photos");

    let mut idle = Times::default();
    for _ in 0..5 {
        user_round(&db, &mut idle, person, &name);
    }
    idle.report("idle");

    let stop = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::new();
    // The face scan: each photo's faces stored again, one short write per photo.
    let scan = faces_by_asset(&db, 2000);
    {
        let (db, stop) = (db.clone(), stop.clone());
        workers.push(std::thread::spawn(move || {
            let mut t = Times::default();
            for (asset, faces) in scan.iter().cycle() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let _ = t.time("bg: scan store", || db.with_conn(|c| people::replace_faces_keeping_identity(c, *asset, "load", faces)));
                // Finding and embedding a photo's faces takes ~50 ms (faces.rs).
                std::thread::sleep(Duration::from_millis(50));
            }
            t
        }));
    }
    // Regrouping as `faces::regroup_now` does it, every second.
    {
        let (db, stop) = (db.clone(), stop.clone());
        workers.push(std::thread::spawn(move || {
            let mut t = Times::default();
            while !stop.load(Ordering::Relaxed) {
                let plan = t.time("bg: regroup read", || db.with_conn(people::ReclusterInput::read)).map(|i| i.plan(&Thresholds::MEASURED));
                if let Ok(p) = plan {
                    let _ = t.time("bg: regroup apply", || db.with_conn(|c| p.apply(c, 1)));
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            t
        }));
    }
    // Memories and the duplicate finder, back to back.
    {
        let (db, stop) = (db.clone(), stop.clone());
        workers.push(std::thread::spawn(move || {
            let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
            let mut t = Times::default();
            while !stop.load(Ordering::Relaxed) {
                let _ = t.time("bg: memories", || db.with_conn(|c| memories::regenerate(c, today, 1)));
                let _ = t.time("bg: duplicates", || db.with_conn(|c| photos_core::duplicates::find_sets_in(c, None)));
            }
            t
        }));
    }
    // The global rayon pool kept full, as resizing and embedding keep it during a scan.
    {
        let stop = stop.clone();
        workers.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                rayon::scope(|s| {
                    for _ in 0..rayon::current_num_threads() * 2 {
                        s.spawn(|_| {
                            let t = Instant::now();
                            let mut x = 0u64;
                            while t.elapsed() < Duration::from_millis(200) {
                                x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
                            }
                            std::hint::black_box(x);
                        });
                    }
                });
            }
            Times::default()
        }));
    }
    std::thread::sleep(Duration::from_secs(2));
    let mut busy = Times::default();
    for _ in 0..5 {
        user_round(&db, &mut busy, person, &name);
    }
    busy.report("under background load, user calls on the global pool");
    // The app runs the People screen's calls on its render pool (`photos_jobs::interactive`).
    let interactive = rayon::ThreadPoolBuilder::new().build().unwrap();
    let mut busy = Times::default();
    for _ in 0..5 {
        interactive.install(|| user_round(&db, &mut busy, person, &name));
    }
    stop.store(true, Ordering::Relaxed);
    let mut background = Times::default();
    for w in workers {
        background.0.extend(w.join().unwrap().0);
    }
    busy.report("under background load, user calls on their own pool");
    background.report("the background work meanwhile");
}
