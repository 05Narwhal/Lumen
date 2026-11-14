//! Mac-formatted disks on real hardware. Ignored by default; run with one attached:
//!
//! ```sh
//! cargo test -p photos-import --test macdisk_hardware -- --ignored --nocapture
//! ```
//!
//! Verified 2026-09-19 on Windows 11, unelevated, with an HFS+ ADATA HD710 PRO over USB.

use std::sync::Arc;
use std::time::Instant;

use photos_import::macdisk::{probe, source};
use photos_import::ImportSource;

#[test]
#[ignore = "needs a Mac-formatted disk attached"]
fn finds_and_reads_mac_partitions() {
    let parts = probe::partitions();
    println!("{} partitions", parts.len());
    for p in parts.iter().filter_map(probe::probe) {
        println!("{} {:?} {} bytes {:?}", p.part.id(), p.kind, p.len, p.disk_name);
        let fs = probe::open(&p).expect("open");
        println!("  volume name {:?}, free {:?}", fs.volume_name(), fs.free_bytes());
        for e in fs.list("/").expect("list").iter().take(12) {
            println!("  /{} dir={} {}", e.name, e.is_dir, e.size);
        }
    }
}

#[test]
#[ignore = "needs a Mac-formatted disk attached"]
fn browses_folders_and_finds_photos_libraries() {
    for p in probe::partitions().iter().filter_map(probe::probe) {
        let fs = Arc::new(probe::open(&p).expect("open"));
        let t = Instant::now();
        let libraries = source::find_photos_libraries(&fs, 3);
        println!("{}: libraries {:?} in {:?}", p.part.id(), libraries, t.elapsed());
        let root = source::folder_listing(&fs, "/").expect("root");
        println!("  / has {} folders, {} media: {:?}", root.folders.len(), root.media_here, root.folders);
        assert!(root.folders.iter().all(|f| !f.starts_with('.') && !f.chars().any(char::is_control)));
        for folder in root.folders.iter().take(3) {
            let path = format!("/{folder}");
            let t = Instant::now();
            let items = source::MacFolderSource::new(fs.clone(), "disk", &path, true).enumerate().expect("enumerate");
            println!("  {path}: {} photos and videos (recursive) in {:?}", items.len(), t.elapsed());
        }
    }
}

#[test]
#[ignore = "needs a Mac-formatted disk with a Photos library attached"]
fn opens_a_photos_library_on_the_disk() {
    use photos_import::apple::files::MacDiskFiles;
    use photos_import::apple::source::PhotosLibrarySource;
    for p in probe::partitions().iter().filter_map(probe::probe) {
        let fs = Arc::new(probe::open(&p).expect("open"));
        for root in source::find_photos_libraries(&fs, 3) {
            let t = Instant::now();
            let lib = PhotosLibrarySource::open(Box::new(MacDiskFiles { fs: fs.clone(), root: root.clone() }), "library").expect("library");
            println!("opened in {:?}: {:?}", t.elapsed(), lib.summary());
            let newest = lib.newest(12);
            assert_eq!(newest.len(), 12.min(lib.summary().items as usize));
            let t = Instant::now();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut lib.open(newest[0]).expect("open newest"), &mut bytes).expect("read");
            println!("newest file: {} bytes in {:?}", bytes.len(), t.elapsed());
            assert_eq!(bytes.len() as u64, newest[0].size);
        }
    }
}

/// The acceptance check for Phase 4b: the 40 newest photos of the library on the disk,
/// with their Live Photo videos, imported into a throwaway Lumen library.
#[test]
#[ignore = "needs a Mac-formatted disk with a Photos library attached"]
fn imports_the_newest_photos_of_a_library_with_what_photos_knows() {
    use photos_import::apple::files::MacDiskFiles;
    use photos_import::apple::source::PhotosLibrarySource;
    use photos_import::{run_import, FilteredSource, ImportOptions};

    let Some(p) = probe::partitions().iter().find_map(probe::probe) else {
        println!("no Mac disk attached");
        return;
    };
    let fs = Arc::new(probe::open(&p).expect("open"));
    let Some(root) = source::find_photos_libraries(&fs, 3).into_iter().next() else {
        println!("no Photos library on {}", p.part.id());
        return;
    };
    let lib = Arc::new(PhotosLibrarySource::open(Box::new(MacDiskFiles { fs: fs.clone(), root }), "library").expect("library"));
    let stem = |rel: &str| rel.rsplit('/').next().unwrap_or("").split(['_', '.']).next().unwrap_or("").to_string();
    let items = lib.enumerate().unwrap();
    // The newest 40, plus enough of the rest to exercise every path: ten Live Photos, ten
    // photos in albums, and the favourites.
    let mut chosen: std::collections::HashSet<String> = lib.newest(40).iter().map(|i| stem(&i.rel_path)).collect();
    chosen.extend(items.iter().filter(|i| i.rel_path.contains("_3.")).take(10).map(|i| stem(&i.rel_path)));
    chosen.extend(items.iter().filter(|i| lib.extras(i).is_some_and(|x| !x.albums.is_empty())).take(10).map(|i| stem(&i.rel_path)));
    chosen.extend(items.iter().filter(|i| lib.extras(i).is_some_and(|x| x.favorite)).map(|i| stem(&i.rel_path)));
    let allow: std::collections::HashSet<String> = items.iter().filter(|i| chosen.contains(&stem(&i.rel_path))).map(|i| i.rel_path.clone()).collect();
    let primaries: Vec<_> = items.iter().filter(|i| allow.contains(&i.rel_path) && lib.extras(i).is_some()).collect();
    let live_videos = items.iter().filter(|i| allow.contains(&i.rel_path) && i.rel_path.contains("_3.")).count();

    let tmp = tempfile::tempdir().unwrap();
    let catalogue = photos_core::Library::create(&tmp.path().join("lib")).unwrap();
    let opts = ImportOptions { ffmpeg: photos_media::Ffmpeg::locate(), ..Default::default() };
    let t = Instant::now();
    let summary = run_import(&catalogue, &FilteredSource::new(lib.clone(), allow.clone()), &opts, &photos_jobs::CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    println!("{} files ({} photos, {} Live Photo videos): {summary:?} in {:?}", allow.len(), primaries.len(), live_videos, t.elapsed());
    assert!(summary.imported > 0);
    assert_eq!(summary.failed, 0);

    let q = |sql: &str| -> i64 { catalogue.db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap() };
    let live = q("SELECT COUNT(*) FROM assets WHERE media_type = 2");
    println!("paired Live Photos: {live}");
    if live_videos > 0 && opts.ffmpeg.is_some() {
        assert!(live > 0, "Live Photo stills and their videos were paired");
    }
    let favourites_expected = primaries.iter().filter(|i| lib.extras(i).unwrap().favorite).count() as i64;
    assert_eq!(q("SELECT COUNT(*) FROM assets WHERE is_favorite = 1"), favourites_expected);
    let memberships_expected: i64 = primaries.iter().map(|i| lib.extras(i).unwrap().albums.len() as i64).sum();
    assert_eq!(q("SELECT COUNT(*) FROM album_items"), memberships_expected);
    println!("favourites {favourites_expected}, album memberships {memberships_expected}");

    // Photos' own capture time wins over the file's.
    for item in primaries.iter().take(5) {
        let Some((utc, _)) = lib.extras(item).unwrap().captured else { continue };
        let stored: Option<i64> = catalogue
            .db
            .with_conn(|c| Ok(c.query_row("SELECT captured_at_utc FROM assets WHERE filename = ?1", [&item.file_name], |r| r.get(0))?))
            .unwrap();
        assert_eq!(stored, Some(utc), "{}", item.file_name);
    }
}
