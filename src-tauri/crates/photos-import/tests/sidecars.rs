//! Round 19 C3: sidecars travel with their photo, XMP marks reach the catalogue, and nothing a
//! source holds is dropped without a row. Everything runs in temp dirs.

use std::path::Path;

use image::{ImageBuffer, Rgb};
use photos_core::sidecars::{self, SKIP_DUPLICATE, SKIP_UNSUPPORTED};
use photos_core::Library;
use photos_import::*;
use photos_jobs::CancelToken;

fn write_jpeg(path: &Path, seed: u8) {
    let img = ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x as u8).wrapping_mul(seed), y as u8, seed]));
    img.save(path).unwrap();
}

const XMP: &str = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmp:Rating="4" xmp:Label="Red"><dc:subject><rdf:Bag><rdf:li>beach</rdf:li><rdf:li>sunset</rdf:li></rdf:Bag></dc:subject><dc:title><rdf:Alt><rdf:li xml:lang="x-default">Golden hour</rdf:li></rdf:Alt></dc:title><dc:description><rdf:Alt><rdf:li xml:lang="x-default">The bay at dusk</rdf:li></rdf:Alt></dc:description></rdf:Description></rdf:RDF></x:xmpmeta>"#;

fn import(lib: &Library, src: &Path) -> ImportSummary {
    run_import(lib, &FolderSource::new(src), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap()
}

type Marks = (i64, i64, Option<String>, Option<String>, Option<String>);

fn marks_of(lib: &Library, filename: &str) -> Marks {
    lib.db
        .with_conn(|c| {
            Ok(c.query_row("SELECT rating, color_label, title, caption, keywords FROM assets WHERE filename = ?1", [filename], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?)
        })
        .unwrap()
}

fn asset_id(lib: &Library, filename: &str) -> i64 {
    lib.db.with_conn(|c| Ok(c.query_row("SELECT id FROM assets WHERE filename = ?1", [filename], |r| r.get(0))?)).unwrap()
}

#[test]
fn sidecars_are_copied_beside_the_original_and_their_marks_applied() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    std::fs::write(src.join("a.xmp"), XMP).unwrap();
    std::fs::write(src.join("a.AAE"), b"aae-bytes").unwrap();
    std::fs::write(src.join("a.thm"), b"thm-bytes").unwrap();
    write_jpeg(&src.join("b.jpg"), 2);
    std::fs::write(src.join("b.jpg.xmp"), XMP.replace("Rating=\"4\"", "Rating=\"2\"")).unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    let summary = import(&lib, &src);
    assert_eq!((summary.imported, summary.failed), (2, 0), "{summary:?}");

    let list = lib.db.with_conn(|c| sidecars::sidecars_of(c, asset_id(&lib, "a.jpg"))).unwrap();
    assert_eq!(list.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(), ["aae", "thm", "xmp"]);
    for s in &list {
        let copy = lib.originals_dir().join(&s.relative_path);
        assert!(copy.is_file(), "{}", copy.display());
        assert!(s.relative_path.ends_with(&format!(".{}", s.kind)), "{}", s.relative_path);
    }
    let aae = list.iter().find(|s| s.kind == "aae").unwrap();
    assert_eq!(std::fs::read(lib.originals_dir().join(&aae.relative_path)).unwrap(), b"aae-bytes");
    assert_eq!(aae.source_name, "a.AAE");

    let (rating, label, title, caption, keywords) = marks_of(&lib, "a.jpg");
    assert_eq!((rating, label), (4, 1));
    assert_eq!((title.as_deref(), caption.as_deref()), (Some("Golden hour"), Some("The bay at dusk")));
    assert_eq!(keywords.as_deref(), Some("beach\nsunset"));

    // `IMG_0001.jpg.xmp` (Lightroom's and darktable's spelling) belongs to `IMG_0001.jpg` too.
    assert_eq!(marks_of(&lib, "b.jpg").0, 2);
    assert_eq!(lib.db.with_conn(|c| sidecars::sidecars_of(c, asset_id(&lib, "b.jpg"))).unwrap().len(), 1);
}

#[test]
fn a_files_own_xmp_packet_gives_its_marks_and_a_sidecar_wins_over_it() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    // A Lightroom export: the packet sits inside the file.
    write_jpeg(&src.join("export.jpg"), 3);
    let mut bytes = std::fs::read(src.join("export.jpg")).unwrap();
    bytes.extend_from_slice(XMP.as_bytes());
    std::fs::write(src.join("export.jpg"), &bytes).unwrap();
    // The same, with a sidecar that disagrees about the stars.
    write_jpeg(&src.join("both.jpg"), 4);
    let mut bytes = std::fs::read(src.join("both.jpg")).unwrap();
    bytes.extend_from_slice(XMP.as_bytes());
    std::fs::write(src.join("both.jpg"), &bytes).unwrap();
    std::fs::write(src.join("both.xmp"), XMP.replace("Rating=\"4\"", "Rating=\"5\"")).unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    import(&lib, &src);

    let (rating, label, title, ..) = marks_of(&lib, "export.jpg");
    assert_eq!((rating, label, title.as_deref()), (4, 1, Some("Golden hour")));
    assert_eq!(marks_of(&lib, "both.jpg").0, 5);
}

#[test]
fn every_file_that_is_not_imported_is_listed_with_its_reason() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    write_jpeg(&src.join("a.jpg"), 1);
    std::fs::copy(src.join("a.jpg"), src.join("sub/a-copy.jpg")).unwrap();
    std::fs::write(src.join("a.xmp"), XMP).unwrap(); // has a partner: kept, not a leftover
    std::fs::write(src.join("lonely.xmp"), XMP).unwrap(); // no partner
    std::fs::write(src.join("notes.txt"), b"hello").unwrap();
    std::fs::write(src.join(".hidden.txt"), b"hidden").unwrap(); // hidden files are the system's, not the person's
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    let summary = import(&lib, &src);
    assert_eq!((summary.imported, summary.skipped), (1, 1), "{summary:?}");

    let skips = lib.db.with_conn(|c| sidecars::skips_of(c, summary.batch_id)).unwrap();
    let got: Vec<(&str, &str)> = skips.iter().map(|s| (s.path.as_str(), s.reason.as_str())).collect();
    assert_eq!(got, [("sub/a-copy.jpg", SKIP_DUPLICATE), ("lonely.xmp", SKIP_UNSUPPORTED), ("notes.txt", SKIP_UNSUPPORTED)]);
    assert_eq!(skips[0].asset_id, Some(asset_id(&lib, "a.jpg")));
    assert_eq!(summary.skip_records, 3);
}
