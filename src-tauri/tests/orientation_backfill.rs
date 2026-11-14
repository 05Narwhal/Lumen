//! Replays the round-3 orientation backfill over a **copy** of a real catalogue:
//!
//! ```sh
//! LUMEN_DB=<scratchpad copy of library.db> cargo test -p lumen --test orientation_backfill -- --ignored --nocapture
//! ```
//!
//! Never point `LUMEN_DB` at a live library: this writes to the database it opens.

use std::path::PathBuf;

#[test]
#[ignore = "needs LUMEN_DB, a copy of a real library.db"]
fn the_backfill_is_idempotent() {
    let path = PathBuf::from(std::env::var("LUMEN_DB").expect("LUMEN_DB"));
    assert!(
        !path.components().any(|c| c.as_os_str() == "Lumen Library"),
        "refusing to write to what looks like a live library: copy library.db first"
    );
    let db = photos_core::Db::open(&path).unwrap();
    let first = lumen_lib::maintenance::refix_orientation_in(&db);
    println!("reset {first} photos that had been turned twice");
    let second = lumen_lib::maintenance::refix_orientation_in(&db);
    println!("second pass: {second}");
    assert_eq!(second, 0, "a second pass finds nothing");
}

/// Renders a few of the photos the backfill resets, from their originals (read only), the
/// way ingest now does, so they can be looked at:
/// `LUMEN_DB=<copy> LUMEN_ORIGINALS=<library>/originals LUMEN_OUT=<scratch dir> LUMEN_FFMPEG_DIR=<repo>/src-tauri/binaries`.
#[test]
#[ignore = "needs LUMEN_DB, LUMEN_ORIGINALS, LUMEN_OUT and FFmpeg"]
fn the_reset_photos_render_upright() {
    let db = photos_core::Db::open(&PathBuf::from(std::env::var("LUMEN_DB").unwrap())).unwrap();
    let originals = PathBuf::from(std::env::var("LUMEN_ORIGINALS").unwrap());
    let out = PathBuf::from(std::env::var("LUMEN_OUT").unwrap());
    let ffmpeg = photos_media::Ffmpeg::locate().expect("FFmpeg");
    let rows: Vec<(String, String, i64)> = db
        .with_conn(|c| {
            let mut st = c.prepare("SELECT relative_path, extension, orientation FROM assets WHERE extension = 'heic' AND json_extract(metadata_json, '$.orientation') = 3 LIMIT 6")?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .unwrap();
    for (i, (rel, ext, stored)) in rows.iter().enumerate() {
        let kind = photos_media::kind_for_extension(ext);
        let img = photos_media::decode_preview_with(&originals.join(rel), kind, Some(&ffmpeg)).unwrap();
        let o = photos_media::effective_orientation(3, None, (img.width(), img.height()), photos_media::decoder_orients(kind));
        println!("{rel}: stored {stored}, owed now {o}");
        photos_media::resize_long_edge(&photos_media::apply_orientation(img, o), 400).to_rgb8().save(out.join(format!("upright-{i}.jpg"))).unwrap();
    }
}
