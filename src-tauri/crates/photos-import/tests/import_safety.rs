//! Round 3, Task 11: an import survives a device unplugged mid-way and an app killed between
//! placing a file and cataloguing it, and never marks a file it did not bring in as seen.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use image::{ImageBuffer, Rgb};
use photos_core::batches::{list_batches, list_failures, BATCH_INTERRUPTED};
use photos_core::Library;
use photos_import::budget;
use photos_import::*;
use photos_jobs::CancelToken;

fn write_jpeg(path: &Path, seed: u8) {
    let img = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
    img.save(path).unwrap();
}

fn seen_count(lib: &Library, key: &str) -> i64 {
    lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM import_seen WHERE source_key = ?1", [key], |r| r.get(0))?)).unwrap()
}

/// A card that is pulled out after `ok` files have been opened.
struct Unplugged {
    inner: FolderSource,
    ok: usize,
    opened: AtomicUsize,
    connected: AtomicBool,
}

impl ImportSource for Unplugged {
    fn label(&self) -> String {
        "Card".into()
    }
    fn kind(&self) -> i64 {
        photos_core::batches::SOURCE_VOLUME
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        self.inner.enumerate()
    }
    fn open(&self, item: &SourceItem) -> Result<Box<dyn std::io::Read + Send>, ImportError> {
        if self.opened.fetch_add(1, Ordering::SeqCst) >= self.ok {
            self.connected.store(false, Ordering::SeqCst);
            return Err(ImportError::Source("The device is gone".into()));
        }
        self.inner.open(item)
    }
    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }
}

/// Review Focus 1.
#[test]
fn a_source_that_disappears_interrupts_the_batch() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..10u8 {
        write_jpeg(&src.join(format!("IMG_{i}.jpg")), i + 1);
    }
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let source = Unplugged { inner: FolderSource::new(&src), ok: 3, opened: AtomicUsize::new(0), connected: AtomicBool::new(true) };
    let opts = ImportOptions { seen_key: Some("vol:card".into()), ..Default::default() };
    let summary = run_import(&lib, &source, &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();

    assert!(summary.interrupted, "{summary:?}");
    assert_eq!(summary.imported, 3);
    let batch = &lib.db.with_conn(|c| list_batches(c, 10)).unwrap()[0];
    assert_eq!(batch.state, BATCH_INTERRUPTED);
    let failures = lib.db.with_conn(|c| list_failures(c, batch.id)).unwrap();
    assert_eq!(failures.len(), 1, "one line for the disconnect, not one per file left: {failures:?}");
    assert!(failures[0].reason.contains("disconnected"));
    assert_eq!(seen_count(&lib, "vol:card"), 3, "exactly what came in is remembered");
    assert!(!lib.incoming_dir().exists() || std::fs::read_dir(lib.incoming_dir()).unwrap().next().is_none());
    let rows: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM import_journal", [], |r| r.get(0))?)).unwrap();
    assert_eq!(rows, 0, "every placement was catalogued");
}

#[test]
fn a_failed_file_is_not_marked_seen() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("ok.jpg"), 5);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    struct Ghost(FolderSource);
    impl ImportSource for Ghost {
        fn label(&self) -> String {
            self.0.label()
        }
        fn kind(&self) -> i64 {
            self.0.kind()
        }
        fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
            let mut items = self.0.enumerate()?;
            items.push(SourceItem { rel_path: "ghost.jpg".into(), file_name: "ghost.jpg".into(), size: 1, modified_at: None, created_at: None });
            Ok(items)
        }
        fn open(&self, item: &SourceItem) -> Result<Box<dyn std::io::Read + Send>, ImportError> {
            self.0.open(item)
        }
    }

    let opts = ImportOptions { seen_key: Some("vol:ghost".into()), ..Default::default() };
    let summary = run_import(&lib, &Ghost(FolderSource::new(&src)), &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((summary.imported, summary.failed, summary.interrupted), (1, 1, false));
    assert_eq!(seen_count(&lib, "vol:ghost"), 1, "the file that failed will be offered again");
    // A duplicate is seen too: it is in the library.
    let again = run_import(&lib, &Ghost(FolderSource::new(&src)), &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!(again.skipped, 1);
    assert_eq!(seen_count(&lib, "vol:ghost"), 1);
}

/// Round 19 §13 (2): the staged copy is read back from the disk before it is placed. A copy
/// whose bytes differ from what the source gave fails that file alone: no row, nothing in
/// `originals/`, no journal entry, the source untouched, the reason in the batch's failures,
/// and the file offered again next time.
#[test]
fn a_copy_that_does_not_read_back_fails_that_file_only() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("good.jpg"), 3);
    write_jpeg(&src.join("bad.jpg"), 7);
    let source_bytes = std::fs::read(src.join("bad.jpg")).unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    fn flip_a_byte(item: &SourceItem, staged: &Path) {
        if item.file_name == "bad.jpg" {
            let mut bytes = std::fs::read(staged).unwrap();
            bytes[100] ^= 0xFF;
            std::fs::write(staged, bytes).unwrap();
        }
    }
    let opts = ImportOptions { seen_key: Some("vol:verify".into()), before_verify: Some(flip_a_byte), ..Default::default() };
    let summary = run_import(&lib, &FolderSource::new(&src), &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();

    assert_eq!((summary.imported, summary.failed, summary.skipped), (1, 1, 0), "{summary:?}");
    let batch = &lib.db.with_conn(|c| list_batches(c, 10)).unwrap()[0];
    let failures = lib.db.with_conn(|c| list_failures(c, batch.id)).unwrap();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].path, "bad.jpg");
    assert!(failures[0].reason.contains("did not match"), "{}", failures[0].reason);

    assert_eq!(std::fs::read(src.join("bad.jpg")).unwrap(), source_bytes, "the source is never touched");
    let names: Vec<String> = lib.db.with_conn(|c| {
        let mut st = c.prepare("SELECT filename FROM assets")?;
        let rows = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(rows)
    }).unwrap();
    assert_eq!(names, vec!["good.jpg".to_string()]);
    let placed = walkdir_files(&lib.originals_dir());
    assert_eq!(placed.len(), 1, "only the verified copy is in originals/: {placed:?}");
    let journal: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM import_journal", [], |r| r.get(0))?)).unwrap();
    assert_eq!(journal, 0);
    assert!(!lib.incoming_dir().exists() || std::fs::read_dir(lib.incoming_dir()).unwrap().next().is_none(), "the bad copy is not left behind");
    assert_eq!(seen_count(&lib, "vol:verify"), 1, "the failed file will be offered again");

    // Without the fault the same file comes in.
    let opts = ImportOptions { seen_key: Some("vol:verify".into()), ..Default::default() };
    let again = run_import(&lib, &FolderSource::new(&src), &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((again.imported, again.skipped, again.failed), (1, 1, 0), "{again:?}");
}

fn walkdir_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// Review Focus 2.
#[test]
fn an_unfinished_placement_is_reconciled_on_open() {
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let place = |name: &str, seed: u8| {
        let rel = format!("2026/01/01/{name}");
        let path = lib.originals_dir().join(&rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        write_jpeg(&path, seed);
        (rel, content_hash(&path).unwrap(), path)
    };
    // Placed, never catalogued: Lumen's own copy, removed.
    let (orphan_rel, orphan_hash, orphan) = place("orphan.jpg", 1);
    // Placed and catalogued, the journal row left behind: the file stays.
    let (kept_rel, kept_hash, kept) = place("kept.jpg", 2);
    // A journal row whose file is not what was placed: never deleted.
    let (other_rel, _, other) = place("other.jpg", 3);
    lib.db
        .with_conn(|c| {
            photos_core::import_journal::record(c, 1, &orphan_hash, &orphan_rel, 1)?;
            photos_core::import_journal::record(c, 1, &kept_hash, &kept_rel, 1)?;
            photos_core::import_journal::record(c, 1, "0000", &other_rel, 1)?;
            photos_core::assets::insert_assets(
                c,
                &[photos_core::assets::NewAsset {
                    content_hash: kept_hash.clone(),
                    relative_path: kept_rel.clone(),
                    filename: "kept.jpg".into(),
                    extension: "jpg".into(),
                    byte_size: 1,
                    imported_at: 1,
                    ..Default::default()
                }],
            )?;
            Ok(())
        })
        .unwrap();

    let report = recover(&lib).unwrap();
    assert_eq!(report.removed, 1);
    assert!(!orphan.exists(), "the uncatalogued copy is gone, so a new import brings it in cleanly");
    assert!(kept.exists() && other.exists());
    let rows: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM import_journal", [], |r| r.get(0))?)).unwrap();
    assert_eq!(rows, 0);
}

/// Review Focus 4: the drive fills while importing (another program writing, or a card
/// bigger than the plan thought). The import stops before crossing the buffer.
#[test]
fn the_guard_stops_before_the_buffer() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..6u8 {
        write_jpeg(&src.join(format!("IMG_{i}.jpg")), i + 1);
    }
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    let source = FolderSource::new(&src);
    let items = source.enumerate().unwrap();
    // Room above the buffer for the first two files and not a byte more.
    let room = budget::IMPORT_FREE_BUFFER + items[0].size + items[1].size;
    let opts = ImportOptions { free_space: Some(FreeSpace::new(move || Some(room))), ..Default::default() };
    let summary = run_import(&lib, &source, &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert!(summary.interrupted, "{summary:?}");
    assert_eq!(summary.imported, 2);
    let batch = &lib.db.with_conn(|c| list_batches(c, 10)).unwrap()[0];
    assert_eq!(batch.state, BATCH_INTERRUPTED);
    let failures = lib.db.with_conn(|c| list_failures(c, batch.id)).unwrap();
    assert_eq!(failures.len(), 1);
    assert!(failures[0].reason.contains("20 GB free"), "{failures:?}");
}
