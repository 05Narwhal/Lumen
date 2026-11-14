use std::path::Path;

use image::{ImageBuffer, Rgb};
use photos_core::batches::{list_batches, list_failures, BATCH_CANCELLED, BATCH_COMPLETE};
use photos_core::Library;
use photos_import::*;
use photos_jobs::CancelToken;

fn write_jpeg(path: &Path, seed: u8) {
    let img = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
    img.save(path).unwrap();
}

fn make_source(dir: &Path) {
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    write_jpeg(&dir.join("a.jpg"), 1);
    write_jpeg(&dir.join("sub/b.JPG"), 2);
    write_jpeg(&dir.join("sub/c.jpeg"), 3);
    std::fs::copy(dir.join("a.jpg"), dir.join("sub/a-copy.jpg")).unwrap(); // duplicate content
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    std::fs::create_dir_all(dir.join("bad.jpg")).unwrap(); // a directory with an image extension: open() fails
    std::fs::create_dir_all(dir.join(".hidden")).unwrap();
    write_jpeg(&dir.join(".hidden/h.jpg"), 4);
}

#[test]
fn content_hash_names_a_file_as_its_import_will() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("export.jpg");
    write_jpeg(&file, 9);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let mut ingested = Vec::new();
    run_import(&lib, &FilesSource::new(vec![file.clone()]), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |i| ingested.push(i.clone())).unwrap();
    assert_eq!(ingested.len(), 1);
    assert_eq!(content_hash(&file).unwrap(), ingested[0].content_hash);
}

#[test]
fn imports_dedups_records_failures_and_is_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    make_source(&src);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let source = FolderSource::new(&src);
    let mut ingested = Vec::new();
    let mut last = ImportProgress::default();

    let summary = run_import(
        &lib,
        &source,
        &ImportOptions::default(),
        &CancelToken::new(),
        &mut |p| last = p.clone(),
        &mut |i| ingested.push(i.clone()),
    )
    .unwrap();

    assert_eq!((summary.imported, summary.skipped, summary.failed), (3, 1, 0), "{summary:?}");
    assert!(!summary.cancelled);
    assert_eq!(last.total, 4, "txt, hidden and the directory are not enumerated");
    assert_eq!(ingested.len(), 3);
    for i in &ingested {
        assert!(i.original.is_file(), "{}", i.original.display());
        let rel = i.original.strip_prefix(lib.originals_dir()).unwrap();
        assert_eq!(rel.components().count(), 4, "YYYY/MM/DD/file: {}", rel.display());
        assert_eq!(i.content_hash.len(), 64);
    }
    assert!(!lib.incoming_dir().exists());

    let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!((batches[0].item_count, batches[0].skipped_count, batches[0].state), (3, 1, BATCH_COMPLETE));
    assert_eq!(batches[0].source_path.as_deref(), Some(source.label().as_str()));
    assert_eq!(lib.db.with_conn(|c| list_failures(c, batches[0].id)).unwrap().len(), 0);

    // Second run: everything is a duplicate.
    let again = run_import(&lib, &source, &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((again.imported, again.skipped, again.failed), (0, 4, 0));
    let stats = lib.db.with_conn(photos_core::assets::stats).unwrap();
    assert_eq!(stats.items, 3);
}

#[test]
fn open_failures_are_recorded_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("ok.jpg"), 5);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    struct Flaky(FolderSource);
    impl ImportSource for Flaky {
        fn label(&self) -> String { self.0.label() }
        fn kind(&self) -> i64 { self.0.kind() }
        fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
            let mut items = self.0.enumerate()?;
            items.push(SourceItem { rel_path: "ghost.jpg".into(), file_name: "ghost.jpg".into(), size: 1, modified_at: None, created_at: None });
            Ok(items)
        }
        fn open(&self, item: &SourceItem) -> Result<Box<dyn std::io::Read + Send>, ImportError> { self.0.open(item) }
    }

    let summary = run_import(&lib, &Flaky(FolderSource::new(&src)), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((summary.imported, summary.failed), (1, 1));
    let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
    let failures = lib.db.with_conn(|c| list_failures(c, batches[0].id)).unwrap();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].path, "ghost.jpg");
}

#[test]
fn cancelled_before_start_marks_batch_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    make_source(&src);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let cancel = CancelToken::new();
    cancel.cancel();
    let summary = run_import(&lib, &FolderSource::new(&src), &ImportOptions::default(), &cancel, &mut |_| {}, &mut |_| {}).unwrap();
    assert!(summary.cancelled);
    assert_eq!(summary.imported, 0);
    let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
    assert_eq!(batches[0].state, BATCH_CANCELLED);
}

#[test]
fn missing_source_folder_fails_the_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let err = run_import(&lib, &FolderSource::new(tmp.path().join("nope")), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {});
    assert!(matches!(err, Err(ImportError::Source(_))));
    let batches = lib.db.with_conn(|c| list_batches(c, 10)).unwrap();
    assert_eq!(batches[0].state, photos_core::batches::BATCH_FAILED);
}

/// Needs FFmpeg (skips with a note otherwise): a video is catalogued with its probe facts.
#[test]
fn imports_video_with_probe_when_ffmpeg_is_present() {
    let Some(ff) = photos_media::Ffmpeg::locate() else {
        eprintln!("note: FFmpeg not found; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    let st = std::process::Command::new(&ff.ffmpeg)
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=10", "-t", "2", "-pix_fmt", "yuv420p"])
        .args(["-metadata", "creation_time=2024-05-01T10:20:30Z"])
        .arg(src.join("clip.mp4"))
        .status()
        .unwrap();
    assert!(st.success());
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let opts = ImportOptions { ffmpeg: Some(ff), ..Default::default() };
    let mut ingested = Vec::new();
    let summary = run_import(&lib, &FolderSource::new(&src), &opts, &CancelToken::new(), &mut |_| {}, &mut |i| ingested.push(i.clone())).unwrap();
    assert_eq!((summary.imported, summary.failed), (1, 0));
    assert_eq!(ingested[0].kind, photos_media::MediaKind::Video);
    let (media_type, duration, w, h, captured, meta): (i64, i64, i64, i64, i64, String) = lib
        .db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT media_type, duration_ms, width, height, captured_at, metadata_json FROM assets WHERE id = ?1",
                [ingested[0].asset_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )?)
        })
        .unwrap();
    assert_eq!(media_type, 1);
    assert!((1900..=2100).contains(&duration), "{duration}");
    assert_eq!((w, h), (320, 240));
    assert_eq!(captured, 1714558830, "creation_time becomes captured_at");
    assert!(meta.contains("\"probe\""), "{meta}");
}

/// A folder whose `a.jpg` the source knows more about than the file does.
struct WithExtras(FolderSource);

impl ImportSource for WithExtras {
    fn label(&self) -> String {
        self.0.label()
    }
    fn kind(&self) -> i64 {
        self.0.kind()
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        self.0.enumerate()
    }
    fn open(&self, i: &SourceItem) -> Result<Box<dyn std::io::Read + Send>, ImportError> {
        self.0.open(i)
    }
    fn extras(&self, i: &SourceItem) -> Option<photos_core::extras::ItemExtras> {
        (i.file_name == "a.jpg").then(|| photos_core::extras::ItemExtras { favorite: true, albums: vec!["Trip".into()], ..Default::default() })
    }
}

#[test]
fn extras_reach_new_and_duplicate_assets() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    write_jpeg(&src.join("b.jpg"), 2);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let favourites = |lib: &Library| -> Vec<(String, i64)> {
        lib.db
            .with_conn(|c| {
                let mut s = c.prepare("SELECT filename, is_favorite FROM assets ORDER BY filename")?;
                let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .unwrap()
    };
    let album_items = |lib: &Library| -> i64 { lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM album_items", [], |r| r.get(0))?)).unwrap() };
    let run = |source: &dyn ImportSource| run_import(&lib, source, &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();

    let first = run(&WithExtras(FolderSource::new(&src)));
    assert_eq!(first.imported, 2);
    assert_eq!(favourites(&lib), [("a.jpg".to_string(), 1), ("b.jpg".to_string(), 0)]);
    assert_eq!(album_items(&lib), 1);

    // Imported again, through a filter as the card and library imports do: nothing new,
    // but the duplicate still receives what the source knows.
    lib.db.with_conn(|c| Ok(c.execute("UPDATE assets SET is_favorite = 0", [])?)).unwrap();
    let allow = ["a.jpg".to_string()].into_iter().collect();
    let second = run(&FilteredSource::new(WithExtras(FolderSource::new(&src)), allow));
    assert_eq!((second.imported, second.skipped), (0, 1));
    assert_eq!(favourites(&lib)[0], ("a.jpg".to_string(), 1));
    assert_eq!(album_items(&lib), 1, "album membership is not duplicated");
}

#[test]
fn a_folder_imported_as_albums_makes_its_tree_once_and_files_duplicates_too() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("Trips");
    for dir in ["2019/Summer", "2020", "Empty", "Junk"] {
        std::fs::create_dir_all(src.join(dir)).unwrap();
    }
    write_jpeg(&src.join("top.jpg"), 1);
    write_jpeg(&src.join("2019/Summer/b.jpg"), 2);
    write_jpeg(&src.join("2019/c.jpg"), 3);
    write_jpeg(&src.join("2020/d.jpg"), 4);
    std::fs::copy(src.join("top.jpg"), src.join("2020/top-copy.jpg")).unwrap();
    std::fs::write(src.join("Junk/notes.txt"), b"ignored").unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    for _ in 0..2 {
        let source = FolderAlbumsSource::new(FolderSource::new(&src));
        run_import(&lib, &source, &ImportOptions { skip_duplicates: true, ..Default::default() }, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    }
    let (albums, collections) = lib
        .db
        .with_conn(|c| {
            let albums: Vec<String> = c
                .prepare(
                    "SELECT COALESCE(p.name || '/', '') || COALESCE(k.name || '/', '') || a.name || ' ' || (SELECT COUNT(*) FROM album_items i WHERE i.album_id = a.id)
                     FROM albums a LEFT JOIN album_collections k ON k.id = a.collection_id LEFT JOIN album_collections p ON p.id = k.parent_id ORDER BY 1",
                )?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let collections: Vec<String> = c.prepare("SELECT name FROM album_collections ORDER BY name")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok((albums, collections))
        })
        .unwrap();
    assert_eq!(albums, ["Trips/2019/2019 1", "Trips/2019/Summer 1", "Trips/2020 2", "Trips/Trips 1"], "the copy in 2020 is a duplicate but still filed there");
    assert_eq!(collections, ["2019", "Trips"], "made once over two imports; empty and junk folders make nothing");
}

#[test]
fn live_photos_pair_inside_their_own_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    for (dir, seed) in [("trip_a", 10u8), ("trip_b", 20u8)] {
        std::fs::create_dir_all(src.join(dir)).unwrap();
        write_jpeg(&src.join(dir).join("IMG_0001.jpg"), seed);
        // Different bytes per folder so neither movie is a duplicate of the other.
        std::fs::write(src.join(dir).join("IMG_0001.mov"), [seed; 64]).unwrap();
    }
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    run_import(&lib, &FolderSource::new(&src), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();

    let rows: Vec<(String, i64, Option<String>)> = lib
        .db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT source_dir, id, live_photo_group FROM assets WHERE media_type = 2 ORDER BY source_dir")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    assert_eq!(rows.len(), 2, "both stills became Live Photos: {rows:?}");
    assert_eq!(rows[0].0, "trip_a");
    assert_ne!(rows[0].2, rows[1].2, "each pair is its own group");
    for (_, still, _) in &rows {
        let mov = lib.db.with_conn(|c| photos_core::assets::live_photo_mov(c, *still)).unwrap().expect("a movie");
        let dir: String = lib.db.with_conn(|c| Ok(c.query_row("SELECT source_dir FROM assets WHERE id = ?1", [mov], |r| r.get(0))?)).unwrap();
        let still_dir: String = lib.db.with_conn(|c| Ok(c.query_row("SELECT source_dir FROM assets WHERE id = ?1", [*still], |r| r.get(0))?)).unwrap();
        assert_eq!(dir, still_dir);
    }
}
