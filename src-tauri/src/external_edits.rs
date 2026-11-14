//! *Edit in* / *Open with* (Round 22 lane C, spec §8): what an app is given, and the copy made
//! for an editor beside its source in `originals/`. Lumen never hands an `Editor` the original
//! itself (non-negotiable 11).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use photos_core::assets::{self, NewAsset};
use photos_core::external_edits as db;
use photos_core::Library;

use crate::error::{AppError, AppResult};
use crate::external_apps::{made_key_hex, AppKind, InstalledApp, Launcher, Media, SendMode, Writes};

/// What an app is handed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sent {
    /// The original itself: an app that never writes it (`Video`, `Developer` + `Never`).
    Original,
    /// A TIFF rendered with the photo's Lumen adjustments.
    CopyRendered,
    /// A byte-for-byte copy of the original file (a RAW stays RAW).
    CopyOriginal,
}

impl Sent {
    /// Part of `made_key`: a rendered and an original copy of the same photo are different files.
    pub fn as_str(self) -> &'static str {
        match self {
            Sent::Original => "original",
            Sent::CopyRendered => "rendered",
            Sent::CopyOriginal => "copy",
        }
    }
}

/// The spec §8 table. `has_adjustments`: the export recipe is not the identity;
/// `app_reads_format`: the photo is not a RAW, or the app reads RAW.
pub fn what_to_send(kind: AppKind, writes: Writes, mode: SendMode, media: Media, has_adjustments: bool, app_reads_format: bool) -> Sent {
    match kind {
        AppKind::Video => Sent::Original,
        AppKind::Developer => match writes {
            // Its sidecar would land in `originals/`, so it works on a copy.
            Writes::Sidecar => Sent::CopyOriginal,
            Writes::Never | Writes::InPlace => Sent::Original,
        },
        AppKind::Editor => {
            // A video is never rendered to a TIFF.
            if media == Media::Video {
                return Sent::CopyOriginal;
            }
            match mode {
                SendMode::Rendered => Sent::CopyRendered,
                SendMode::OriginalFile => Sent::CopyOriginal,
                SendMode::Automatic if has_adjustments || !app_reads_format => Sent::CopyRendered,
                SendMode::Automatic => Sent::CopyOriginal,
            }
        }
    }
}

/// `IMG_1.NEF` rendered → `IMG_1-Edit.tif`; then `IMG_1-Edit-2.tif`, … A copy of the original
/// keeps the source's extension (`IMG_1-Edit.NEF`). `taken` says whether a name is in use (a file
/// on disk or a row in `external_edits`).
pub fn copy_name(source_file: &str, sent: Sent, taken: &dyn Fn(&str) -> bool) -> String {
    let p = Path::new(source_file);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| source_file.to_string());
    let ext = match sent {
        Sent::CopyRendered => "tif".to_string(),
        Sent::CopyOriginal | Sent::Original => p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    let named = |suffix: &str| if ext.is_empty() { format!("{stem}-Edit{suffix}") } else { format!("{stem}-Edit{suffix}.{ext}") };
    let first = named("");
    if !taken(&first) {
        return first;
    }
    (2u32..).map(|n| named(&format!("-{n}"))).find(|n| !taken(n)).expect("a free name")
}

/// See `photos_core::external_apps::made_key_hex`.
pub fn made_key(source_hash: &str, recipe_json: &str, sent: Sent, bits: u8) -> String {
    made_key_hex(source_hash, recipe_json, sent.as_str(), bits)
}

/// What one asset became for one app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub path: PathBuf,
    pub sent: Sent,
    /// A new file was written.
    pub made: bool,
    /// The asset a rendered copy was catalogued as (its derivatives still to be made).
    pub asset_id: Option<i64>,
}

/// A file's size and modification time in ms: what *coming back* (C4) compares.
pub fn file_stamp(path: &Path) -> Option<(i64, i64)> {
    let m = std::fs::metadata(path).ok()?;
    let ms = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.len() as i64, ms))
}

/// The bits per channel a rendered copy has: the export pipeline is 8-bit (`write_export`).
const RENDER_BITS: u8 = 8;

/// One `prepare` per (source, app) at a time, so two quick clicks make one copy.
type Locks = HashMap<(i64, String), Arc<Mutex<()>>>;
static PREPARING: LazyLock<Mutex<Locks>> = LazyLock::new(Default::default);

fn lock_for(source: i64, app: &str) -> Arc<Mutex<()>> {
    PREPARING.lock().unwrap_or_else(|p| p.into_inner()).entry((source, app.to_string())).or_default().clone()
}

fn read_source(lib: &Library, id: i64) -> AppResult<db::SourceRow> {
    lib.db.with_conn(|c| db::source_row(c, id))?.ok_or_else(|| AppError {
        code: "not_found".into(),
        message: "That photo is no longer in the library.".into(),
        detail: Some(id.to_string()),
    })
}

pub fn media_of(extension: &str) -> Media {
    let e = extension.to_ascii_lowercase();
    if photos_media::kind_for_extension(&e) == photos_media::MediaKind::Video {
        Media::Video
    } else if photos_core::formats::RAW_EXTENSIONS.contains(&e.as_str()) {
        Media::Raw
    } else {
        Media::Photo
    }
}

/// The folder part of a library path (`2024/05/01/x.jpg` → `2024/05/01`).
fn folder_of(relative_path: &str) -> &str {
    relative_path.rfind('/').map_or("", |i| &relative_path[..i])
}

fn join_rel(folder: &str, name: &str) -> String {
    if folder.is_empty() { name.to_string() } else { format!("{folder}/{name}") }
}

/// The file an app is given for `source_asset` (spec §8): the original itself, an untouched copy
/// made before, or a new copy beside the source in `originals/`. A rendered copy is catalogued at
/// once with the source's date; a copy of the original waits uncatalogued (`asset_id NULL`) until
/// the app changes its bytes. `render` writes an asset's TIFF to a path.
pub fn prepare(lib: &Library, app: &InstalledApp, mode: SendMode, source_asset: i64, render: &dyn Fn(i64, &Path) -> AppResult<()>) -> AppResult<Prepared> {
    let lock = lock_for(source_asset, &app.id);
    let _held = lock.lock().unwrap_or_else(|p| p.into_inner());
    let originals = lib.originals_dir();
    let src = read_source(lib, source_asset)?;
    let source_path = originals.join(&src.relative_path);
    // A copy itself opens as it is: the person goes on editing that file.
    if src.edited_in.is_some() {
        return Ok(Prepared { path: source_path, sent: Sent::Original, made: false, asset_id: None });
    }
    let media = media_of(&src.extension);
    let recipe = lib.db.with_conn(|c| photos_core::edits::export_recipe(c, source_asset))?.map(|(r, _)| r).filter(|r| !r.is_identity());
    let reads = media != Media::Raw || app.reads_raw;
    let sent = what_to_send(app.kind, app.writes, mode, media, recipe.is_some(), reads);
    if sent == Sent::Original {
        return Ok(Prepared { path: source_path, sent, made: false, asset_id: None });
    }
    let (recipe_json, bits) = match (sent, &recipe) {
        (Sent::CopyRendered, Some(r)) => (serde_json::to_string(r).unwrap_or_default(), RENDER_BITS),
        (Sent::CopyRendered, None) => (String::new(), RENDER_BITS),
        _ => (String::new(), 0),
    };
    let key = made_key(&src.content_hash, &recipe_json, sent, bits);

    // Used again: same key, the file as it was left, and nobody has saved over it since.
    for c in lib.db.with_conn(|c| db::reusable(c, source_asset, &app.id, &key))? {
        let path = originals.join(&c.relative_path);
        if file_stamp(&path) == Some((c.seen_size, c.seen_mtime)) {
            return Ok(Prepared { path, sent, made: false, asset_id: c.asset_id });
        }
    }

    // A new copy. The name is reserved on disk (`create_new`) so another app's copy of the same
    // photo, made at the same moment, takes the next one.
    let folder = folder_of(&src.relative_path).to_string();
    let (rel, path) = loop {
        let name = copy_name(&src.filename, sent, &|n| {
            let rel = join_rel(&folder, n);
            originals.join(&rel).exists() || lib.db.with_conn(|c| db::path_taken(c, &rel)).unwrap_or(true)
        });
        let rel = join_rel(&folder, &name);
        let path = originals.join(&rel);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => break (rel, path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    let part = crate::derivatives::part_path(&path);
    let written = match sent {
        Sent::CopyRendered => render(source_asset, &part),
        _ => std::fs::copy(&source_path, &part).map(|_| ()).map_err(AppError::from),
    }
    .and_then(|()| std::fs::rename(&part, &path).map_err(AppError::from));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&part);
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    let made_hash = match sent {
        Sent::CopyRendered => photos_import::content_hash(&path).map_err(|e| AppError { code: "hash_failed".into(), message: "The copy could not be read back.".into(), detail: Some(e.to_string()) })?,
        // The same bytes as the source.
        _ => src.content_hash.clone(),
    };
    let Some((size, mtime)) = file_stamp(&path) else {
        return Err(AppError { code: "copy_missing".into(), message: "The copy disappeared as it was made.".into(), detail: Some(path.display().to_string()) });
    };
    let now = photos_core::now_unix();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    // Identical rendered bytes already in the library (another app's copy of the same render)
    // can't be a second asset: this one waits, like a copy of the original.
    let catalogue = sent == Sent::CopyRendered && lib.db.with_conn(|c| assets::find_by_hash(c, &made_hash))?.is_none();
    let asset_id = if catalogue {
        let row = NewAsset {
            content_hash: made_hash.clone(),
            relative_path: rel.clone(),
            filename: name,
            extension: "tif".into(),
            media_type: 0,
            byte_size: size,
            orientation: 1,
            imported_at: now,
            file_modified_at: Some(mtime / 1000),
            ..src.inherit.clone()
        };
        let id = lib.db.with_conn(|c| assets::insert_assets(c, &[row]))?[0];
        lib.db.with_conn(|c| db::set_edited_in(c, id, &app.id))?;
        Some(id)
    } else {
        None
    };
    let row = db::NewCopy {
        source_asset_id: source_asset,
        asset_id,
        app_id: &app.id,
        relative_path: &rel,
        made_key: &key,
        made_hash: &made_hash,
        seen_size: size,
        seen_mtime: mtime,
        created_at: now,
    };
    lib.db.with_conn(|c| db::insert_copy(c, &row))?;
    Ok(Prepared { path, sent, made: true, asset_id })
}

/// An original handed to an app that should not write it (`Never`, `Video`), as it was then. C4
/// compares it when Lumen regains focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandedOver {
    pub asset_id: i64,
    pub app_id: String,
    pub size: i64,
    pub mtime: i64,
}

static HANDED_OVER: Mutex<Vec<HandedOver>> = Mutex::new(Vec::new());

/// The originals handed over this session.
pub fn handed_over() -> Vec<HandedOver> {
    HANDED_OVER.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

fn remember_handed_over(asset_id: i64, app_id: &str, path: &Path) {
    let Some((size, mtime)) = file_stamp(path) else { return };
    let mut all = HANDED_OVER.lock().unwrap_or_else(|p| p.into_inner());
    all.retain(|h| !(h.asset_id == asset_id && h.app_id == app_id));
    all.push(HandedOver { asset_id, app_id: app_id.to_string(), size, mtime });
}

/// What `open_with` did: the files launched, how many were new, and the rendered copies whose
/// derivatives the caller schedules.
#[derive(Debug, Default)]
pub struct Opened {
    pub paths: Vec<PathBuf>,
    pub made: u32,
    pub new_assets: Vec<i64>,
}

/// Prepares every asset for `app` and launches it once with all the files. A photo that fails is
/// left out (logged); nothing prepared is an error.
pub fn open_with(lib: &Library, app: &InstalledApp, mode: SendMode, ids: &[i64], render: &dyn Fn(i64, &Path) -> AppResult<()>, launcher: &dyn Launcher) -> AppResult<Opened> {
    let mut out = Opened::default();
    let mut last_err = None;
    for &id in ids {
        match prepare(lib, app, mode, id, render) {
            Ok(p) => {
                if p.sent == Sent::Original {
                    remember_handed_over(id, &app.id, &p.path);
                }
                out.made += u32::from(p.made);
                if p.made {
                    out.new_assets.extend(p.asset_id);
                }
                out.paths.push(p.path);
            }
            Err(e) => {
                tracing::warn!(asset = id, app = %app.id, "could not prepare for the app: {}", e.detail.clone().unwrap_or_else(|| e.message.clone()));
                last_err = Some(e);
            }
        }
    }
    if out.paths.is_empty() {
        return Err(last_err.unwrap_or_else(|| AppError { code: "nothing_to_open".into(), message: "No photos to open.".into(), detail: None }));
    }
    launcher.launch(app, &out.paths).map_err(|e| AppError { code: "launch_failed".into(), message: format!("{} did not start.", app.name), detail: Some(e) })?;
    Ok(out)
}

// --- coming back (spec §8) ---------------------------------------------------------------------

/// What a look at the copies found. Ids are the photos' (assets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// A catalogued copy was saved over: new bytes, derivatives to be made again.
    Updated(i64),
    /// A copy became a photo (a waiting copy was saved, or a *Save As* appeared beside it).
    Adopted(i64),
    /// A copy that was still waiting is gone from disk; its row went.
    Removed,
    /// An original handed to an app that should not write it has changed since.
    OriginalChanged { asset_id: i64, app_id: String },
}

/// A file whose time is younger than this (ms) is left for the next look: the app may still be
/// writing it.
const SETTLE_MS: i64 = 2_000;

/// What a *Save As* from a RAW produces.
const SAVED_AS: [&str; 7] = ["tif", "tiff", "psd", "psb", "jpg", "jpeg", "png"];

pub type Hasher<'a> = &'a dyn Fn(&Path) -> io::Result<String>;

/// The production hasher: the library's own content hash.
pub fn content_hasher(path: &Path) -> io::Result<String> {
    photos_import::content_hash(path).map_err(io::Error::other)
}

/// Compares every copy's file with what Lumen last saw (one `stat` each; nothing is read unless
/// size or time changed) and brings back what the person's editors did, then checks the originals
/// handed over this session. `now` is unix ms, like `file_stamp`'s time; `hasher` is the library's
/// content hash. Derived data only: no original is touched.
pub fn sync(lib: &Library, now: i64, hasher: Hasher<'_>) -> Vec<Change> {
    let mut out = sync_copies(lib, now, hasher);
    let mut handed = HANDED_OVER.lock().unwrap_or_else(|p| p.into_inner());
    out.extend(check_handed_over(lib, now, hasher, &mut handed));
    out
}

fn sync_copies(lib: &Library, now: i64, hasher: Hasher<'_>) -> Vec<Change> {
    let Ok(rows) = lib.db.with_conn(db::all_copies) else { return Vec::new() };
    let originals = lib.originals_dir();
    let mut out = Vec::new();
    // One folder listing per (folder, stem) however many rows share it.
    let mut listed: std::collections::HashSet<(String, String)> = Default::default();
    for row in &rows {
        let path = originals.join(&row.relative_path);
        let Some(stamp) = file_stamp(&path) else {
            // A copy nobody saved and nobody kept: forgotten. A photo's row stays with its photo.
            if row.asset_id.is_none() && lib.db.with_conn(|c| db::delete_copy(c, row.id)).is_ok() {
                out.push(Change::Removed);
            }
            continue;
        };
        if stamp != (row.seen_size, row.seen_mtime) {
            out.extend(copy_changed(lib, row, &path, stamp, now, hasher));
        }
        let folder = folder_of(&row.relative_path).to_string();
        let stem = Path::new(&row.relative_path).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if listed.insert((folder.clone(), stem.clone())) {
            out.extend(adopt_siblings(lib, row, &folder, &stem, now, hasher));
        }
    }
    out
}

/// The facts a saved file carries that the catalogue holds: shape and orientation (read again),
/// with the size and time just stat'ed.
fn file_facts(path: &Path, hash: &str, (size, mtime): (i64, i64)) -> db::FileChange {
    let meta = photos_media::read_metadata(path, photos_media::kind_for_path(path));
    db::FileChange {
        content_hash: hash.to_string(),
        byte_size: size,
        file_modified_at: mtime / 1000,
        // The decoder fills the true size in; a stale one would mislead its orientation check.
        width: meta.width.map(i64::from),
        height: meta.height.map(i64::from),
        orientation: i64::from(meta.orientation),
    }
}

/// Catalogues `path` as a photo beside `source`, with its date and place, `edited_in` the app.
fn catalogue_saved(lib: &Library, source: i64, app_id: &str, rel: &str, path: &Path, hash: &str, stamp: (i64, i64)) -> Option<i64> {
    let src = read_source(lib, source).ok()?;
    let kind = photos_media::kind_for_path(path);
    let facts = file_facts(path, hash, stamp);
    let row = NewAsset {
        content_hash: hash.to_string(),
        relative_path: rel.to_string(),
        filename: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        extension: photos_media::extension_of(path),
        media_type: photos_media::media_type_code(kind),
        byte_size: facts.byte_size,
        width: facts.width,
        height: facts.height,
        orientation: facts.orientation,
        imported_at: photos_core::now_unix(),
        file_modified_at: Some(facts.file_modified_at),
        ..src.inherit
    };
    let id = lib.db.with_conn(|c| {
        let id = assets::insert_assets(c, &[row])?[0];
        db::set_edited_in(c, id, app_id)?;
        Ok(id)
    });
    match id {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(path = %path.display(), "could not catalogue a saved copy: {e}");
            None
        }
    }
}

fn copy_changed(lib: &Library, row: &db::CopyFull, path: &Path, stamp: (i64, i64), now: i64, hasher: Hasher<'_>) -> Option<Change> {
    if now - stamp.1 < SETTLE_MS {
        return None;
    }
    let hash = match hasher(path) {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(path = %path.display(), "could not read a copy back: {e}");
            return None;
        }
    };
    let seen = |lib: &Library| lib.db.with_conn(|c| db::set_seen(c, row.id, stamp.0, stamp.1));
    let known = row.asset_hash.as_deref().unwrap_or(&row.made_hash);
    if hash == known {
        // Saved with the same bytes, or only touched.
        let _ = seen(lib);
        return None;
    }
    // Bytes that are another photo's can't be this one (the hash is unique): left alone, once.
    if lib.db.with_conn(|c| assets::find_by_hash(c, &hash)).ok().flatten().is_some_and(|other| Some(other) != row.asset_id) {
        let _ = seen(lib);
        return None;
    }
    match (row.asset_id, row.asset_hash.as_deref()) {
        (Some(asset), Some(old)) => {
            let facts = file_facts(path, &hash, stamp);
            let done = lib.db.with_conn(|c| {
                db::file_changed(c, asset, &facts)?;
                db::set_seen(c, row.id, stamp.0, stamp.1)
            });
            if let Err(e) = done {
                tracing::warn!(asset, "could not follow a saved copy: {e}");
                return None;
            }
            photos_media::DerivativeStore::new(&lib.root).delete_all(old);
            Some(Change::Updated(asset))
        }
        _ => {
            let new = catalogue_saved(lib, row.source_asset_id, &row.app_id, &row.relative_path, path, &hash, stamp)?;
            let _ = lib.db.with_conn(|c| {
                db::link_asset(c, row.id, new)?;
                db::set_seen(c, row.id, stamp.0, stamp.1)
            });
            Some(Change::Adopted(new))
        }
    }
}

/// Files saved beside a copy under the same stem (`IMG_1-Edit*.{tif,psd,…}`): further copies of
/// the same photo for the same app.
fn adopt_siblings(lib: &Library, row: &db::CopyFull, folder: &str, stem: &str, now: i64, hasher: Hasher<'_>) -> Vec<Change> {
    let dir = lib.originals_dir().join(folder);
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut names: Vec<String> = entries.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let p = Path::new(&name);
        let ext = photos_media::extension_of(p);
        let lower_stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !SAVED_AS.contains(&ext.as_str()) || !lower_stem.starts_with(stem) {
            continue;
        }
        let rel = join_rel(folder, &name);
        let known = lib.db.with_conn(|c| Ok(db::path_taken(c, &rel)? || db::asset_at_path(c, &rel)?.is_some())).unwrap_or(true);
        let path = dir.join(&name);
        let Some(stamp) = file_stamp(&path).filter(|_| !known) else { continue };
        if now - stamp.1 < SETTLE_MS {
            continue;
        }
        let Ok(hash) = hasher(&path) else { continue };
        let owner = lib.db.with_conn(|c| assets::find_by_hash(c, &hash)).ok().flatten();
        let asset = if owner.is_some() { None } else { catalogue_saved(lib, row.source_asset_id, &row.app_id, &rel, &path, &hash, stamp) };
        if owner.is_none() && asset.is_none() {
            continue;
        }
        // Another photo's bytes can't be a second photo: the row waits, so it is not read again.
        let copy = db::NewCopy {
            source_asset_id: row.source_asset_id,
            asset_id: asset,
            app_id: &row.app_id,
            relative_path: &rel,
            made_key: "saved-as",
            made_hash: &hash,
            seen_size: stamp.0,
            seen_mtime: stamp.1,
            created_at: photos_core::now_unix(),
        };
        if lib.db.with_conn(|c| db::insert_copy(c, &copy)).is_ok() {
            out.extend(asset.map(Change::Adopted));
        }
    }
    out
}

/// Compares the originals handed over with what they were. A real change is reported once (the
/// remembered size and time move on) and nothing in the catalogue is rewritten: the next *Check the
/// library* lists the file as changed.
fn check_handed_over(lib: &Library, now: i64, hasher: Hasher<'_>, list: &mut [HandedOver]) -> Vec<Change> {
    let originals = lib.originals_dir();
    let mut out = Vec::new();
    for h in list.iter_mut() {
        let Some(src) = lib.db.with_conn(|c| db::source_row(c, h.asset_id)).ok().flatten() else { continue };
        let path = originals.join(&src.relative_path);
        let Some((size, mtime)) = file_stamp(&path) else { continue };
        if (size, mtime) == (h.size, h.mtime) || now - mtime < SETTLE_MS {
            continue;
        }
        let Ok(hash) = hasher(&path) else { continue };
        (h.size, h.mtime) = (size, mtime);
        if hash != src.content_hash {
            out.push(Change::OriginalChanged { asset_id: h.asset_id, app_id: h.app_id.clone() });
        }
    }
    out
}

// --- when to look ------------------------------------------------------------------------------

/// An original handed to an app was changed behind Lumen's back (toast on the frontend).
pub const ORIGINAL_CHANGED: &str = "external://original-changed";

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OriginalChanged {
    /// The app's name.
    pub app: String,
    /// The photo's file name.
    pub name: String,
}

/// The window must have been back this long since the last look (no watcher: a look is a stat per
/// copy, but a window that flickers focus should not queue one each time).
const FOCUS_GAP: std::time::Duration = std::time::Duration::from_secs(5);

static LAST_LOOK: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static LOOKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The window regained focus: look at the copies, at most once per `FOCUS_GAP`.
pub fn on_focus(app: &tauri::AppHandle) {
    {
        let mut last = LAST_LOOK.lock().unwrap_or_else(|p| p.into_inner());
        if last.is_some_and(|t| t.elapsed() < FOCUS_GAP) {
            return;
        }
        *last = Some(std::time::Instant::now());
    }
    schedule_sync(app);
}

/// Queues one look on the Maintenance lane (start-up, focus). A look already queued or running
/// covers this one.
pub fn schedule_sync(app: &tauri::AppHandle) {
    use std::sync::atomic::Ordering;
    use tauri::Manager;
    if LOOKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let a = app.clone();
    app.state::<crate::state::AppState>().jobs.submit("external-edits:sync", photos_jobs::Lane::Maintenance, move |_| {
        run_sync(&a);
        LOOKING.store(false, Ordering::SeqCst);
    });
}

fn run_sync(app: &tauri::AppHandle) {
    use tauri::{Emitter, Manager};
    let state = app.state::<crate::state::AppState>();
    let Ok(lib) = state.library() else { return };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let changes = sync(&lib, now, &content_hasher);
    let mut shown = false;
    for change in changes {
        match change {
            Change::Updated(id) | Change::Adopted(id) => {
                // Derivatives made again; the grid shows the new picture.
                if let Ok(info) = lib.db.with_conn(|c| assets::render_info(c, id)) {
                    let kind = photos_media::kind_for_extension(&info.extension);
                    crate::derivatives::schedule_ingest(app.clone(), crate::derivatives::ingested_from(&lib, id, &info, kind));
                    shown = true;
                }
            }
            Change::Removed => {}
            Change::OriginalChanged { asset_id, app_id } => {
                let custom = state.settings.read().expect("settings lock").external_apps.custom.clone();
                let app_name = crate::external_apps::detected(&custom).into_iter().find(|a| a.id == app_id).map_or(app_id, |a| a.name);
                let name = lib.db.with_conn(|c| db::source_row(c, asset_id)).ok().flatten().map(|s| s.filename).unwrap_or_default();
                let _ = app.emit(ORIGINAL_CHANGED, OriginalChanged { app: app_name, name });
            }
        }
    }
    if shown {
        state.changes.changed(app, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_to_send_follows_the_table() {
        use AppKind::*;
        use SendMode::*;
        let p = Media::Photo;
        let rows = [
            (what_to_send(Video, Writes::Never, Automatic, Media::Video, false, true), Sent::Original),
            (what_to_send(Developer, Writes::Never, Automatic, p, true, true), Sent::Original),
            (what_to_send(Developer, Writes::Sidecar, Automatic, Media::Raw, true, true), Sent::CopyOriginal),
            (what_to_send(Editor, Writes::InPlace, Rendered, p, false, true), Sent::CopyRendered),
            (what_to_send(Editor, Writes::InPlace, OriginalFile, p, true, true), Sent::CopyOriginal),
            (what_to_send(Editor, Writes::InPlace, Automatic, p, true, true), Sent::CopyRendered),
            (what_to_send(Editor, Writes::InPlace, Automatic, p, false, true), Sent::CopyOriginal),
            // A RAW to GIMP, no adjustments.
            (what_to_send(Editor, Writes::InPlace, Automatic, Media::Raw, false, false), Sent::CopyRendered),
            (what_to_send(Editor, Writes::InPlace, Automatic, Media::Video, true, true), Sent::CopyOriginal),
        ];
        for (i, (got, want)) in rows.iter().enumerate() {
            assert_eq!(got, want, "row {i}");
        }
    }

    #[test]
    fn an_editor_never_gets_the_original() {
        for writes in [Writes::InPlace, Writes::Sidecar, Writes::Never] {
            for mode in [SendMode::Automatic, SendMode::Rendered, SendMode::OriginalFile] {
                for media in [Media::Photo, Media::Raw, Media::Video] {
                    for adj in [false, true] {
                        for reads in [false, true] {
                            assert_ne!(what_to_send(AppKind::Editor, writes, mode, media, adj, reads), Sent::Original);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn copy_name_takes_the_next_free() {
        let none = |_: &str| false;
        assert_eq!(copy_name("IMG_1.NEF", Sent::CopyRendered, &none), "IMG_1-Edit.tif");
        let first_taken = |n: &str| n == "IMG_1-Edit.tif";
        assert_eq!(copy_name("IMG_1.NEF", Sent::CopyRendered, &first_taken), "IMG_1-Edit-2.tif");
        let two_taken = |n: &str| n == "IMG_1-Edit.tif" || n == "IMG_1-Edit-2.tif";
        assert_eq!(copy_name("IMG_1.NEF", Sent::CopyRendered, &two_taken), "IMG_1-Edit-3.tif");
        assert_eq!(copy_name("IMG_1.NEF", Sent::CopyOriginal, &none), "IMG_1-Edit.NEF");
        assert_eq!(copy_name("Čaj ü 1.jpg", Sent::CopyOriginal, &none), "Čaj ü 1-Edit.jpg");
    }

    // --- prepare, on temp libraries -------------------------------------------------------

    use photos_core::edits::{save_recipe, Recipe};

    fn app(id: &str, kind: AppKind, writes: Writes) -> InstalledApp {
        InstalledApp {
            id: id.into(),
            name: id.into(),
            kind,
            accepts: vec![Media::Photo, Media::Raw, Media::Video],
            reads_raw: true,
            writes,
            target: PathBuf::from("C:/Apps/fake.exe"),
            exec: None,
            custom: false,
            added_by: None,
        }
    }

    fn photoshop() -> InstalledApp {
        app("photoshop", AppKind::Editor, Writes::InPlace)
    }

    /// A library with one photo at `originals/<rel>` holding `bytes`.
    fn library_with(rel: &str, filename: &str, bytes: &[u8]) -> (tempfile::TempDir, Library, i64) {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::create(&dir.path().join("lib")).unwrap();
        let path = lib.originals_dir().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        let ext = Path::new(filename).extension().unwrap().to_string_lossy().into_owned();
        let row = NewAsset {
            content_hash: photos_import::content_hash(&path).unwrap(),
            relative_path: rel.into(),
            filename: filename.into(),
            extension: ext,
            byte_size: bytes.len() as i64,
            orientation: 1,
            captured_at: Some(1_700_000_000),
            gps_lat: Some(46.0),
            ..Default::default()
        };
        let id = lib.db.with_conn(|c| assets::insert_assets(c, &[row])).unwrap()[0];
        (dir, lib, id)
    }

    fn edit(lib: &Library, id: i64, rotate: u16) {
        let r = Recipe { rotate, ..Recipe::identity() };
        lib.db.with_conn(|c| save_recipe(c, id, &r, 1)).unwrap();
    }

    /// Writes different bytes per call, so each render is a new file.
    fn stub_render() -> impl Fn(i64, &Path) -> AppResult<()> {
        let n = std::sync::atomic::AtomicU32::new(0);
        move |id, out| {
            let k = n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::fs::write(out, format!("TIFF of {id} #{k}"))?;
            Ok(())
        }
    }

    fn count(lib: &Library, sql: &str) -> i64 {
        lib.db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }

    #[test]
    fn a_rendered_copy_becomes_a_photo_beside_its_source() {
        let (_d, lib, id) = library_with("2024/05/01/aaaa1111_IMG_1.jpg", "IMG_1.jpg", b"jpeg bytes");
        edit(&lib, id, 90);
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        assert_eq!(p.sent, Sent::CopyRendered);
        assert!(p.made);
        assert_eq!(p.path, lib.originals_dir().join("2024/05/01/IMG_1-Edit.tif"));
        assert!(p.path.is_file());
        let new = p.asset_id.expect("catalogued");
        let (captured, edited_in, ext, lat): (i64, String, String, f64) = lib
            .db
            .with_conn(|c| Ok(c.query_row("SELECT captured_at, edited_in, extension, gps_lat FROM assets WHERE id = ?1", [new], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?))
            .unwrap();
        assert_eq!((captured, edited_in.as_str(), ext.as_str(), lat), (1_700_000_000, "photoshop", "tif", 46.0));
        let linked = count(&lib, &format!("SELECT COUNT(*) FROM external_edits WHERE source_asset_id = {id} AND asset_id = {new} AND app_id = 'photoshop'"));
        assert_eq!(linked, 1);
    }

    #[test]
    fn an_original_copy_waits_uncatalogued() {
        let (_d, lib, id) = library_with("2024/05/01/bbbb2222_IMG_2.jpg", "IMG_2.jpg", b"plain jpeg");
        let source_hash: String = lib.db.with_conn(|c| Ok(c.query_row("SELECT content_hash FROM assets WHERE id = ?1", [id], |r| r.get(0))?)).unwrap();
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        assert_eq!(p.sent, Sent::CopyOriginal);
        assert_eq!(p.path, lib.originals_dir().join("2024/05/01/IMG_2-Edit.jpg"));
        assert_eq!(std::fs::read(&p.path).unwrap(), b"plain jpeg");
        assert_eq!(p.asset_id, None);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 1);
        let (asset, made_hash): (Option<i64>, String) =
            lib.db.with_conn(|c| Ok(c.query_row("SELECT asset_id, made_hash FROM external_edits", [], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap();
        assert_eq!((asset, made_hash), (None, source_hash));
    }

    #[test]
    fn the_same_copy_is_used_again() {
        let (_d, lib, id) = library_with("2024/05/01/cccc3333_IMG_3.jpg", "IMG_3.jpg", b"jpeg");
        edit(&lib, id, 90);
        let render = stub_render();
        let a = prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap();
        let again = prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap();
        assert_eq!((again.path.clone(), again.made), (a.path.clone(), false));
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 1);
        // Another recipe is another picture.
        edit(&lib, id, 180);
        let b = prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap();
        assert!(b.made);
        assert_ne!(b.path, a.path);
        // The app saved over b: a new copy, b left as the app wrote it.
        std::fs::write(&b.path, b"saved by the editor, longer than before").unwrap();
        let c = prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap();
        assert!(c.made);
        assert_ne!(c.path, b.path);
        assert_eq!(std::fs::read(&b.path).unwrap(), b"saved by the editor, longer than before");
    }

    #[test]
    fn two_requests_at_once_make_one_copy() {
        let (_d, lib, id) = library_with("2024/05/01/dddd4444_IMG_4.jpg", "IMG_4.jpg", b"jpeg");
        edit(&lib, id, 90);
        let render = |id: i64, out: &Path| -> AppResult<()> {
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::fs::write(out, format!("TIFF of {id}"))?;
            Ok(())
        };
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap());
            let b = s.spawn(|| prepare(&lib, &photoshop(), SendMode::Automatic, id, &render).unwrap());
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_eq!(a.path, b.path);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 1);
        let tifs = std::fs::read_dir(a.path.parent().unwrap()).unwrap().filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "tif")).count();
        assert_eq!(tifs, 1);
    }

    #[test]
    fn an_editor_never_gets_the_source_path() {
        let (_d, lib, id) = library_with("2024/05/01/eeee5555_IMG_5.NEF", "IMG_5.NEF", b"raw bytes");
        let source = lib.originals_dir().join("2024/05/01/eeee5555_IMG_5.NEF");
        for mode in [SendMode::Automatic, SendMode::Rendered, SendMode::OriginalFile] {
            for reads_raw in [false, true] {
                let ed = InstalledApp { reads_raw, ..photoshop() };
                let p = prepare(&lib, &ed, mode, id, &stub_render()).unwrap();
                assert_ne!(p.path, source, "{mode:?} reads_raw={reads_raw}");
                assert_ne!(p.sent, Sent::Original);
            }
        }
        // A video app, by contrast, is handed the original.
        let p = prepare(&lib, &app("resolve", AppKind::Video, Writes::Never), SendMode::Automatic, id, &stub_render()).unwrap();
        assert_eq!((p.path, p.sent), (source, Sent::Original));
    }

    struct FakeLauncher(Mutex<Vec<Vec<PathBuf>>>);

    impl Launcher for FakeLauncher {
        fn launch(&self, _: &InstalledApp, files: &[PathBuf]) -> Result<(), String> {
            self.0.lock().unwrap().push(files.to_vec());
            Ok(())
        }
    }

    #[test]
    fn paths_reach_the_launcher_whole() {
        let (_d, lib, id) = library_with("2024/05/01 x/ffff6666_Čaj ü 1.jpg", "Čaj ü 1.jpg", b"tea");
        let launcher = FakeLauncher(Mutex::new(Vec::new()));
        let opened = open_with(&lib, &photoshop(), SendMode::Automatic, &[id], &stub_render(), &launcher).unwrap();
        let calls = launcher.0.into_inner().unwrap();
        let copy = lib.originals_dir().join("2024/05/01 x/Čaj ü 1-Edit.jpg");
        assert_eq!(calls, vec![vec![copy.clone()]]);
        assert!(copy.is_file());
        assert_eq!((opened.made, opened.paths), (1, vec![copy]));
    }

    #[test]
    fn edit_in_on_a_copy_opens_that_file() {
        let (_d, lib, id) = library_with("2024/05/01/gggg7777_IMG_7.jpg", "IMG_7.jpg", b"jpeg");
        edit(&lib, id, 90);
        let first = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        let copy = first.asset_id.unwrap();
        let p = prepare(&lib, &photoshop(), SendMode::Rendered, copy, &stub_render()).unwrap();
        assert_eq!((p.path, p.sent, p.made), (first.path, Sent::Original, false));
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 1);
    }

    // --- coming back (C4) -------------------------------------------------------------------

    /// Long after every file's time, so nothing counts as still being written.
    const LATER: i64 = 4_000_000_000_000;

    fn hashing() -> (impl Fn(&Path) -> io::Result<String>, Arc<std::sync::atomic::AtomicU32>) {
        let calls = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let c = calls.clone();
        (move |p: &Path| {
            c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            photos_import::content_hash(p).map_err(io::Error::other)
        }, calls)
    }

    fn calls(n: &Arc<std::sync::atomic::AtomicU32>) -> u32 {
        n.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn text(lib: &Library, sql: &str) -> String {
        lib.db.with_conn(|c| Ok(c.query_row(sql, [], |r| r.get::<_, String>(0))?)).unwrap()
    }

    /// A rendered copy of a fresh photo, catalogued: (library, source id, copy id, copy path).
    fn rendered_copy() -> (tempfile::TempDir, Library, i64, i64, PathBuf) {
        let (d, lib, id) = library_with("2024/05/01/aaaa1111_IMG_1.jpg", "IMG_1.jpg", b"jpeg bytes");
        edit(&lib, id, 90);
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        (d, lib, id, p.asset_id.unwrap(), p.path)
    }

    #[test]
    fn an_unchanged_copy_costs_one_stat() {
        let (_d, lib, ..) = rendered_copy();
        let (hasher, n) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(calls(&n), 0, "nothing changed: nothing hashed");
    }

    #[test]
    fn a_saved_copy_is_rehashed_and_shown_again() {
        let (_d, lib, _src, copy, path) = rendered_copy();
        let old: String = text(&lib, &format!("SELECT content_hash FROM assets WHERE id = {copy}"));
        let store = photos_media::DerivativeStore::new(&lib.root);
        let thumb = store.thumb_path(&old, 256);
        std::fs::create_dir_all(thumb.parent().unwrap()).unwrap();
        std::fs::write(&thumb, b"old thumbnail").unwrap();

        std::fs::write(&path, b"saved by the editor, longer than the render").unwrap();
        let (hasher, n) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![Change::Updated(copy)]);
        let new = photos_import::content_hash(&path).unwrap();
        assert_ne!(new, old);
        assert_eq!(text(&lib, &format!("SELECT content_hash FROM assets WHERE id = {copy}")), new);
        assert_eq!(count(&lib, &format!("SELECT byte_size FROM assets WHERE id = {copy}")), 43);
        let (size, mtime) = file_stamp(&path).unwrap();
        assert_eq!(count(&lib, &format!("SELECT seen_size FROM external_edits WHERE asset_id = {copy}")), size);
        assert_eq!(count(&lib, &format!("SELECT seen_mtime FROM external_edits WHERE asset_id = {copy}")), mtime);
        assert!(!thumb.exists(), "the old bytes' derivatives are gone");
        // Looked at once: the second look is a stat.
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(calls(&n), 1);
    }

    #[test]
    fn a_touched_copy_with_the_same_bytes_is_not_an_update() {
        let (_d, lib, _src, copy, path) = rendered_copy();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let (hasher, _) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(count(&lib, &format!("SELECT seen_size FROM external_edits WHERE asset_id = {copy}")), bytes.len() as i64);
    }

    #[test]
    fn a_waiting_copy_becomes_a_photo_when_saved() {
        let (_d, lib, id) = library_with("2024/05/01/bbbb2222_IMG_2.jpg", "IMG_2.jpg", b"plain jpeg");
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        assert_eq!(p.asset_id, None);
        let (hasher, n) = hashing();
        // The editor has not saved: the same bytes, nothing to do.
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 1);
        std::fs::write(&p.path, b"retouched jpeg, different bytes").unwrap();
        let changes = sync_copies(&lib, LATER, &hasher);
        let [Change::Adopted(new)] = changes[..] else { panic!("{changes:?}") };
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 2);
        assert_eq!(text(&lib, &format!("SELECT edited_in FROM assets WHERE id = {new}")), "photoshop");
        assert_eq!(count(&lib, &format!("SELECT captured_at FROM assets WHERE id = {new}")), 1_700_000_000);
        assert_eq!(text(&lib, &format!("SELECT relative_path FROM assets WHERE id = {new}")), "2024/05/01/IMG_2-Edit.jpg");
        assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM external_edits WHERE asset_id = {new} AND source_asset_id = {id}")), 1);
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(calls(&n), 1, "only the changed file was hashed");
    }

    #[test]
    fn a_save_as_beside_it_is_brought_in() {
        let (_d, lib, id) = library_with("2024/05/01/cccc3333_IMG_1.NEF", "IMG_1.NEF", b"raw bytes");
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        assert_eq!((p.sent, p.path.file_name().unwrap().to_str().unwrap()), (Sent::CopyOriginal, "IMG_1-Edit.NEF"));
        let folder = p.path.parent().unwrap();
        for (name, bytes) in [("IMG_1-Edit.psd", "psd"), ("IMG_1-Edit copy.tif", "tif"), ("IMG_10-Edit.tif", "other stem"), ("IMG_1-Edit.xmp", "xmp")] {
            std::fs::write(folder.join(name), bytes).unwrap();
        }
        let (hasher, _) = hashing();
        let changes = sync_copies(&lib, LATER, &hasher);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes.iter().all(|c| matches!(c, Change::Adopted(_))));
        let paths = text(&lib, "SELECT group_concat(relative_path, '|') FROM (SELECT relative_path FROM assets WHERE edited_in IS NOT NULL ORDER BY relative_path)");
        assert_eq!(paths, "2024/05/01/IMG_1-Edit copy.tif|2024/05/01/IMG_1-Edit.psd");
        assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM external_edits WHERE source_asset_id = {id} AND app_id = 'photoshop' AND asset_id IS NOT NULL")), 2);
        assert_eq!(count(&lib, &format!("SELECT COUNT(*) FROM external_edits WHERE source_asset_id = {id} AND asset_id IS NULL")), 1, "the NEF copy still waits");
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![], "nothing is brought in twice");
    }

    #[test]
    fn a_save_as_with_a_known_pictures_bytes_waits_instead() {
        let (_d, lib, id) = library_with("2024/05/01/dddd4444_IMG_1.jpg", "IMG_1.jpg", b"plain jpeg");
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        // Saved as a byte-for-byte copy of a photo the library already holds.
        std::fs::write(p.path.parent().unwrap().join("IMG_1-Edit copy.jpg"), b"plain jpeg").unwrap();
        let (hasher, n) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 1);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 2);
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(calls(&n), 1, "remembered, not hashed again");
    }

    #[test]
    fn a_file_still_being_written_waits() {
        let (_d, lib, _src, copy, path) = rendered_copy();
        std::fs::write(&path, b"half of what the editor is writing").unwrap();
        let (_, mtime) = file_stamp(&path).unwrap();
        let (hasher, n) = hashing();
        assert_eq!(sync_copies(&lib, mtime + 1_000, &hasher), vec![]);
        assert_eq!(calls(&n), 0, "too fresh to read");
        assert_eq!(sync_copies(&lib, mtime + 2_000, &hasher), vec![Change::Updated(copy)]);
    }

    #[test]
    fn a_deleted_waiting_copy_drops_its_row() {
        let (_d, lib, id) = library_with("2024/05/01/eeee5555_IMG_5.jpg", "IMG_5.jpg", b"plain jpeg");
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        std::fs::remove_file(&p.path).unwrap();
        let (hasher, _) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![Change::Removed]);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 0);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 1);
    }

    #[test]
    fn a_deleted_catalogued_copy_keeps_its_row_and_photo() {
        let (_d, lib, _src, _copy, path) = rendered_copy();
        std::fs::remove_file(&path).unwrap();
        let (hasher, _) = hashing();
        assert_eq!(sync_copies(&lib, LATER, &hasher), vec![]);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM external_edits"), 1);
        assert_eq!(count(&lib, "SELECT COUNT(*) FROM assets"), 2);
    }

    /// A 4 × 4 flattened RGB PSD, uncompressed (what `Save As .psd` leaves when nothing is layered).
    fn tiny_psd() -> Vec<u8> {
        let mut b = b"8BPS".to_vec();
        b.extend([0, 1, 0, 0, 0, 0, 0, 0, 0, 3]); // version 1, 6 reserved, 3 channels
        b.extend([0, 0, 0, 4, 0, 0, 0, 4, 0, 8, 0, 3]); // 4 high, 4 wide, 8 bit, RGB
        b.extend([0u8; 12]); // no colour data, no resources, no layers
        b.extend([0, 0]); // raw image data
        b.extend((0..48).map(|i| (i * 5) as u8));
        b
    }

    /// Verified for C4 (plan *Verify* PSD): the photo pipeline does **not** read a PSD yet. `psd` is
    /// not in `photos-media`'s extension lists, so a PSD copy is brought in but its ingest fails and
    /// it lands in *Files Lumen can't open* (FFmpeg has a `psd` decoder; routing it is a follow-up).
    /// If this starts failing, PSD support arrived: change the test to the decode it now does.
    #[test]
    fn a_psd_copy_is_brought_in_but_the_pipeline_cannot_read_it_yet() {
        let (_d, lib, id) = library_with("2024/05/01/hhhh8888_IMG_8.NEF", "IMG_8.NEF", b"raw bytes");
        let p = prepare(&lib, &photoshop(), SendMode::Automatic, id, &stub_render()).unwrap();
        let psd = p.path.parent().unwrap().join("IMG_8-Edit.psd");
        std::fs::write(&psd, tiny_psd()).unwrap();
        let (hasher, _) = hashing();
        let changes = sync_copies(&lib, LATER, &hasher);
        let [Change::Adopted(new)] = changes[..] else { panic!("{changes:?}") };
        assert_eq!(text(&lib, &format!("SELECT extension FROM assets WHERE id = {new}")), "psd");
        let kind = photos_media::kind_for_path(&psd);
        assert_eq!(kind, photos_media::MediaKind::Unknown);
        assert!(photos_media::decode_preview(&psd, kind).is_err());
    }

    #[test]
    fn a_changed_original_is_reported_once() {
        let (_d, lib, id) = library_with("2024/05/01/ffff6666_IMG_6.mov", "IMG_6.mov", b"movie bytes");
        let path = lib.originals_dir().join("2024/05/01/ffff6666_IMG_6.mov");
        let (size, mtime) = file_stamp(&path).unwrap();
        let mut handed = vec![HandedOver { asset_id: id, app_id: "resolve".into(), size, mtime }];
        let (hasher, n) = hashing();
        assert_eq!(check_handed_over(&lib, LATER, &hasher, &mut handed), vec![]);
        assert_eq!(calls(&n), 0);
        let before: String = text(&lib, &format!("SELECT content_hash FROM assets WHERE id = {id}"));
        std::fs::write(&path, b"movie bytes, trimmed by the app").unwrap();
        let want = Change::OriginalChanged { asset_id: id, app_id: "resolve".into() };
        assert_eq!(check_handed_over(&lib, LATER, &hasher, &mut handed), vec![want]);
        assert_eq!(check_handed_over(&lib, LATER, &hasher, &mut handed), vec![], "once");
        assert_eq!(text(&lib, &format!("SELECT content_hash FROM assets WHERE id = {id}")), before, "the catalogue is not rewritten");
        // Touched with the same bytes is no change.
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::write(&path, b"movie bytes").unwrap();
        assert_eq!(check_handed_over(&lib, LATER, &hasher, &mut handed), vec![], "back to the catalogued bytes");
    }
}
