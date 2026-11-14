//! Round 21 B2: the repair check. A source is judged against the library without reading a file
//! unless identity (name, size, folder) does not settle it. Every test builds its own library and
//! source in a temp dir.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use image::{ImageBuffer, Rgb};
use photos_core::Library;
use photos_import::repair::{self, RepairCheck, RepairClass, RepairOutcome, RepairStatus};
use photos_import::*;
use photos_jobs::CancelToken;

fn write_jpeg(path: &Path, seed: u8) {
    let img = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
    img.save(path).unwrap();
}

fn import(lib: &Library, src: &Path) {
    run_import(lib, &FolderSource::new(src), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
}

/// A folder source that remembers which files were opened (read).
struct Counting {
    inner: FolderSource,
    opened: Mutex<Vec<String>>,
    count: AtomicUsize,
}

impl Counting {
    fn new(root: &Path) -> Self {
        Self { inner: FolderSource::new(root), opened: Mutex::new(Vec::new()), count: AtomicUsize::new(0) }
    }
    fn opens(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
    fn opened(&self) -> Vec<String> {
        let mut v = self.opened.lock().unwrap().clone();
        v.sort();
        v
    }
}

impl ImportSource for Counting {
    fn label(&self) -> String {
        self.inner.label()
    }
    fn kind(&self) -> i64 {
        self.inner.kind()
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        self.inner.enumerate()
    }
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.opened.lock().unwrap().push(item.rel_path.clone());
        self.inner.open(item)
    }
    fn sidecars(&self, item: &SourceItem) -> Vec<SourceItem> {
        self.inner.sidecars(item)
    }
}

/// The tests' decoder: a file decodes when it begins `GOOD` or is a JPEG (begins `FF D8`).
fn probe(path: &Path, _name: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.starts_with(b"GOOD") || bytes.starts_with(&[0xFF, 0xD8]) { Ok(()) } else { Err("invalid data".into()) }
}

fn check(lib: &Library, source: &dyn ImportSource, only: Option<&[i64]>) -> RepairCheck {
    repair::check_source(source, lib, only, &probe, &|_: RepairStatus| {}, &CancelToken::new()).unwrap()
}

fn find<'a>(r: &'a RepairCheck, name: &str) -> &'a repair::RepairItem {
    r.items.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("{name} not in {:?}", r.items))
}

fn asset_id(lib: &Library, filename: &str) -> i64 {
    lib.db.with_conn(|c| Ok(c.query_row("SELECT id FROM assets WHERE filename = ?1", [filename], |r| r.get(0))?)).unwrap()
}

fn original(lib: &Library, filename: &str) -> std::path::PathBuf {
    let rel: String = lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE filename = ?1", [filename], |r| r.get(0))?)).unwrap();
    lib.originals_dir().join(rel)
}

fn sql(lib: &Library, statement: &str) {
    lib.db.with_conn(|c| Ok(c.execute(statement, [])?)).unwrap();
}

/// `filename`'s library original cut in half and flagged unreadable; returns the half.
fn truncate(lib: &Library, filename: &str) -> Vec<u8> {
    let path = original(lib, filename);
    let bytes = std::fs::read(&path).unwrap();
    let half = bytes[..bytes.len() / 2].to_vec();
    std::fs::write(&path, &half).unwrap();
    lib.db.with_conn(|c| Ok(c.execute("UPDATE assets SET unreadable = 1 WHERE filename = ?1", [filename])?)).unwrap();
    half
}

/// A library that imported `a.jpg`, `sub/b.jpg` and `c.jpg` (with `c.xmp`) from `card`.
fn setup() -> (tempfile::TempDir, std::path::PathBuf, Library) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    write_jpeg(&src.join("sub").join("b.jpg"), 2);
    write_jpeg(&src.join("c.jpg"), 3);
    std::fs::write(src.join("c.xmp"), b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src);
    (tmp, src, lib)
}

#[test]
fn an_intact_file_is_safe_and_is_not_read() {
    let (_tmp, src, lib) = setup();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    assert!(r.items.is_empty(), "{:?}", r.items);
    assert_eq!(r.counts.safe, 3);
    assert!(!r.cancelled);
    assert_eq!(source.opens(), 0, "identity settles it: no file is read");
}

#[test]
fn a_file_the_library_never_saw_is_new_and_hashed() {
    let (_tmp, src, lib) = setup();
    write_jpeg(&src.join("sub").join("d.jpg"), 4);
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    let d = find(&r, "d.jpg");
    assert_eq!((d.status, d.dir.as_str(), d.asset_id), (RepairClass::New, "sub", None));
    assert!(d.byte_size > 0);
    assert_eq!(r.counts.new, 1);
    assert_eq!(source.opened(), vec!["sub/d.jpg".to_string()], "only the file without an identity match is read");
}

#[test]
fn the_same_bytes_under_another_name_are_safe() {
    let (_tmp, src, lib) = setup();
    std::fs::copy(src.join("a.jpg"), src.join("a copy.jpg")).unwrap();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    assert!(r.items.is_empty(), "{:?}", r.items);
    assert_eq!(r.counts.safe, 4);
    assert_eq!(source.opened(), vec!["a copy.jpg".to_string()]);
}

#[test]
fn a_flagged_original_is_damaged() {
    let (_tmp, src, lib) = setup();
    truncate(&lib, "a.jpg");
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    let a = find(&r, "a.jpg");
    assert_eq!((a.status, a.asset_id), (RepairClass::Damaged, Some(asset_id(&lib, "a.jpg"))));
    assert_eq!((r.items.len(), r.counts.damaged, r.counts.safe), (1, 1, 2));
    assert_eq!(source.opened(), vec!["a.jpg".to_string()], "only the damaged candidate is read, to probe it");
}

#[test]
fn a_missing_original_is_damaged() {
    let (_tmp, src, lib) = setup();
    std::fs::remove_file(original(&lib, "b.jpg")).unwrap();
    let r = check(&lib, &Counting::new(&src), None);
    let b = find(&r, "b.jpg");
    assert_eq!((b.status, b.dir.as_str()), (RepairClass::Damaged, "sub"));
    assert_eq!(r.items.len(), 1);
}

#[test]
fn an_original_whose_size_changed_is_damaged() {
    let (_tmp, src, lib) = setup();
    let path = original(&lib, "a.jpg");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 10);
    std::fs::write(&path, bytes).unwrap();
    let r = check(&lib, &Counting::new(&src), None);
    assert_eq!(find(&r, "a.jpg").status, RepairClass::Damaged);
    assert_eq!(r.items.len(), 1);
}

#[test]
fn a_flagged_original_is_damaged_even_when_the_source_size_differs() {
    let (_tmp, src, lib) = setup();
    sql(&lib, "UPDATE assets SET unreadable = 1 WHERE filename = 'a.jpg'");
    // The source holds a different file of the same name and folder (e.g. a good copy).
    let mut bytes = std::fs::read(src.join("a.jpg")).unwrap();
    bytes.extend_from_slice(b"padding");
    std::fs::write(src.join("a.jpg"), bytes).unwrap();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    let a = find(&r, "a.jpg");
    assert_eq!((a.status, a.asset_id), (RepairClass::Damaged, Some(asset_id(&lib, "a.jpg"))));
    assert_eq!(source.opens(), 1, "name and folder settle it; only the probe reads it");
}

#[test]
fn a_changed_source_of_a_healthy_original_is_judged_by_its_bytes() {
    let (_tmp, src, lib) = setup();
    let mut bytes = std::fs::read(src.join("a.jpg")).unwrap();
    bytes.extend_from_slice(b"padding");
    std::fs::write(src.join("a.jpg"), bytes).unwrap();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    assert_eq!(find(&r, "a.jpg").status, RepairClass::New);
    assert_eq!(source.opened(), vec!["a.jpg".to_string()]);
}

#[test]
fn a_sidecar_the_library_lacks_is_listed() {
    let (_tmp, src, lib) = setup();
    // c.xmp was kept at import; a.aae appeared on the card later.
    std::fs::write(src.join("a.aae"), b"<plist/>").unwrap();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    let aae = find(&r, "a.aae");
    assert_eq!((aae.status, aae.asset_id), (RepairClass::Sidecar, Some(asset_id(&lib, "a.jpg"))));
    assert_eq!((r.items.len(), r.counts.sidecar), (1, 1));
    assert_eq!(source.opens(), 0, "a sidecar is told apart by name and kind");

    // The kept one goes missing from the catalogue: it is listed too.
    sql(&lib, "DELETE FROM asset_sidecars WHERE kind = 'xmp'");
    let r = check(&lib, &Counting::new(&src), None);
    assert_eq!(find(&r, "c.xmp").status, RepairClass::Sidecar);
    assert_eq!(r.counts.sidecar, 2);
}

#[test]
fn a_damaged_original_also_lists_its_missing_sidecars() {
    let (_tmp, src, lib) = setup();
    truncate(&lib, "c.jpg");
    sql(&lib, "DELETE FROM asset_sidecars");
    let r = check(&lib, &Counting::new(&src), None);
    assert_eq!(find(&r, "c.jpg").status, RepairClass::Damaged);
    assert_eq!(find(&r, "c.xmp").status, RepairClass::Sidecar);
}

#[test]
fn a_trashed_photo_is_in_trash_by_identity_and_by_bytes() {
    let (_tmp, src, lib) = setup();
    sql(&lib, "UPDATE assets SET state = 1, trashed_at = 1 WHERE filename IN ('a.jpg', 'sub/b.jpg', 'b.jpg')");
    std::fs::copy(src.join("sub").join("b.jpg"), src.join("b copy.jpg")).unwrap();
    let source = Counting::new(&src);
    let r = check(&lib, &source, None);
    assert_eq!(find(&r, "a.jpg").status, RepairClass::InTrash);
    assert_eq!(find(&r, "b copy.jpg").status, RepairClass::InTrash);
    assert_eq!(find(&r, "b.jpg").status, RepairClass::InTrash);
    assert_eq!((r.counts.in_trash, r.counts.safe), (3, 1));
    assert_eq!(source.opened(), vec!["b copy.jpg".to_string()], "only the renamed copy is read");
}

#[test]
fn only_assets_looks_at_those_photos_and_nothing_else() {
    let (_tmp, src, lib) = setup();
    sql(&lib, "UPDATE assets SET unreadable = 1 WHERE filename IN ('a.jpg', 'c.jpg')");
    write_jpeg(&src.join("new.jpg"), 9);
    let a = asset_id(&lib, "a.jpg");
    let source = Counting::new(&src);
    let r = check(&lib, &source, Some(&[a]));
    assert_eq!(r.items.len(), 1, "{:?}", r.items);
    // The flag alone: the source holds the very bytes of the library's copy.
    assert_eq!((find(&r, "a.jpg").status, find(&r, "a.jpg").asset_id), (RepairClass::SourceDamaged, Some(a)));
    assert_eq!(source.opened(), vec!["a.jpg".to_string()], "no hashing outside the asked-for photos; the probe reads only the candidate");
}

#[test]
fn a_cancelled_check_says_so_and_reads_nothing() {
    let (_tmp, src, lib) = setup();
    write_jpeg(&src.join("new.jpg"), 9);
    let cancel = CancelToken::new();
    cancel.cancel();
    let source = Counting::new(&src);
    let r = repair::check_source(&source, &lib, None, &probe, &|_: RepairStatus| {}, &cancel).unwrap();
    assert!(r.cancelled);
    assert_eq!(source.opens(), 0);
}

#[test]
fn progress_reports_every_file_and_the_check_writes_nothing() {
    let (_tmp, src, lib) = setup();
    write_jpeg(&src.join("new.jpg"), 9);
    let rows_before: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
    let a_before = std::fs::read(original(&lib, "a.jpg")).unwrap();
    let last = Mutex::new(None::<RepairStatus>);
    let r = repair::check_source(&Counting::new(&src), &lib, None, &probe, &|s| *last.lock().unwrap() = Some(s), &CancelToken::new()).unwrap();
    let last = last.into_inner().unwrap().expect("progress was reported");
    assert_eq!((last.done, last.total), (4, 4));
    assert_eq!(last.counts, r.counts);
    let rows_after: i64 = lib.db.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM assets", [], |r| r.get(0))?)).unwrap();
    assert_eq!(rows_after, rows_before);
    assert_eq!(std::fs::read(original(&lib, "a.jpg")).unwrap(), a_before);
}

// ---- B5: re-import and the swap -------------------------------------------------------------

fn apply(lib: &Library, source: &dyn ImportSource, items: &[repair::RepairItem]) -> repair::RepairResult {
    repair::apply(source, lib, "Card", items, &ImportOptions::default(), &probe, &CancelToken::new(), &mut |_| {}, &mut |_| {}, &mut |_| {}).unwrap()
}

fn scalar<T: rusqlite::types::FromSql>(lib: &Library, q: &str, id: i64) -> T {
    lib.db.with_conn(|c| Ok(c.query_row(q, [id], |r| r.get(0))?)).unwrap()
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// `a.jpg`'s library original cut in half and flagged, as Round 18 C1 found 52 of them.
fn truncate_a(lib: &Library) -> Vec<u8> {
    truncate(lib, "a.jpg")
}

fn damaged_a(lib: &Library, src: &Path) -> repair::RepairItem {
    let size = std::fs::metadata(src.join("a.jpg")).unwrap().len();
    repair::RepairItem { id: 1, name: "a.jpg".into(), dir: String::new(), status: RepairClass::Damaged, asset_id: Some(asset_id(lib, "a.jpg")), byte_size: size }
}

#[test]
fn a_repaired_photo_keeps_its_place() {
    let (tmp, src, lib) = setup();
    let id = asset_id(&lib, "a.jpg");
    let now = 1;
    lib.db
        .with_conn(|c| {
            c.execute("INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (7, 'Trip', 0, ?1, ?1)", [now])?;
            c.execute("INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (7, ?1, 1.0, 1)", [id])?;
            c.execute("INSERT INTO edits (asset_id, recipe_json, updated_at) VALUES (?1, '{\"exposure\":0.5}', 1)", [id])?;
            Ok(())
        })
        .unwrap();
    let half = truncate_a(&lib);
    let r = check(&lib, &Counting::new(&src), None);
    assert_eq!(find(&r, "a.jpg").status, RepairClass::Damaged);

    let good = std::fs::read(src.join("a.jpg")).unwrap();
    let out = apply(&lib, &FolderSource::new(&src), &r.items);
    assert_eq!((out.repaired, out.imported, out.skipped), (1, 0, 0), "{out:?}");
    assert_eq!(out.repaired_ids, vec![id]);

    // Same asset, same album, same edit; the good bytes in its place and the flag gone.
    assert_eq!(asset_id(&lib, "a.jpg"), id);
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM album_items WHERE album_id = 7 AND asset_id = ?1", id), 1);
    assert_eq!(scalar::<String>(&lib, "SELECT recipe_json FROM edits WHERE asset_id = ?1", id), "{\"exposure\":0.5}");
    assert_eq!(scalar::<i64>(&lib, "SELECT unreadable FROM assets WHERE id = ?1", id), 0);
    assert_eq!(std::fs::read(original(&lib, "a.jpg")).unwrap(), good);
    assert_eq!(scalar::<String>(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id), blake3::hash(&good).to_hex().to_string());
    assert_eq!(scalar::<i64>(&lib, "SELECT byte_size FROM assets WHERE id = ?1", id), good.len() as i64);

    // The damaged bytes are kept aside, never deleted; no temp file is left.
    let kept = files_under(&lib.root.join("kept-aside"));
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read(&kept[0]).unwrap(), half);
    let rel: String = scalar(&lib, "SELECT relative_path FROM assets WHERE id = ?1", id);
    assert!(kept[0].to_string_lossy().replace('\\', "/").ends_with(&rel), "{kept:?} keeps the path under originals");
    assert!(files_under(&lib.originals_dir()).iter().all(|p| !p.to_string_lossy().ends_with(".part")));
    assert_eq!(out.outcomes.len(), 1);
    assert_eq!((out.outcomes[0].outcome, out.outcomes[0].asset_id), (RepairOutcome::Repaired, Some(id)));

    // The replaced bytes are remembered: their hash, where they went, their size.
    let (old_hash, kept_rel, old_size): (String, Option<String>, i64) = lib
        .db
        .with_conn(|c| Ok(c.query_row("SELECT content_hash, kept_rel, old_byte_size FROM replaced_originals WHERE asset_id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?))
        .unwrap();
    assert_eq!(old_hash, blake3::hash(&half).to_hex().to_string());
    assert_eq!(old_size, half.len() as i64);
    assert_eq!(std::fs::read(lib.root.join(kept_rel.expect("a kept file"))).unwrap(), half);

    // Idempotent: a second check lists nothing.
    let again = check(&lib, &Counting::new(&src), None);
    assert!(again.items.is_empty(), "{:?}", again.items);
    drop(tmp);
}

/// A source whose `a.jpg` breaks off after a few bytes (a card pulled out mid-copy).
struct Failing(FolderSource, usize);

struct Broken(Box<dyn Read + Send>, usize, usize);
impl Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1 >= self.2 {
            return Err(std::io::Error::other("the card went away"));
        }
        let room = buf.len().min(self.2 - self.1);
        let n = self.0.read(&mut buf[..room])?;
        self.1 += n;
        Ok(n)
    }
}

impl ImportSource for Failing {
    fn label(&self) -> String {
        self.0.label()
    }
    fn kind(&self) -> i64 {
        self.0.kind()
    }
    fn enumerate(&self) -> Result<Vec<SourceItem>, ImportError> {
        self.0.enumerate()
    }
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>, ImportError> {
        Ok(Box::new(Broken(self.0.open(item)?, 0, self.1)))
    }
}

#[test]
fn a_source_that_stops_mid_copy_leaves_the_library_as_it_was() {
    let (_tmp, src, lib) = setup();
    let half = truncate_a(&lib);
    let id = asset_id(&lib, "a.jpg");
    let hash_before: String = scalar(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id);
    let files_before = files_under(&lib.originals_dir());

    let out = apply(&lib, &Failing(FolderSource::new(&src), 10), &[damaged_a(&lib, &src)]);
    assert_eq!((out.repaired, out.skipped), (0, 1), "{out:?}");
    assert_eq!(out.outcomes[0].outcome, RepairOutcome::Failed);
    assert!(out.outcomes[0].reason.as_deref().is_some_and(|r| r.contains("the card went away")), "{:?}", out.outcomes);
    assert_eq!(std::fs::read(original(&lib, "a.jpg")).unwrap(), half);
    assert_eq!(scalar::<String>(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id), hash_before);
    assert_eq!(scalar::<i64>(&lib, "SELECT unreadable FROM assets WHERE id = ?1", id), 1);
    assert_eq!(files_under(&lib.originals_dir()), files_before, "no temp file left behind");
    assert!(!lib.root.join("kept-aside").exists());
}

#[test]
fn bytes_that_belong_to_another_photo_are_not_swapped_in() {
    let (_tmp, src, lib) = setup();
    let half = truncate_a(&lib);
    std::fs::copy(src.join("sub").join("b.jpg"), src.join("a.jpg")).unwrap();
    let id = asset_id(&lib, "a.jpg");
    let hash_before: String = scalar(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id);
    let files_before = files_under(&lib.originals_dir());

    let out = apply(&lib, &FolderSource::new(&src), &[damaged_a(&lib, &src)]);
    assert_eq!((out.repaired, out.skipped), (0, 1), "{out:?}");
    assert!(out.notes.iter().any(|n| n.contains("already in the library as another photo")), "{:?}", out.notes);
    assert_eq!(out.outcomes[0].outcome, RepairOutcome::Duplicate);
    assert_eq!(std::fs::read(original(&lib, "a.jpg")).unwrap(), half);
    assert_eq!(scalar::<String>(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id), hash_before);
    assert_eq!(files_under(&lib.originals_dir()), files_before);
    assert!(!lib.root.join("kept-aside").exists());
}

#[test]
fn new_files_and_sidecars_go_through_import() {
    let (_tmp, src, lib) = setup();
    write_jpeg(&src.join("sub").join("d.jpg"), 4);
    sql(&lib, "DELETE FROM asset_sidecars");
    let r = check(&lib, &Counting::new(&src), None);
    assert_eq!(find(&r, "d.jpg").status, RepairClass::New);
    assert_eq!(find(&r, "c.xmp").status, RepairClass::Sidecar);

    let out = apply(&lib, &FolderSource::new(&src), &r.items);
    assert_eq!((out.imported, out.sidecars, out.repaired, out.skipped), (1, 1, 0, 0), "{out:?}");
    let batch = out.batch_id.expect("the new files have a batch");
    let label: String = scalar(&lib, "SELECT source_label FROM import_batches WHERE id = ?1", batch);
    assert_eq!(label, "Repair · Card");
    let mut kinds: Vec<_> = out.outcomes.iter().map(|o| (o.name.clone(), o.outcome)).collect();
    kinds.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(kinds, vec![("c.xmp".to_string(), RepairOutcome::Sidecar), ("d.jpg".to_string(), RepairOutcome::Imported)]);
    let d = asset_id(&lib, "d.jpg");
    assert_eq!(scalar::<i64>(&lib, "SELECT import_batch_id FROM assets WHERE id = ?1", d), batch);
    let c = asset_id(&lib, "c.jpg");
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM asset_sidecars WHERE asset_id = ?1 AND kind = 'xmp'", c), 1);

    let again = check(&lib, &Counting::new(&src), None);
    assert!(again.items.is_empty(), "{:?}", again.items);
}

#[test]
fn a_selection_takes_its_rows_and_never_a_trashed_one() {
    let item = |id: i64, status| repair::RepairItem { id, name: format!("{id}.jpg"), dir: String::new(), status, asset_id: None, byte_size: 1 };
    let items = vec![item(1, RepairClass::New), item(2, RepairClass::New), item(3, RepairClass::Damaged), item(4, RepairClass::InTrash)];
    let ids = |sel: repair::RepairSelection| repair::select(&items, &sel).iter().map(|i| i.id).collect::<Vec<_>>();
    assert_eq!(ids(serde_json::from_str(r#"{"all":true,"status":"new","except":[2]}"#).unwrap()), vec![1]);
    assert_eq!(ids(serde_json::from_str(r#"{"ids":[3,4,9]}"#).unwrap()), vec![3]);
    assert_eq!(ids(serde_json::from_str(r#"{"all":true,"status":"inTrash","except":[]}"#).unwrap()), Vec::<i64>::new());
}

// ---- Round 22 B1: the check and the swap stop cycling (§3 rules 1–4) -------------------------

/// A library imported from `card` holding the named cases, every original flagged unreadable:
/// `same` (the card holds the very bytes), `other` (the card holds other bytes that do not decode),
/// `good` (the card holds a good copy).
fn fixture(cases: &[&str]) -> (tempfile::TempDir, std::path::PathBuf, Library) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    for case in cases {
        let bytes: &[u8] = match *case {
            "same" => b"BAD1 same bytes",
            "other" => b"BAD2 library copy",
            "good" => b"BAD3 library copy",
            _ => unreachable!(),
        };
        std::fs::write(src.join(format!("{case}.jpg")), bytes).unwrap();
    }
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src);
    sql(&lib, "UPDATE assets SET unreadable = 1");
    for case in cases {
        match *case {
            "other" => std::fs::write(src.join("other.jpg"), b"BAD2 a different and longer copy").unwrap(),
            "good" => std::fs::write(src.join("good.jpg"), b"GOOD3 the good copy, longer").unwrap(),
            _ => {}
        }
    }
    (tmp, src, lib)
}

fn damaged_item(lib: &Library, src: &Path, name: &str) -> repair::RepairItem {
    let size = std::fs::metadata(src.join(name)).unwrap().len();
    repair::RepairItem { id: 1, name: name.into(), dir: String::new(), status: RepairClass::Damaged, asset_id: Some(asset_id(lib, name)), byte_size: size }
}

#[test]
fn an_identical_source_copy_is_source_damaged() {
    let (_tmp, src, lib) = fixture(&["same"]);
    let r = check(&lib, &FolderSource::new(&src), None);
    let item = find(&r, "same.jpg");
    assert_eq!((item.status, item.asset_id), (RepairClass::SourceDamaged, Some(asset_id(&lib, "same.jpg"))));
    assert_eq!((r.counts.source_damaged, r.counts.damaged), (1, 0));
}

#[test]
fn a_different_copy_that_does_not_decode_is_source_damaged() {
    let (_tmp, src, lib) = fixture(&["other"]);
    let r = check(&lib, &FolderSource::new(&src), None);
    assert_eq!(find(&r, "other.jpg").status, RepairClass::SourceDamaged);
    assert_eq!((r.counts.source_damaged, r.counts.damaged), (1, 0));
}

#[test]
fn a_good_copy_is_damaged_and_repairable() {
    let (_tmp, src, lib) = fixture(&["good"]);
    let r = check(&lib, &FolderSource::new(&src), None);
    assert_eq!(find(&r, "good.jpg").status, RepairClass::Damaged);
    assert_eq!((r.counts.source_damaged, r.counts.damaged), (0, 1));
    assert_eq!(repair::select(&r.items, &repair::RepairSelection::Ids { ids: vec![find(&r, "good.jpg").id] }).len(), 1);
}

#[test]
fn source_damaged_items_cannot_be_selected() {
    let item = |id: i64, status| repair::RepairItem { id, name: format!("{id}.jpg"), dir: String::new(), status, asset_id: Some(id), byte_size: 1 };
    let items = vec![item(1, RepairClass::SourceDamaged), item(2, RepairClass::SourceDamaged)];
    let all = repair::RepairSelection::All { all: true, status: RepairClass::SourceDamaged, except: vec![] };
    assert!(repair::select(&items, &all).is_empty());
    assert!(repair::select(&items, &repair::RepairSelection::Ids { ids: vec![1, 2] }).is_empty());
}

/// (files, bytes) under `kept-aside/`, and every asset's (hash, size, unreadable) by id.
type Snapshot = (usize, u64, Vec<(String, i64, i64)>);

fn snapshot(lib: &Library) -> Snapshot {
    let kept = files_under(&lib.root.join("kept-aside"));
    let bytes = kept.iter().map(|p| std::fs::metadata(p).unwrap().len()).sum();
    let rows = lib
        .db
        .with_conn(|c| {
            let mut st = c.prepare("SELECT content_hash, byte_size, unreadable FROM assets ORDER BY id")?;
            let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    (kept.len(), bytes, rows)
}

/// A check, then everything it lets the person tick, applied.
fn check_and_apply_all(lib: &Library, src: &Path) -> repair::RepairResult {
    let r = check(lib, &FolderSource::new(src), None);
    let ids = r.items.iter().map(|i| i.id).collect();
    let chosen: Vec<_> = repair::select(&r.items, &repair::RepairSelection::Ids { ids }).into_iter().cloned().collect();
    apply(lib, &FolderSource::new(src), &chosen)
}

#[test]
fn a_second_run_changes_nothing() {
    let (_tmp, src, lib) = fixture(&["same", "other", "good"]);
    let out = check_and_apply_all(&lib, &src);
    assert_eq!(out.repaired, 1, "{out:?}");
    let first = snapshot(&lib);
    assert_eq!(first.0, 1, "only the good-copy case set a file aside");
    let again = check_and_apply_all(&lib, &src);
    assert_eq!((again.repaired, again.imported), (0, 0), "{again:?}");
    assert_eq!(snapshot(&lib), first);
}

#[test]
fn swap_refuses_the_same_bytes() {
    let (_tmp, src, lib) = fixture(&["same"]);
    let out = apply(&lib, &FolderSource::new(&src), &[damaged_item(&lib, &src, "same.jpg")]);
    assert_eq!(out.outcomes[0].outcome, RepairOutcome::SameFile, "{out:?}");
    assert_eq!(out.repaired, 0);
    assert!(!lib.root.join("kept-aside").exists());
    assert!(files_under(&lib.originals_dir()).iter().all(|p| !p.to_string_lossy().ends_with(".part")));
}

#[test]
fn swap_refuses_a_copy_that_does_not_decode() {
    let (_tmp, src, lib) = fixture(&["other"]);
    let id = asset_id(&lib, "other.jpg");
    let out = apply(&lib, &FolderSource::new(&src), &[damaged_item(&lib, &src, "other.jpg")]);
    assert_eq!(out.outcomes[0].outcome, RepairOutcome::SourceDamaged, "{out:?}");
    assert_eq!(out.outcomes[0].reason.as_deref(), Some("invalid data"));
    assert_eq!(scalar::<i64>(&lib, "SELECT unreadable FROM assets WHERE id = ?1", id), 1);
    assert_eq!(std::fs::read(original(&lib, "other.jpg")).unwrap(), b"BAD2 library copy");
    assert!(!lib.root.join("kept-aside").exists());
    assert!(files_under(&lib.originals_dir()).iter().all(|p| !p.to_string_lossy().ends_with(".part")));
}

#[test]
fn a_replaced_file_on_the_source_is_not_offered_as_new() {
    let (_tmp, src, lib) = fixture(&["good"]);
    assert_eq!(check_and_apply_all(&lib, &src).repaired, 1);
    std::fs::write(src.join("old copy.jpg"), b"BAD3 library copy").unwrap();
    let r = check(&lib, &FolderSource::new(&src), None);
    assert!(r.items.is_empty(), "{:?}", r.items);
    assert_eq!(r.counts.safe, 2);
}

#[test]
fn a_swap_names_its_phases_and_its_file() {
    let (_tmp, src, lib) = fixture(&["good"]);
    let r = check(&lib, &FolderSource::new(&src), None);
    let seen = Mutex::new(Vec::<RepairStatus>::new());
    let out = repair::apply(
        &FolderSource::new(&src),
        &lib,
        "Card",
        &r.items,
        &ImportOptions::default(),
        &probe,
        &CancelToken::new(),
        &mut |_| {},
        &mut |_| {},
        &mut |s| seen.lock().unwrap().push(s.clone()),
    )
    .unwrap();
    assert_eq!(out.repaired, 1);
    let seen = seen.into_inner().unwrap();
    let phases: Vec<_> = seen.iter().map(|s| s.phase).collect();
    assert!(phases.contains(&repair::RepairPhase::Copying) && phases.contains(&repair::RepairPhase::Verifying), "{phases:?}");
    assert!(seen.iter().all(|s| s.current.as_deref() == Some("good.jpg") && s.total == 1), "{seen:?}");
    assert!(seen.iter().all(|s| s.bytes_total == 0), "a small file reports no bytes");
}

// --- Round 22 B3: keeping a recovery -------------------------------------------------------

/// A library with one PNG, its original cut in half and flagged; the asset's id, its row as a
/// string and the damaged bytes.
fn damaged_png() -> (tempfile::TempDir, Library, i64, Vec<u8>) {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("d.png"), 5);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src);
    let id = asset_id(&lib, "d.png");
    lib.db
        .with_conn(|c| {
            c.execute("INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (7, 'Trip', 0, 1, 1)", [])?;
            c.execute("INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (7, ?1, 1.0, 1)", [id])?;
            c.execute("UPDATE assets SET rating = 4, captured_at = 1234567 WHERE id = ?1", [id])?;
            Ok(())
        })
        .unwrap();
    let half = truncate(&lib, "d.png");
    // The catalogue knows the bytes as they are on disk (damaged at import).
    let hash = blake3::hash(&half).to_hex().to_string();
    lib.db.with_conn(|c| Ok(c.execute("UPDATE assets SET content_hash = ?2, byte_size = ?3 WHERE id = ?1", rusqlite::params![id, hash, half.len() as i64])?)).unwrap();
    (tmp, lib, id, half)
}

fn row(lib: &Library, id: i64) -> String {
    lib.db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT relative_path, extension, media_type, byte_size, content_hash, unreadable, recovered_pct, rating, captured_at FROM assets WHERE id = ?1",
                [id],
                |r| {
                    Ok(format!(
                        "{:?}",
                        (
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, i64>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, String>(4)?,
                            r.get::<_, i64>(5)?,
                            r.get::<_, Option<i64>>(6)?,
                            r.get::<_, Option<i64>>(7)?,
                            r.get::<_, Option<i64>>(8)?,
                        )
                    ))
                },
            )?)
        })
        .unwrap()
}

/// A recovered photo staged as `<library>/recovered/<id>.jpg`.
fn staged(lib: &Library, id: i64) -> (std::path::PathBuf, Vec<u8>) {
    let path = lib.root.join("recovered").join(format!("{id}.jpg"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_jpeg(&path, 9);
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
}

#[test]
fn keeping_a_recovery_keeps_the_photo() {
    let (_tmp, lib, id, half) = damaged_png();
    let (staged_path, recovered) = staged(&lib, id);

    repair::keep_recovered(&lib, id, &staged_path, 62, &probe).unwrap();

    assert_eq!(asset_id(&lib, "d.png"), id);
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM album_items WHERE album_id = 7 AND asset_id = ?1", id), 1);
    assert_eq!(scalar::<i64>(&lib, "SELECT rating FROM assets WHERE id = ?1", id), 4);
    assert_eq!(scalar::<i64>(&lib, "SELECT captured_at FROM assets WHERE id = ?1", id), 1234567);
    assert_eq!(scalar::<i64>(&lib, "SELECT recovered_pct FROM assets WHERE id = ?1", id), 62);
    assert_eq!(scalar::<i64>(&lib, "SELECT unreadable FROM assets WHERE id = ?1", id), 0);
    assert_eq!(scalar::<String>(&lib, "SELECT extension FROM assets WHERE id = ?1", id), "jpg");
    let rel: String = scalar(&lib, "SELECT relative_path FROM assets WHERE id = ?1", id);
    assert!(rel.ends_with(".jpg"), "{rel}");
    assert_eq!(std::fs::read(lib.originals_dir().join(&rel)).unwrap(), recovered);
    assert_eq!(scalar::<String>(&lib, "SELECT content_hash FROM assets WHERE id = ?1", id), blake3::hash(&recovered).to_hex().to_string());

    let kept = files_under(&lib.root.join("kept-aside"));
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(std::fs::read(&kept[0]).unwrap(), half);
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM replaced_originals WHERE asset_id = ?1", id), 1);
    assert!(files_under(&lib.originals_dir()).iter().all(|p| !p.to_string_lossy().ends_with(".part")));
}

#[test]
fn undo_restores_everything() {
    let (_tmp, lib, id, half) = damaged_png();
    let before = row(&lib, id);
    let original_path = original(&lib, "d.png");
    let (staged_path, recovered) = staged(&lib, id);
    repair::keep_recovered(&lib, id, &staged_path, 62, &probe).unwrap();
    std::fs::remove_file(&staged_path).unwrap();

    let back = repair::undo_replace(&lib, id).unwrap();

    assert_eq!(row(&lib, id), before);
    assert_eq!(std::fs::read(&original_path).unwrap(), half);
    assert_eq!(back, staged_path);
    assert_eq!(std::fs::read(&back).unwrap(), recovered);
    assert!(files_under(&lib.root.join("kept-aside")).is_empty());
    assert_eq!(files_under(&lib.originals_dir()).len(), 1);
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM replaced_originals WHERE asset_id = ?1", id), 0);
}

#[test]
fn discard_leaves_the_library_as_it_was() {
    let (_tmp, lib, id, half) = damaged_png();
    let before = row(&lib, id);
    let (staged_path, _) = staged(&lib, id);
    // Discard is the staged file's removal and nothing else.
    std::fs::remove_file(&staged_path).unwrap();
    assert_eq!(row(&lib, id), before);
    assert_eq!(std::fs::read(original(&lib, "d.png")).unwrap(), half);
    assert!(!lib.root.join("kept-aside").exists());
    assert_eq!(scalar::<i64>(&lib, "SELECT COUNT(*) FROM replaced_originals WHERE asset_id = ?1", id), 0);
}
