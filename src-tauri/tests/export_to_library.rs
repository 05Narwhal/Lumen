//! Phase 8b, *Add to library*: an export is imported like any chosen file and remembers the
//! photo it was made from. `start_export` needs a running app, so this composes the same
//! public steps it takes: `render_export` with metadata, a `FilesSource` import, and
//! `link_derived` by content hash.
//!
//! `cargo test -p lumen --test export_to_library -- --ignored` (needs ExifTool; set
//! `LUMEN_EXIFTOOL_DIR` or run `scripts/fetch-exiftool`).

use photos_core::export::{ExportFormat, ExportMeta, ExportOptions};
use photos_core::{assets, Library};
use photos_import::{content_hash, run_import, FilesSource, ImportOptions};
use photos_jobs::CancelToken;
use photos_media::ExifTool;

#[test]
#[ignore = "needs ExifTool"]
fn an_export_added_to_the_library_keeps_its_date_and_its_source() {
    let tool = ExifTool::locate().expect("ExifTool");
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::create(&tmp.path().join("lib")).unwrap();

    // The source: a photo dated by its camera, imported.
    let camera = tmp.path().join("DSC_0042.jpg");
    image::RgbImage::from_fn(400, 300, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])).save(&camera).unwrap();
    let mut s = tool.session().unwrap();
    let out = s.run(["-overwrite_original", "-ExifIFD:DateTimeOriginal=2021:06:12 07:45:10", "-IFD0:Model=Z 6_2", camera.to_str().unwrap()]).unwrap();
    assert!(out.errors().is_empty(), "{out:?}");
    drop(s);
    let mut first = Vec::new();
    run_import(&lib, &FilesSource::new(vec![camera]), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |i| first.push(i.clone())).unwrap();
    let source = &first[0];
    let source_detail = lib.db.with_conn(|c| assets::get_asset(c, source.asset_id, false)).unwrap();

    // Export it as PNG (a format with no EXIF of its own) beside the library, as start_export does.
    let dir = lib.root.join("tmp").join("export-1");
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("DSC_0042.png");
    let info = lib.db.with_conn(|c| assets::render_info(c, source.asset_id)).unwrap();
    let spec = lumen_lib::derivatives::RenderSpec::from_info(&info, &source.original);
    let options = ExportOptions { format: ExportFormat::Png, quality: 90, lossless: false, max_long_edge: None, metadata: true, destination: Default::default(), video_cut: Default::default() };
    let carrier = lumen_lib::derivatives::Carrier::new(tool);
    let lost = lumen_lib::derivatives::render_export(&spec, None, &options, Some((&carrier, &ExportMeta::default())), None, &out).unwrap();
    assert_eq!(lost, Some(vec![]));

    // Import and link, as `add_exports_to_library` does.
    let hash = content_hash(&out).unwrap();
    let mut second = Vec::new();
    let summary = run_import(&lib, &FilesSource::new(vec![out.clone()]), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |i| second.push(i.clone())).unwrap();
    assert_eq!(summary.imported, 1);
    assert!(lib.db.with_conn(|c| assets::link_derived(c, &hash, source.asset_id)).unwrap());
    let exported = lib.db.with_conn(|c| assets::get_asset(c, second[0].asset_id, false)).unwrap();
    assert_eq!(exported.derived_from.as_ref().map(|d| (d.id, d.filename.as_str())), Some((source.asset_id, "DSC_0042.jpg")));
    assert_eq!(exported.captured_at, source_detail.captured_at, "dated by the carried EXIF, not by the moment of export");
    assert!(exported.captured_at.is_some());

    // The same bytes again are a duplicate, not a second photo.
    let again = run_import(&lib, &FilesSource::new(vec![out]), &ImportOptions::default(), &CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap();
    assert_eq!((again.imported, again.skipped), (0, 1));
}
