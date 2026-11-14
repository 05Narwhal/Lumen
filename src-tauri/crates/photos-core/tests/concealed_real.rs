//! Migration 0019 over a **copy** of a real catalogue, and what the concealment filter costs:
//! `LUMEN_DB=<copy of library.db> cargo test --release -p photos-core --test concealed_real -- --ignored --nocapture`

use std::time::Instant;

use photos_core::layout::{count_scoped, LayoutIndex};
use photos_core::scope::LayoutScope;

#[test]
#[ignore = "needs LUMEN_DB, a copy of a real library.db"]
fn the_round_3_migration_applies_and_the_library_stays_fast() {
    let path = std::path::PathBuf::from(std::env::var("LUMEN_DB").expect("LUMEN_DB"));
    assert!(!path.components().any(|c| c.as_os_str() == "Lumen Library"), "copy library.db first");
    let db = photos_core::Db::open(&path).unwrap();
    db.with_conn(|c| {
        let concealed: i64 = c.query_row("SELECT COUNT(*) FROM assets WHERE concealed = 1", [], |r| r.get(0))?;
        assert_eq!(concealed, 0, "nothing is hidden until the user hides it");
        let n = count_scoped(c, &LayoutScope::library())?;
        let mut times = Vec::new();
        for _ in 0..7 {
            let t = Instant::now();
            let index = LayoutIndex::load_scoped(c, &LayoutScope::library(), true)?;
            times.push(t.elapsed());
            std::hint::black_box(index);
        }
        times.sort();
        println!("{n} items; layout index load median {:?} (min {:?})", times[3], times[0]);
        Ok(())
    })
    .unwrap();
}
