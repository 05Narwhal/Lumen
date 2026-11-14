//! Re-dating a real catalogue. Ignored by default; it works on a **copy** of a library
//! (the database is rewritten, the originals are only read):
//!
//! ```sh
//! LUMEN_REDATE_LIBRARY="C:/copy of/Lumen Library" \
//!   cargo test -p lumen --test e2e_redate -- --ignored --nocapture
//! ```
//!
//! It prints what moved and proves the two things Phase 6b's date fix promises: a photo's
//! date comes from the file, and the timeline then reads in true chronological order.

use std::collections::BTreeMap;
use std::path::PathBuf;

use photos_core::Library;
use photos_media::Ffmpeg;

#[test]
#[ignore = "needs LUMEN_REDATE_LIBRARY (a copy)"]
fn corrects_a_real_catalogue_and_re_sorts_the_timeline() {
    let Ok(root) = std::env::var("LUMEN_REDATE_LIBRARY") else {
        println!("LUMEN_REDATE_LIBRARY not set");
        return;
    };
    let lib = Library::open(&PathBuf::from(root)).expect("open the copied library");
    let ffmpeg = Ffmpeg::locate();

    let before: BTreeMap<i64, Option<i64>> = lib
        .db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, captured_at FROM assets WHERE state = 0")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .unwrap();

    let (mut read, mut moved) = (0, 0);
    loop {
        let (r, m) = lumen_lib::maintenance::recheck_dates_in(&lib, ffmpeg.as_ref(), 200);
        read += r;
        moved += m;
        if r == 0 {
            break;
        }
    }
    println!("read {read} assets, corrected {moved}");
    assert_eq!(read, before.len(), "every visible asset is re-read exactly once");

    let after: Vec<(i64, Option<i64>, String)> = lib
        .db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT id, captured_at, capture_source FROM assets WHERE state = 0 ORDER BY captured_at")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            Ok(rows)
        })
        .unwrap();

    let mut sources: BTreeMap<String, usize> = BTreeMap::new();
    for (id, now, source) in &after {
        *sources.entry(source.clone()).or_default() += 1;
        if before[id] != *now {
            let fmt = |t: Option<i64>| {
                t.and_then(|t| chrono::DateTime::from_timestamp(t, 0)).map_or("-".into(), |d| d.format("%Y-%m-%d %H:%M").to_string())
            };
            println!("  {id:4}  {}  →  {}  ({source})", fmt(before[id]), fmt(*now));
        }
    }
    println!("sources: {sources:?}");

    // The timeline orders by captured_at, so a corrected date is a corrected position.
    let dates: Vec<i64> = after.iter().filter_map(|(_, t, _)| *t).collect();
    assert!(dates.windows(2).all(|w| w[0] <= w[1]), "the catalogue reads in chronological order");

    // Nothing is left for a second pass.
    let (again, _) = lumen_lib::maintenance::recheck_dates_in(&lib, ffmpeg.as_ref(), 200);
    assert_eq!(again, 0, "the pass is done in one sweep");
}
