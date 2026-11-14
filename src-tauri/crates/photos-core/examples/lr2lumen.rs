//! `cargo run -p photos-core --example lr2lumen -- <file.xmp|.lrtemplate|.lua> [--as-shot-kelvin N]`
//! prints what Lumen makes of a Lightroom preset or sidecar: `lightroom::Converted` as JSON.
//!
//! `… -- <catalog.lrcat> --image <id_local> [--as-shot-kelvin N]` prints one photo's develop
//! settings from a Lightroom Classic catalog. The catalog is copied to a temp folder (with its
//! `-wal` / `-shm` / `-journal`) and opened read-only there: the original is never opened.
//!
//! Session 11B's dataset builder calls this, so Python and Rust share one mapping.

use std::path::{Path, PathBuf};

use photos_core::lightroom::{convert, read_lua_table, read_settings};

fn main() {
    if let Err(e) = run() {
        eprintln!("lr2lumen: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut file: Option<PathBuf> = None;
    let mut kelvin: Option<f32> = None;
    let mut image: Option<i64> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--as-shot-kelvin" => kelvin = Some(args.next().and_then(|v| v.parse().ok()).ok_or("--as-shot-kelvin needs a number")?),
            "--image" => image = Some(args.next().and_then(|v| v.parse().ok()).ok_or("--image needs an id_local")?),
            _ if file.is_none() => file = Some(PathBuf::from(a)),
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    let file = file.ok_or("usage: lr2lumen <file.xmp|.lrtemplate|.lua|.lrcat> [--image ID] [--as-shot-kelvin N]")?;
    let is_catalog = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("lrcat"));
    let settings = if is_catalog {
        let id = image.ok_or("a catalog needs --image <id_local>")?;
        read_lua_table(&catalog_text(&file, id)?).map_err(|e| e.to_string())?
    } else {
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        read_settings(&file.to_string_lossy(), &text).map_err(|e| e.to_string())?
    };
    let converted = convert(&settings, kelvin);
    println!("{}", serde_json::to_string_pretty(&converted).map_err(|e| e.to_string())?);
    Ok(())
}

/// `Adobe_imageDevelopSettings.text` of one image, read from a temp copy of the catalog.
fn catalog_text(catalog: &Path, image: i64) -> Result<String, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let name = catalog.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    std::fs::copy(catalog, dir.path().join(&name)).map_err(|e| format!("{}: {e}", catalog.display()))?;
    for suffix in ["-wal", "-shm", "-journal"] {
        let side = PathBuf::from(format!("{}{suffix}", catalog.display()));
        if side.exists() {
            std::fs::copy(&side, dir.path().join(format!("{name}{suffix}"))).map_err(|e| e.to_string())?;
        }
    }
    let conn = rusqlite::Connection::open_with_flags(dir.path().join(&name), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    conn.query_row("SELECT text FROM Adobe_imageDevelopSettings WHERE image = ?1", [image], |r| r.get::<_, String>(0)).map_err(|e| format!("image {image}: {e}"))
}
