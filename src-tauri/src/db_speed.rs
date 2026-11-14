//! Round 7: every call the logs named as a slow database call, timed on a COPY of a real
//! library's database. Thumbnails are read from `LUMEN_LIBRARY` (read only); nothing is written
//! there.
//!
//! ```sh
//! LUMEN_DB=<copy of library.db> LUMEN_LIBRARY=<library> cargo test --release -p lumen --lib db_speed_on_a_real_library -- --ignored --nocapture --test-threads=1
//! ```

use std::time::Instant;

use photos_core::db::Db;

#[test]
#[ignore = "needs a copy of a real library.db"]
fn collection_queries_on_a_real_library() {
    let db = Db::open(std::path::Path::new(&std::env::var("LUMEN_DB").unwrap())).unwrap();
    for id in photos_core::collections::CollectionId::ALL {
        let scope = photos_core::scope::LayoutScope { query: Some(id.query()), ..photos_core::scope::LayoutScope::library() };
        let (from_where, order, params) = scope.sql();
        println!("{id:?}: SELECT COUNT(*) {from_where}  -- order {order} params {}", params.len());
        time(&format!("{id:?} count"), 3, || db.with_conn(|c| photos_core::layout::count_scoped(c, &scope)).unwrap());
        time(&format!("{id:?} cover"), 3, || {
            db.with_conn(|c| Ok(c.query_row(&format!("SELECT a.id {from_where} ORDER BY {order} LIMIT 1"), rusqlite::params_from_iter(params.iter()), |r| r.get::<_, i64>(0)).ok()))
                .unwrap()
        });
    }
}

fn time<T>(name: &str, runs: usize, mut f: impl FnMut() -> T) -> T {
    let mut out = None;
    let mut times = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        out = Some(f());
        times.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let first = times[0];
    times.sort_by(f64::total_cmp);
    println!("{name:<38} first {first:>9.1} ms   best {:>9.1} ms", times[0]);
    out.expect("ran")
}

#[test]
#[ignore = "needs a copy of a real library.db"]
fn db_speed_on_a_real_library() {
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
    let db = Db::open(std::path::Path::new(&env("LUMEN_DB"))).unwrap();
    let root = std::path::PathBuf::from(env("LUMEN_LIBRARY"));
    let only = std::env::var("LUMEN_ONLY").ok();
    // The pool's own warnings are what this is about: print them.
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::WARN).with_test_writer().try_init();
    let want = |k: &str| only.as_deref().is_none_or(|o| o.split(',').any(|x| x == k));

    if want("people") {
        time("people::list_people", 3, || db.with_conn(photos_core::people::list_people).unwrap());
    }
    if want("devices") {
        time("devices::list_devices", 3, || db.with_conn(photos_core::devices::list_devices).unwrap());
    }
    if want("collections") {
        time("collections::list_collections", 3, || db.with_conn(photos_core::collections::list_collections).unwrap());
    }
    if want("layout") {
        let scope = photos_core::scope::LayoutScope::library();
        time("LayoutIndex::load_scoped(library)", 3, || {
            db.with_conn(|c| photos_core::layout::LayoutIndex::load_scoped(c, &scope, true)).unwrap()
        });
    }
    if want("layout") {
        let people = db.with_conn(photos_core::people::list_people).unwrap();
        if let Some(p) = people.iter().max_by_key(|p| p.face_count) {
            let scope = photos_core::scope::LayoutScope { person_id: Some(p.id), ..photos_core::scope::LayoutScope::library() };
            time("LayoutIndex::load_scoped(biggest person)", 3, || {
                db.with_conn(|c| photos_core::layout::LayoutIndex::load_scoped(c, &scope, true)).unwrap()
            });
        }
        let albums: Vec<i64> = db.with_conn(|c| Ok(c.prepare("SELECT id FROM albums")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)).unwrap();
        for album in albums.iter().take(3) {
            let scope = photos_core::scope::LayoutScope { album_id: Some(*album), ..photos_core::scope::LayoutScope::library() };
            time(&format!("LayoutIndex::load_scoped(album {album})"), 2, || {
                db.with_conn(|c| photos_core::layout::LayoutIndex::load_scoped(c, &scope, true)).unwrap()
            });
        }
    }
    if want("suggestions") {
        let people = db.with_conn(photos_core::people::list_people).unwrap();
        if let Some(p) = people.iter().max_by_key(|p| p.face_count) {
            time("people::suggestions(biggest)", 3, || {
                photos_core::people::suggestions_on(&db, p.id, &photos_core::people::Thresholds::MEASURED, 24).unwrap()
            });
        }
    }
    if want("recluster") {
        time("GroupingRows::read_faces (the hold)", 3, || db.with_conn(photos_core::people::GroupingRows::read_faces).unwrap());
        let input = time("ReclusterInput::read", 2, || db.with_conn(photos_core::people::GroupingRows::read_faces).map(photos_core::people::ReclusterInput::from_rows).unwrap());
        let mut input = Some(input);
        let plan = time("ReclusterInput::plan (no conn)", 1, || input.take().expect("once").plan(&photos_core::people::Thresholds::MEASURED));
        let moved = time("ReclusterPlan::apply_counting", 1, || db.with_conn(|c| plan.apply_counting(c, photos_core::now_unix())).unwrap());
        println!("  moved {moved:?}");
    }
    if want("memories") {
        let protos = photos_ml::prototypes();
        let now = photos_core::now_unix();
        let today = chrono::Utc::now().date_naive();
        time("memories::regenerate_on", 2, || photos_core::memories::regenerate_on(&db, today, now, &protos).unwrap());
    }
    if want("duplicates") {
        time("duplicates::find_sets (no files)", 2, || db.with_conn(photos_core::duplicates::find_sets).unwrap());
        let sets = time("duplicates (with files, cold)", 1, || {
            crate::commands::duplicates::with_look(&root, |look| photos_core::duplicates::find_sets_on(&db, Some(look))).unwrap()
        });
        println!("  {} sets", sets.len());
        time("duplicates (with files, warm)", 1, || {
            crate::commands::duplicates::with_look(&root, |look| photos_core::duplicates::find_sets_on(&db, Some(look))).unwrap()
        });
    }
}
