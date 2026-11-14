//! A real Apple Photos database. Ignored by default; point it at a copy of one:
//!
//! ```sh
//! LUMEN_PHOTOS_SQLITE=/path/to/Photos.sqlite cargo test -p photos-import --test apple_library -- --ignored --nocapture
//! ```
//!
//! Prints counts only, never names or captions. Verified 2026-09-19 against a Photos 5+
//! library of 38,761 assets.

use photos_import::apple::db::read_library;

#[test]
#[ignore = "needs LUMEN_PHOTOS_SQLITE"]
fn reads_a_real_library() {
    let Ok(path) = std::env::var("LUMEN_PHOTOS_SQLITE") else {
        println!("LUMEN_PHOTOS_SQLITE not set");
        return;
    };
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open");
    let t = std::time::Instant::now();
    let assets = read_library(&conn).expect("read");
    let count = |f: &dyn Fn(&photos_import::apple::db::AppleAsset) -> bool| assets.iter().filter(|a| f(a)).count();
    println!("{} assets in {:?}", assets.len(), t.elapsed());
    println!("  favourites {}", count(&|a| a.favorite));
    println!("  hidden {}, trashed {}, unpicked burst frames {}", count(&|a| a.hidden), count(&|a| a.trashed), count(&|a| !a.visible));
    println!("  referenced {}", count(&|a| a.referenced()));
    println!("  videos {}", count(&|a| a.video));
    println!("  with date {}, with place {}, with time zone {}", count(&|a| a.created.is_some()), count(&|a| a.location.is_some()), count(&|a| a.tz_offset_minutes.is_some()));
    println!("  with original filename {}", count(&|a| a.original_filename.is_some()));
    println!("  with title {}, caption {}, keywords {}", count(&|a| a.title.is_some()), count(&|a| a.caption.is_some()), count(&|a| !a.keywords.is_empty()));
    println!("  in an album {}", count(&|a| !a.albums.is_empty()));
    assert!(!assets.is_empty());
    assert!(assets.iter().all(|a| !a.uuid.is_empty()));
}

/// The memories rules over a real Photos catalogue: dates, places and favourites straight
/// from `Photos.sqlite`, no images decoded (so curation has no sharpness to go on).
#[test]
#[ignore = "needs LUMEN_PHOTOS_SQLITE"]
fn finds_memories_in_a_real_photos_history() {
    let Ok(path) = std::env::var("LUMEN_PHOTOS_SQLITE") else {
        println!("LUMEN_PHOTOS_SQLITE not set");
        return;
    };
    let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open");
    let mut photos: Vec<photos_core::memories::Photo> = read_library(&conn)
        .expect("read")
        .into_iter()
        .enumerate()
        .filter(|(_, a)| a.visible && !a.trashed && !a.referenced())
        .filter_map(|(i, a)| {
            let local = a.created? + a.tz_offset_minutes.unwrap_or(0) as i64 * 60;
            Some(photos_core::memories::Photo {
                id: i as i64,
                local,
                lat: a.location.map(|l| l.0),
                lon: a.location.map(|l| l.1),
                favorite: a.favorite,
                rating: 0,
                sharpness: None,
                phash: None,
                // What maintenance would store: the atlas town.
                place: a.location.and_then(|l| photos_core::places::reverse_geocode(l.0, l.1)).and_then(|p| p.city.or(p.country)),
            })
        })
        .collect();
    photos.sort_by_key(|p| p.local);
    let today = chrono::Local::now().date_naive();
    let t = std::time::Instant::now();
    let drafts = photos_core::memories::cluster(&photos, today);
    println!("{} photos → {} memories in {:?}; home {:?}", photos.len(), drafts.len(), t.elapsed(), photos_core::memories::home(&photos));
    for kind in [photos_core::memories::KIND_ON_THIS_DAY, photos_core::memories::KIND_TRIP, photos_core::memories::KIND_EVENT] {
        let of_kind: Vec<_> = drafts.iter().filter(|d| d.kind == kind).collect();
        println!("kind {kind}: {}", of_kind.len());
        for d in of_kind.iter().take(12) {
            let curated = d.items.iter().filter(|i| i.2).count();
            println!("  {} — {} ({} photos, {curated} highlights)", d.title, d.subtitle.clone().unwrap_or_default(), d.items.len());
        }
    }
}
