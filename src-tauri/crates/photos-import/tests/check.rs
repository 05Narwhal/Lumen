//! Round 19 C4: *Check before deleting*. A folder is judged file by file against the library
//! (Safe / Not in the library / Other), the library's own copy is re-read, and the whole
//! library can be re-hashed against its catalogue. Everything runs in temp dirs and nothing a
//! check looks at may change.

use std::path::{Path, PathBuf};

use image::{ImageBuffer, Rgb};
use photos_core::Library;
use photos_import::check::{self, CheckVerdict};
use photos_import::*;
use photos_jobs::CancelToken;

fn write_jpeg(path: &Path, seed: u8) {
    let img = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
    img.save(path).unwrap();
}

fn import(lib: &Library, src: &Path, opts: ImportOptions) -> ImportSummary {
    run_import(lib, &FolderSource::new(src), &opts, &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap()
}

fn status_of(r: &check::FolderCheck, name: &str) -> (CheckVerdict, Option<String>) {
    let item = r.items.iter().find(|i| i.path == name).unwrap_or_else(|| panic!("{name} not in {:?}", r.items));
    (item.status, item.reason.clone())
}

/// Every file under the library root with its bytes, and every asset row: the "nothing changed"
/// proof.
fn snapshot(lib: &Library) -> (Vec<(PathBuf, Vec<u8>)>, Vec<String>) {
    let mut files = Vec::new();
    for e in walkdir::WalkDir::new(&lib.root).into_iter().flatten().filter(|e| e.file_type().is_file()) {
        let name = e.file_name().to_string_lossy().to_string();
        // The database and its journal are written by opening the library, not by a check.
        if name.starts_with("library.db") {
            continue;
        }
        files.push((e.path().to_path_buf(), std::fs::read(e.path()).unwrap()));
    }
    files.sort();
    let rows = lib
        .db
        .with_conn(|c| {
            let mut st = c.prepare(
                "SELECT id, relative_path, content_hash, state, rating FROM assets ORDER BY id",
            )?;
            let rows = st
                .query_map([], |r| {
                    Ok(format!("{}|{}|{}|{}|{}", r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    (files, rows)
}

fn never_cancelled() -> CancelToken {
    CancelToken::new()
}

fn quiet(_: check::CheckProgress) {}

#[test]
fn a_folder_is_judged_file_by_file_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    std::fs::write(src.join("a.xmp"), b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
    write_jpeg(&src.join("sub").join("d.jpg"), 4);
    write_jpeg(&src.join("bad.jpg"), 7);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    fn flip_a_byte(item: &SourceItem, staged: &Path) {
        if item.file_name == "bad.jpg" {
            let mut bytes = std::fs::read(staged).unwrap();
            bytes[100] ^= 0xFF;
            std::fs::write(staged, bytes).unwrap();
        }
    }
    let summary = import(&lib, &src, ImportOptions { before_verify: Some(flip_a_byte), ..Default::default() });
    assert_eq!((summary.imported, summary.failed), (2, 1), "{summary:?}");

    // Added after the import: never brought in.
    write_jpeg(&src.join("b.jpg"), 2);
    std::fs::write(src.join("notes.txt"), b"hello").unwrap();
    std::fs::write(src.join("orphan.xmp"), b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" a=\"orphan\"/>").unwrap();
    // A file the library has, but whose library copy was damaged afterwards.
    let damaged = lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE filename = 'd.jpg'", [], |r| r.get::<_, String>(0))?)).unwrap();
    let copy = lib.originals_dir().join(&damaged);
    let mut bytes = std::fs::read(&copy).unwrap();
    bytes[100] ^= 0xFF;
    std::fs::write(&copy, bytes).unwrap();

    let before = snapshot(&lib);
    let src_before: Vec<_> = walkdir::WalkDir::new(&src).into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| (e.path().to_path_buf(), std::fs::read(e.path()).unwrap())).collect();

    let r = check::check_folder(&lib, &src, &never_cancelled(), &quiet).unwrap();

    assert_eq!(status_of(&r, "a.jpg"), (CheckVerdict::Safe, None));
    assert_eq!(status_of(&r, "a.xmp"), (CheckVerdict::Safe, None));
    assert_eq!(status_of(&r, "b.jpg"), (CheckVerdict::NotInLibrary, Some("not_imported".into())));
    assert_eq!(status_of(&r, "bad.jpg"), (CheckVerdict::NotInLibrary, Some("failed_verify".into())));
    assert_eq!(status_of(&r, "sub/d.jpg"), (CheckVerdict::NotInLibrary, Some("copy_differs".into())));
    assert_eq!(status_of(&r, "orphan.xmp"), (CheckVerdict::NotInLibrary, Some("sidecar_not_kept".into())));
    assert_eq!(status_of(&r, "notes.txt").0, CheckVerdict::Other);
    assert_eq!((r.safe, r.not_in_library, r.other), (2, 4, 1));
    assert!(!r.cancelled && !r.is_safe_to_delete());

    assert_eq!(snapshot(&lib), before, "a check changes nothing in the library");
    let src_after: Vec<_> = walkdir::WalkDir::new(&src).into_iter().flatten().filter(|e| e.file_type().is_file()).map(|e| (e.path().to_path_buf(), std::fs::read(e.path()).unwrap())).collect();
    assert_eq!(src_after, src_before, "a check never touches the folder");
}

#[test]
fn a_folder_whose_every_file_is_safe_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    write_jpeg(&src.join("b.jpg"), 2);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src, ImportOptions::default());
    // A copy of a photo under another name is the same bytes, so it is safe too.
    std::fs::copy(src.join("a.jpg"), src.join("a copy.jpg")).unwrap();

    let r = check::check_folder(&lib, &src, &never_cancelled(), &quiet).unwrap();
    assert_eq!((r.safe, r.not_in_library, r.other), (3, 0, 0), "{r:?}");
    assert!(r.is_safe_to_delete());
}

#[test]
fn a_photo_in_the_trash_is_not_safe() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src, ImportOptions::default());
    lib.db.with_conn(|c| Ok(c.execute("UPDATE assets SET state = 1, trashed_at = 5", [])?)).unwrap();

    let r = check::check_folder(&lib, &src, &never_cancelled(), &quiet).unwrap();
    assert_eq!(status_of(&r, "a.jpg"), (CheckVerdict::NotInLibrary, Some("in_trash".into())));
}

#[test]
fn a_cancelled_check_says_so_and_is_not_a_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src, ImportOptions::default());

    let cancel = CancelToken::new();
    cancel.cancel();
    let r = check::check_folder(&lib, &src, &cancel, &quiet).unwrap();
    assert!(r.cancelled);
    assert!(!r.is_safe_to_delete());
}

#[test]
fn the_library_is_re_hashed_against_its_catalogue() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    write_jpeg(&src.join("b.jpg"), 2);
    write_jpeg(&src.join("c.jpg"), 3);
    std::fs::write(src.join("c.xmp"), b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>").unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src, ImportOptions::default());

    let clean = check::check_library(&lib, &never_cancelled(), &quiet).unwrap();
    assert_eq!((clean.checked, clean.problems.len(), clean.cancelled), (4, 0, false), "{clean:?}");
    assert!(clean.is_intact());

    let rel = |name: &str| -> PathBuf {
        let r: String = lib.db.with_conn(|c| Ok(c.query_row("SELECT relative_path FROM assets WHERE filename = ?1", [name], |r| r.get(0))?)).unwrap();
        lib.originals_dir().join(r)
    };
    let a = rel("a.jpg");
    let mut bytes = std::fs::read(&a).unwrap();
    bytes[100] ^= 0xFF;
    std::fs::write(&a, bytes).unwrap();
    std::fs::remove_file(rel("b.jpg")).unwrap();
    let xmp = rel("c.jpg").with_extension("xmp");
    std::fs::remove_file(&xmp).unwrap();

    let before = snapshot(&lib);
    let r = check::check_library(&lib, &never_cancelled(), &quiet).unwrap();
    let find = |name: &str| r.problems.iter().find(|p| p.filename == name).unwrap_or_else(|| panic!("{name} in {:?}", r.problems));
    assert_eq!(find("a.jpg").status, check::CheckProblem::Changed);
    assert_eq!(find("b.jpg").status, check::CheckProblem::Missing);
    assert_eq!(find("c.xmp").status, check::CheckProblem::Missing);
    assert_eq!(r.problems.len(), 3);
    assert!(!r.is_intact());
    assert_eq!(snapshot(&lib), before);
}

/// Round 22 final seam (d): a copy made for an external editor is the editor's to change, so
/// *Check the library* leaves the paths `external_edits` names alone.
#[test]
fn the_library_check_skips_the_paths_external_edits_names() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("card");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    write_jpeg(&src.join("b.jpg"), 2);
    write_jpeg(&src.join("c.jpg"), 3);
    let lib = Library::create(&tmp.path().join("lib")).unwrap();
    import(&lib, &src, ImportOptions::default());
    let row = |name: &str| -> (i64, String) {
        lib.db.with_conn(|c| Ok(c.query_row("SELECT id, relative_path FROM assets WHERE filename = ?1", [name], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap()
    };
    let (a, a_rel) = row("a.jpg");
    let (b, b_rel) = row("b.jpg");
    let (_, c_rel) = row("c.jpg");
    lib.db
        .with_conn(|c| {
            photos_core::external_edits::insert_copy(
                c,
                &photos_core::external_edits::NewCopy {
                    source_asset_id: a,
                    asset_id: Some(b),
                    app_id: "photoshop",
                    relative_path: &b_rel,
                    made_key: "k",
                    made_hash: "h",
                    seen_size: 1,
                    seen_mtime: 1,
                    created_at: 1,
                },
            )?;
            Ok(())
        })
        .unwrap();
    // The editor saved over one copy; the other copy's file is gone; a plain original is damaged.
    let originals = lib.originals_dir();
    let mut bytes = std::fs::read(originals.join(&b_rel)).unwrap();
    bytes[100] ^= 0xFF;
    std::fs::write(originals.join(&b_rel), bytes).unwrap();
    std::fs::remove_file(originals.join(&c_rel)).unwrap();
    let mut bytes = std::fs::read(originals.join(&a_rel)).unwrap();
    bytes[100] ^= 0xFF;
    std::fs::write(originals.join(&a_rel), bytes).unwrap();

    let r = check::check_library(&lib, &never_cancelled(), &quiet).unwrap();
    let names: Vec<_> = r.problems.iter().map(|p| p.filename.as_str()).collect();
    assert_eq!(names, vec!["a.jpg", "c.jpg"], "the edited copy is not a problem: {names:?}");
    assert_eq!(r.checked, 2, "and it is not counted");
}
