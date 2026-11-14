//! End-to-end on real files: import a folder into a scratch library and render every
//! derivative size. Ignored by default; run with
//! `LUMEN_E2E_FOLDER="C:/Users/you/Pictures/Screenshots" cargo test -p lumen --test e2e_import -- --ignored --nocapture`.

use std::path::PathBuf;
use std::time::Instant;

use photos_core::Library;
use photos_import::{run_import, FolderSource, ImportOptions};
use photos_jobs::CancelToken;
use photos_media::{DerivativeStore, Ffmpeg};

#[test]
#[ignore]
fn imports_a_real_folder_and_renders_derivatives() {
    let Some(folder) = std::env::var_os("LUMEN_E2E_FOLDER") else {
        eprintln!("LUMEN_E2E_FOLDER not set; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let lib = Library::create(&tmp.path().join("Lumen Library")).unwrap();
    let source = FolderSource::new(PathBuf::from(folder));
    let ffmpeg = Ffmpeg::locate();
    eprintln!("ffmpeg: {:?}", ffmpeg.as_ref().map(|f| f.version.as_str()));
    let mut ingested = Vec::new();
    let t0 = Instant::now();
    let summary = run_import(&lib, &source, &ImportOptions { ffmpeg: ffmpeg.clone(), ..Default::default() }, &CancelToken::new(), &mut |_| {}, &mut |i| ingested.push(i.clone())).unwrap();
    let import_secs = t0.elapsed().as_secs_f64();
    eprintln!("import: {summary:?} in {import_secs:.2}s ({:.1} files/s)", (summary.imported + summary.skipped) as f64 / import_secs.max(0.001));
    assert!(summary.imported > 0, "nothing imported from the folder");

    let store = DerivativeStore::new(&lib.root);
    let t1 = Instant::now();
    let mut ok = 0usize;
    let mut failed = 0usize;
    let mut misshapen = Vec::new();
    for i in &ingested {
        // The same order as the app: ingest decides orientation against the decoded
        // pixels and records it, and every later size renders from the recorded value.
        if let Err(e) = lumen_lib::derivatives::ingest_now(&lib, ffmpeg.as_ref(), i) {
            failed += 1;
            eprintln!("ingest failed for {}: {:?}", i.original.display(), e.detail);
            continue;
        }
        let info = lib.db.with_conn(|c| photos_core::assets::render_info(c, i.asset_id)).unwrap();
        for size in [256u32, 1024, 2560] {
            let spec = lumen_lib::derivatives::RenderSpec {
                hash: &i.content_hash,
                source: &i.original,
                kind: i.kind,
                orientation: info.orientation as u16,
                hdr: photos_media::hdr::HdrKind::from_code(i.hdr),
                edit: None,
                ai_rasters: None,
            };
            match lumen_lib::derivatives::render(&store, size, &spec, ffmpeg.as_ref()) {
                Ok(p) => {
                    assert!(p.is_file());
                    ok += 1;
                    // The picture must have the shape the catalogue lays it out with, or
                    // the grid reserves a portrait slot for a landscape image (or the
                    // image itself is on its side, as an iPhone import once was).
                    if size == 2560 {
                        let (dw, dh) = image::image_dimensions(&p).unwrap();
                        let (sw, sh) = (info.width.unwrap_or(0), info.height.unwrap_or(0));
                        let (lw, lh) = if photos_media::orientation_swaps(info.orientation as u16) { (sh, sw) } else { (sw, sh) };
                        eprintln!(
                            "  {:<40} stored {sw}x{sh} o={} -> derivative {dw}x{dh}",
                            i.original.file_name().unwrap().to_string_lossy(),
                            info.orientation
                        );
                        if (lw > lh) != (dw > dh) && lw != lh {
                            misshapen.push(i.original.display().to_string());
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("render failed for {}: {:?}", i.original.display(), e.detail);
                }
            }
        }
    }
    assert!(misshapen.is_empty(), "derivatives turned relative to the catalogue: {misshapen:?}");
    let render_secs = t1.elapsed().as_secs_f64();
    eprintln!("derivatives: {ok} ok, {failed} failed in {render_secs:.2}s ({:.1} images/s incl. 3 sizes)", ingested.len() as f64 / render_secs.max(0.001));
    let hdr: Vec<(i64, i64)> = lib
        .db
        .with_conn(|c| {
            let mut stmt = c.prepare("SELECT hdr, COUNT(*) FROM assets GROUP BY hdr ORDER BY hdr")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .unwrap();
    eprintln!("hdr (0 none, 1 gain map, 2 PQ, 3 HLG): {hdr:?}");
    let stats = lib.db.with_conn(photos_core::assets::stats).unwrap();
    eprintln!("stats: {stats:?}");
    let idx = lib.db.with_conn(|c| photos_core::layout::LayoutIndex::load(c, false)).unwrap();
    let layout = idx.compute(1280, 200, 4, Some(photos_core::layout::GroupBy::Month));
    eprintln!("layout: {} rows, {} sections, {} px tall", layout.rows.len(), layout.summary.sections.len(), layout.summary.total_height);

    // Phase 5: edit every still, export it, and check the export has the edit's shape. RAW
    // "full size" is the embedded preview, so shapes are compared, not pixel counts.
    let recipe = photos_core::edits::Recipe {
        rotate: 90,
        develop: Some(photos_core::edits::Develop {
            basic: photos_core::edits::Basic { exposure: 0.3, ..Default::default() },
            ..Default::default()
        }),
        ..photos_core::edits::Recipe::identity()
    };
    let exports = tmp.path().join("exports");
    std::fs::create_dir_all(&exports).unwrap();
    let options = photos_core::export::ExportOptions { format: photos_core::export::ExportFormat::Jpeg, quality: 90, lossless: false, max_long_edge: None, metadata: false, destination: Default::default(), video_cut: Default::default() };
    let t2 = Instant::now();
    let mut exported = 0usize;
    let mut wrong_shape = Vec::new();
    for i in ingested.iter().filter(|i| i.kind != photos_media::MediaKind::Video) {
        lib.db.with_conn(|c| photos_core::edits::save_recipe(c, i.asset_id, &recipe, 1)).unwrap();
        let info = lib.db.with_conn(|c| photos_core::assets::render_info(c, i.asset_id)).unwrap();
        let spec = lumen_lib::derivatives::RenderSpec::from_info(&info, &i.original);
        let out = exports.join(format!("{}.jpg", i.asset_id));
        match lumen_lib::derivatives::render_export(&spec, ffmpeg.as_ref(), &options, None, None, &out) {
            Ok(_) => {
                exported += 1;
                let (w, h) = (info.width.unwrap_or(1) as u32, info.height.unwrap_or(1) as u32);
                let (w, h) = if photos_media::orientation_swaps(info.orientation as u16) { (h, w) } else { (w, h) };
                let (ew, eh) = recipe.output_size(w, h);
                let (dw, dh) = image::image_dimensions(&out).unwrap();
                if ((dw as f64 / dh as f64) / (ew as f64 / eh as f64) - 1.0).abs() > 0.02 {
                    wrong_shape.push(format!("{}: {dw}x{dh}, expected the shape of {ew}x{eh}", i.original.display()));
                }
            }
            Err(e) => eprintln!("export failed for {}: {:?}", i.original.display(), e.detail),
        }
    }
    let export_secs = t2.elapsed().as_secs_f64();
    eprintln!("export: {exported} edited stills in {export_secs:.2}s ({:.1} photos/s, full size, JPEG q90)", exported as f64 / export_secs.max(0.001));
    assert!(wrong_shape.is_empty(), "exports with the wrong shape: {wrong_shape:?}");
    assert_eq!(failed, 0, "{failed} derivatives failed");
}
