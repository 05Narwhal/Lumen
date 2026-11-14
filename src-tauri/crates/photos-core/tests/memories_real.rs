//! Memories on a real catalogue. Ignored by default; point it at a *copy* of a library's
//! database (it runs migrations and rewrites the memories tables):
//!
//! ```sh
//! LUMEN_DB=/tmp/copy/library.db cargo test --release -p photos-core --test memories_real -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::Instant;

use photos_core::memories;
use photos_core::Db;

#[test]
#[ignore = "needs LUMEN_DB, a copy of a library database"]
fn regenerates_memories_for_a_real_library() {
    let Some(path) = std::env::var_os("LUMEN_DB") else {
        println!("LUMEN_DB not set");
        return;
    };
    let db = Db::open(&PathBuf::from(path)).expect("open the copy");
    let today = chrono::Local::now().date_naive();
    db.with_conn(|c| {
        let photos = memories::load_photos(c)?;
        let gps = photos.iter().filter(|p| p.lat.is_some()).count();
        let quality = photos.iter().filter(|p| p.sharpness.is_some()).count();
        println!("{} photos with a capture time ({gps} with GPS, {quality} with curation facts)", photos.len());
        println!("home: {:?}", memories::home(&photos));
        let t = Instant::now();
        let n = memories::regenerate(c, today, photos_core::now_unix())?;
        println!("regenerated {n} memories in {:?}", t.elapsed());
        for m in memories::list_memories(c, true)? {
            println!("  [{}] {} — {} ({} photos, {} highlights)", m.kind, m.title, m.subtitle.unwrap_or_default(), m.item_count, m.curated_count);
        }
        Ok(())
    })
    .unwrap();
}
