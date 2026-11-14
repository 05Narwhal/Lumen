//! What Develop starts from when rawler refuses a RAW file: Nikon's High Efficiency NEFs,
//! bodies it doesn't know, formats it has no decoder for (`docs/RAW-FORMATS.md`). Two stand-ins:
//!
//! 1. **Adobe DNG Converter**, when the person has it (free, but not Lumen's to ship): the file
//!    converted to a lossless DNG, which rawler develops as sensor data. The DNG is cached under
//!    app data as `<cache>/<content hash>.dng`, never beside the original or inside a library;
//!    the original is only read.
//! 2. Otherwise, or when the converter fails, the camera's own JPEG inside the file: an 8-bit
//!    base Develop labels *Camera rendering, not RAW*.
//!
//! A missing converter is a state, never an error. The binary calls [`configure`] at start;
//! unconfigured (command-line tools) the refusal stands, as before.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use super::Linear;
use crate::MediaError;

/// How long a conversion may take before it is ended and the camera rendering used. A 45 MP
/// file takes a few seconds; this is for a converter that hangs.
pub const CONVERT_TIMEOUT: Duration = Duration::from_secs(180);

/// The DNG cache's size. Derived data, but a heavy shooter's could otherwise fill a disk: the
/// least recently opened DNGs go first (spec, Round 17).
pub const CACHE_CAP: u64 = 20 * 1024 * 1024 * 1024;

/// Where Adobe DNG Converter installs, as `lumen_train`'s prep looks for it.
pub const STANDARD_PATHS: &[&str] = &[
    "C:/Program Files/Adobe/Adobe DNG Converter/Adobe DNG Converter.exe",
    "C:/Program Files (x86)/Adobe/Adobe DNG Converter/Adobe DNG Converter.exe",
    "/Applications/Adobe DNG Converter.app/Contents/MacOS/Adobe DNG Converter",
];

/// The environment variable naming the converter. Set to a path that does not exist, it hides
/// an installed one.
pub const CONVERTER_ENV: &str = "LUMEN_DNG_CONVERTER";

static CACHE_DIR: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Develop refused RAWs through a DNG cache in `cache_dir` (the app's
/// `config_dir/JL-Studios/Lumen/cache/dng`).
pub fn configure(cache_dir: impl Into<PathBuf>) {
    *CACHE_DIR.write().unwrap_or_else(|p| p.into_inner()) = Some(cache_dir.into());
}

fn cache_dir() -> Option<PathBuf> {
    CACHE_DIR.read().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Adobe DNG Converter: `LUMEN_DNG_CONVERTER` when set, else a standard install. Looked up per
/// call, so installing it counts without a restart.
pub fn converter() -> Option<PathBuf> {
    locate(std::env::var_os(CONVERTER_ENV), STANDARD_PATHS.iter().map(Path::new))
}

/// [`converter`]'s rule: the variable wins, even when it names nothing.
pub fn locate<'a>(env: Option<OsString>, standard: impl IntoIterator<Item = &'a Path>) -> Option<PathBuf> {
    if let Some(p) = env.filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    standard.into_iter().find(|p| p.is_file()).map(Path::to_path_buf)
}

/// What a refused RAW develops from.
#[derive(Debug, PartialEq)]
pub enum Base {
    /// The converted DNG's sensor data, with the base look, in the sensor's orientation.
    Sensor(Linear),
    /// The file's embedded camera JPEG (the caller decodes it as it does any RAW preview).
    Camera,
}

/// Whether the app set up a cache for stand-ins.
pub fn configured() -> bool {
    cache_dir().is_some()
}

/// The configured stand-in for the refused RAW at `original`; `None` when unconfigured.
pub fn develop_base(original: &Path) -> Option<Result<Base, MediaError>> {
    let dir = cache_dir()?;
    Some(base_for(original, &dir, converter().as_deref(), &run_converter, &super::raw::decode))
}

/// What the cache holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheUse {
    pub bytes: u64,
    pub files: u64,
}

/// The finished DNGs in `dir`, with their sizes and when each was last opened. A conversion in
/// progress lives in a `.part-` folder and is not one of them.
fn finished(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            let meta = e.metadata().ok()?;
            let is_dng = meta.is_file() && path.extension().is_some_and(|x| x.eq_ignore_ascii_case("dng"));
            is_dng.then(|| (path, meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)))
        })
        .collect()
}

pub fn cache_use(dir: &Path) -> CacheUse {
    finished(dir).iter().fold(CacheUse::default(), |u, (_, bytes, _)| CacheUse { bytes: u.bytes + bytes, files: u.files + 1 })
}

/// The configured cache's size, for Settings; nothing when unconfigured.
pub fn configured_cache_use() -> CacheUse {
    cache_dir().map(|d| cache_use(&d)).unwrap_or_default()
}

/// Remove the least recently opened DNGs until `dir` holds at most `cap` bytes. `keep`, the one
/// being opened, always stays. Derived data under app data, never a photo.
pub fn trim_cache(dir: &Path, cap: u64, keep: &Path) {
    let mut dngs = finished(dir);
    let mut total: u64 = dngs.iter().map(|d| d.1).sum();
    dngs.sort_by_key(|d| d.2);
    for (path, bytes, _) in dngs {
        if total <= cap {
            break;
        }
        if path == keep {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => total -= bytes,
            // In use by a decode right now: the next trim gets it.
            Err(e) => tracing::warn!("stand-in: could not trim {}: {e}", path.display()),
        }
    }
}

/// Empty the cache and forget which photos failed, so the next open of each tries again.
pub fn clear_cache(dir: &Path) {
    for (path, _, _) in finished(dir) {
        if let Err(e) = std::fs::remove_file(&path) {
            tracing::warn!("stand-in: could not clear {}: {e}", path.display());
        }
    }
    clear_crashed_parts(dir);
    if let Some(f) = failed().as_mut() {
        f.clear();
    }
}

/// [`clear_cache`] on the configured cache.
pub fn clear_configured_cache() {
    if let Some(d) = cache_dir() {
        clear_cache(&d);
    }
}

/// Remove the private folders a crash left behind: every `.part-` folder but this run's.
pub fn clear_crashed_parts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mine = format!("-{}-", std::process::id());
    for e in entries.filter_map(Result::ok) {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(".part-") && !name.contains(&mine) && e.path().is_dir() {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// The folder a library keeps `originals/` in, for a file inside one; else the file's folder.
fn library_root_of(original: &Path) -> Option<PathBuf> {
    original
        .ancestors()
        .skip(1)
        .find(|a| a.file_name().is_some_and(|n| n.eq_ignore_ascii_case("originals")))
        .and_then(Path::parent)
        .or_else(|| original.parent())
        .map(Path::to_path_buf)
}

/// Whether DNGs may be cached in `cache_dir` for `original`: never inside its library (the
/// folder holding `originals/`), never beside it. Both are resolved first, so case, separators
/// and `..` cannot hide the overlap. Nothing is created to find out.
pub fn cache_allowed(cache_dir: &Path, original: &Path) -> bool {
    let Some(root) = library_root_of(original) else { return false };
    match (resolved(cache_dir), resolved(&root)) {
        (Some(cache), Some(root)) => !cache.starts_with(&root),
        _ => false,
    }
}

/// `p` made absolute without `.` or `..`, its longest existing part resolved by the file
/// system (links, case), the rest appended; on Windows without the verbatim prefix and in
/// lower case, since the file system ignores case there.
fn resolved(p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let abs = std::path::absolute(p).ok()?;
    let mut lexical = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            c => lexical.push(c),
        }
    }
    let mut rest = Vec::new();
    let mut existing = lexical.as_path();
    let real = loop {
        if let Ok(r) = std::fs::canonicalize(existing) {
            break r;
        }
        rest.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
    };
    let full = rest.into_iter().rev().fold(real, |acc, part| acc.join(part));
    let s = full.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    Some(PathBuf::from(if cfg!(windows) { s.to_lowercase() } else { s.to_string() }))
}

/// The cached DNG for a content hash.
pub fn cached_dng(cache_dir: &Path, hash: &str) -> PathBuf {
    cache_dir.join(format!("{hash}.dng"))
}

/// One lock per photo, so two opens of the same photo never convert it twice at once.
fn lock_for(hash: &str) -> Arc<Mutex<()>> {
    static LOCKS: Mutex<Option<HashMap<String, Arc<Mutex<()>>>>> = Mutex::new(None);
    let mut locks = LOCKS.lock().unwrap_or_else(|p| p.into_inner());
    locks.get_or_insert_with(HashMap::new).entry(hash.to_string()).or_default().clone()
}

/// Photos whose conversion failed this session: they go straight to the camera rendering
/// instead of waiting for the converter again on every new preview size.
fn failed() -> std::sync::MutexGuard<'static, Option<HashSet<String>>> {
    static FAILED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    FAILED.lock().unwrap_or_else(|p| p.into_inner())
}

fn has_failed(hash: &str) -> bool {
    failed().as_ref().is_some_and(|f| f.contains(hash))
}

fn mark_failed(hash: &str) {
    failed().get_or_insert_with(HashSet::new).insert(hash.to_string());
}

fn content_hash(path: &Path) -> std::io::Result<String> {
    let mut h = blake3::Hasher::new();
    h.update_reader(std::fs::File::open(path)?)?;
    Ok(h.finalize().to_hex().to_string())
}

/// A converter: `(tool, original, out_dir)`, writing one `.dng` into `out_dir`.
pub type Convert<'a> = &'a (dyn Fn(&Path, &Path, &Path) -> Result<(), String> + Sync);
/// A DNG decoder: the sensor data with the base look.
pub type Decode<'a> = &'a (dyn Fn(&Path) -> Result<Linear, MediaError> + Sync);

/// The decision: a cached DNG; else a conversion into a private folder, renamed into place
/// only once complete; else (no converter, a failure, an undecodable DNG) the camera JPEG. A
/// photo that cannot be read is an error of its own, never a reason to convert or to fall back.
pub fn base_for(original: &Path, cache_dir: &Path, tool: Option<&Path>, convert: Convert, decode: Decode) -> Result<Base, MediaError> {
    base_for_capped(original, cache_dir, tool, convert, decode, CACHE_CAP)
}

/// [`base_for`] with the cache's cap given.
pub fn base_for_capped(original: &Path, cache_dir: &Path, tool: Option<&Path>, convert: Convert, decode: Decode, cap: u64) -> Result<Base, MediaError> {
    if !cache_allowed(cache_dir, original) {
        tracing::warn!(cache = %cache_dir.display(), "stand-in: the DNG cache would be inside the photo's library; using the camera rendering");
        return Ok(Base::Camera);
    }
    let hash = content_hash(original)?;
    let dng = cached_dng(cache_dir, &hash);
    if dng.is_file() {
        return Ok(decoded(&dng, &hash, decode));
    }
    let Some(tool) = tool else {
        tracing::info!("stand-in: Adobe DNG Converter not found; using the camera rendering");
        return Ok(Base::Camera);
    };
    if has_failed(&hash) {
        return Ok(Base::Camera);
    }
    let lock = lock_for(&hash);
    let _held = lock.lock().unwrap_or_else(|p| p.into_inner());
    // Another open converted it while this one waited.
    if dng.is_file() {
        return Ok(decoded(&dng, &hash, decode));
    }
    if has_failed(&hash) {
        return Ok(Base::Camera);
    }
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(|| clear_crashed_parts(cache_dir));
    let t = Instant::now();
    match convert_into_place(original, cache_dir, &hash, tool, convert) {
        Ok(()) => {
            tracing::info!(ms = t.elapsed().as_millis() as u64, "stand-in: converted to DNG");
            trim_cache(cache_dir, cap, &dng);
            Ok(decoded(&dng, &hash, decode))
        }
        Err(e) => {
            // The disk went away during the conversion: not the file's fault, and it comes back.
            std::fs::File::open(original)?;
            tracing::warn!("stand-in: Adobe DNG Converter failed ({e}); using the camera rendering");
            mark_failed(&hash);
            Ok(Base::Camera)
        }
    }
}

fn decoded(dng: &Path, hash: &str, decode: Decode) -> Base {
    match decode(dng) {
        Ok(lin) => {
            // Opening is what the cap's "oldest opened" is measured by.
            let opened = std::fs::File::options().write(true).open(dng).and_then(|f| f.set_modified(std::time::SystemTime::now()));
            if let Err(e) = opened {
                tracing::debug!("stand-in: could not mark {} as opened: {e}", dng.display());
            }
            Base::Sensor(lin)
        }
        // Nothing is wrong with the DNG, and the library may come back.
        Err(e) if super::raw::is_no_library(&e) => {
            tracing::warn!("stand-in: {e}; using the camera rendering");
            Base::Camera
        }
        Err(e) => {
            // Derived data under app data, not the photo: a bad copy is dropped.
            tracing::warn!("stand-in: the cached DNG could not be developed ({e}); using the camera rendering");
            let _ = std::fs::remove_file(dng);
            mark_failed(hash);
            Base::Camera
        }
    }
}

fn convert_into_place(original: &Path, cache_dir: &Path, hash: &str, tool: &Path, convert: Convert) -> Result<(), String> {
    static N: AtomicU64 = AtomicU64::new(0);
    let part = cache_dir.join(format!(".part-{hash}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    let out = (|| {
        convert(tool, original, &part)?;
        let made = std::fs::read_dir(&part)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dng")))
            .ok_or_else(|| "it made no DNG".to_string())?;
        std::fs::rename(&made, cached_dng(cache_dir, hash)).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(&part);
    out
}

/// Adobe DNG Converter on `original`, writing `<stem>.dng` into `out_dir`: lossless (`-c`), no
/// embedded preview (`-p0`). Below normal priority, no console window, ended with Lumen
/// (`child_jobs`), and killed after [`CONVERT_TIMEOUT`].
pub fn run_converter(tool: &Path, original: &Path, out_dir: &Path) -> Result<(), String> {
    run_converter_for(tool, original, out_dir, CONVERT_TIMEOUT)
}

/// `p` absolute with the platform's own separators. Adobe DNG Converter on Windows refuses a
/// path with a `/` in it ("Failed to convert", exit code 0), and a library path is
/// `originals\` joined with `2024/05/01/…`.
pub fn native(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    abs.components().collect()
}

pub fn run_converter_for(tool: &Path, original: &Path, out_dir: &Path, timeout: Duration) -> Result<(), String> {
    let (program, prefix) = crate::ffmpeg::launcher(tool, true, cfg!(unix));
    let mut c = crate::ffmpeg::cmd(&program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        c.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }
    let mut child = c
        .args(prefix)
        .args(["-c", "-p0", "-d"])
        .arg(native(out_dir))
        .arg(native(original))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start: {e}"))?;
    photos_core::child_jobs::adopt(&child);
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("it ended with {status}")),
            Ok(None) => {}
            Err(e) => return Err(e.to_string()),
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("it took longer than {} s", timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// A stand-in RAW with its own bytes (each test its own hash).
    fn nef(dir: &Path, tag: &str) -> PathBuf {
        let p = dir.join("DSC_0001.NEF");
        std::fs::write(&p, format!("not really a nef: {tag}")).unwrap();
        p
    }

    /// A converter that writes `<stem>.dng` holding `body`, counting its calls.
    fn fake_converter<'a>(calls: &'a AtomicUsize, body: &'static str) -> impl Fn(&Path, &Path, &Path) -> Result<(), String> + Sync + 'a {
        move |_, original, out| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::fs::write(out.join(original.file_stem().unwrap()).with_extension("dng"), body).map_err(|e| e.to_string())
        }
    }

    /// Decodes a fake DNG: "good" is a 1×1 grey image, anything else fails.
    fn fake_decode(p: &Path) -> Result<Linear, MediaError> {
        match std::fs::read_to_string(p).unwrap_or_default().as_str() {
            "good" => Ok(Linear::new(1, 1, [0.5; 3])),
            other => Err(MediaError::Decode(format!("bad dng: {other}"))),
        }
    }

    fn tool(dir: &Path) -> PathBuf {
        let t = dir.join("converter.exe");
        std::fs::write(&t, "").unwrap();
        t
    }

    #[test]
    fn the_variable_names_the_converter_and_can_hide_it() {
        let dir = tempdir();
        let installed = tool(dir.path());
        let standard = [installed.as_path()];
        assert_eq!(locate(None, standard), Some(installed.clone()), "a standard install");
        assert_eq!(locate(Some(installed.clone().into()), []), Some(installed.clone()), "the variable");
        assert_eq!(locate(Some(dir.path().join("nothing.exe").into()), standard), None, "pointing at nothing hides it");
        assert_eq!(locate(Some(OsString::new()), standard), Some(installed), "an empty variable is unset");
        assert_eq!(locate(None, [dir.path().join("missing").as_path()]), None);
    }

    #[test]
    fn a_miss_converts_once_then_the_cache_answers() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "miss-then-hit");
        let cache = app.path().join("cache/dng");
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let conv = fake_converter(&calls, "good");
        assert_eq!(base_for(&original, &cache, Some(&t), &conv, &fake_decode).unwrap(), Base::Sensor(Linear::new(1, 1, [0.5; 3])));
        assert_eq!(base_for(&original, &cache, Some(&t), &conv, &fake_decode).unwrap(), Base::Sensor(Linear::new(1, 1, [0.5; 3])));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "converted once");
        let hash = content_hash(&original).unwrap();
        assert!(cached_dng(&cache, &hash).is_file(), "cached by content hash");
        // Only the DNG is left: the private folder is gone, nothing sits beside the original.
        let names: Vec<_> = std::fs::read_dir(&cache).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![OsString::from(format!("{hash}.dng"))]);
        assert_eq!(std::fs::read_dir(lib.path()).unwrap().count(), 1, "the original's folder is untouched");
        assert_eq!(std::fs::read_to_string(&original).unwrap(), "not really a nef: miss-then-hit");
        // A cached DNG is used without a converter.
        assert!(matches!(base_for(&original, &cache, None, &conv, &fake_decode), Ok(Base::Sensor(_))));
    }

    #[test]
    fn no_converter_is_the_camera_rendering() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "no converter");
        let calls = AtomicUsize::new(0);
        assert_eq!(base_for(&original, &app.path().join("dng"), None, &fake_converter(&calls, "good"), &fake_decode).unwrap(), Base::Camera);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_failed_conversion_falls_back_and_is_not_retried() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "fails");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let failing = |_: &Path, _: &Path, _: &Path| -> Result<(), String> {
            calls.fetch_add(1, Ordering::SeqCst);
            Err("timed out".into())
        };
        assert_eq!(base_for(&original, &cache, Some(&t), &failing, &fake_decode).unwrap(), Base::Camera);
        assert_eq!(base_for(&original, &cache, Some(&t), &failing, &fake_decode).unwrap(), Base::Camera);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "not retried this session");
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0, "no part folder left behind");
    }

    #[test]
    fn a_converter_that_makes_no_dng_or_a_bad_one_falls_back() {
        let (lib, app) = (tempdir(), tempdir());
        let t = tool(app.path());
        let original = nef(lib.path(), "makes nothing");
        let nothing = |_: &Path, _: &Path, _: &Path| -> Result<(), String> { Ok(()) };
        assert_eq!(base_for(&original, &app.path().join("a"), Some(&t), &nothing, &fake_decode).unwrap(), Base::Camera);

        let original = nef(lib.path(), "makes a bad dng");
        let cache = app.path().join("b");
        let calls = AtomicUsize::new(0);
        assert_eq!(base_for(&original, &cache, Some(&t), &fake_converter(&calls, "garbage"), &fake_decode).unwrap(), Base::Camera);
        assert!(!cached_dng(&cache, &content_hash(&original).unwrap()).exists(), "a DNG rawler cannot develop is dropped");
    }

    #[test]
    fn a_half_written_dng_never_counts() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "half written");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let hash = content_hash(&original).unwrap();
        // A crash mid-conversion leaves only a private folder: not a hit.
        std::fs::create_dir_all(cache.join(format!(".part-{hash}-1-0"))).unwrap();
        std::fs::write(cache.join(format!(".part-{hash}-1-0/DSC_0001.dng")), "go").unwrap();
        let seen = Mutex::new(Vec::new());
        let watching = |_: &Path, original: &Path, out: &Path| -> Result<(), String> {
            let dng = out.join(original.file_stem().unwrap()).with_extension("dng");
            std::fs::write(&dng, "go").unwrap();
            // While the converter writes, the final name does not exist yet.
            seen.lock().unwrap().push(cached_dng(&cache, &hash).exists());
            std::fs::write(&dng, "good").map_err(|e| e.to_string())
        };
        assert!(matches!(base_for(&original, &cache, Some(&t), &watching, &fake_decode), Ok(Base::Sensor(_))));
        assert_eq!(*seen.lock().unwrap(), vec![false]);
        assert_eq!(std::fs::read_to_string(cached_dng(&cache, &hash)).unwrap(), "good");
    }

    #[test]
    fn two_opens_of_one_photo_convert_it_once() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "two at once");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let slow = |_: &Path, original: &Path, out: &Path| -> Result<(), String> {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150));
            std::fs::write(out.join(original.file_stem().unwrap()).with_extension("dng"), "good").map_err(|e| e.to_string())
        };
        std::thread::scope(|s| {
            let a = s.spawn(|| base_for(&original, &cache, Some(&t), &slow, &fake_decode));
            let b = s.spawn(|| base_for(&original, &cache, Some(&t), &slow, &fake_decode));
            assert!(matches!(a.join().unwrap(), Ok(Base::Sensor(_))));
            assert!(matches!(b.join().unwrap(), Ok(Base::Sensor(_))));
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_cache_is_never_inside_the_photos_library_or_beside_it() {
        let root = tempdir();
        let lib = root.path().join("Lumen Library");
        let original = lib.join("originals/2024/05/01/ab12__DSC_0001.NEF");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, "nef").unwrap();
        let app = root.path().join("JL-Studios/Lumen/cache/dng");
        assert!(cache_allowed(&app, &original), "app data");
        assert!(!cache_allowed(&lib.join("cache/dng"), &original), "inside the library");
        assert!(!cache_allowed(&lib.join("originals/x/../dng"), &original), "through ..");
        assert!(!cache_allowed(&lib, &original), "the library itself");
        assert!(!lib.join("cache").exists() && !lib.join("originals/dng").exists(), "nothing was created to decide");
        if cfg!(windows) {
            let shouted = PathBuf::from(lib.to_string_lossy().to_uppercase()).join("DNG");
            assert!(!cache_allowed(&shouted, &original), "case does not hide it on Windows");
        }
        // A loose file (a device, a folder outside any library): never beside it.
        let loose = root.path().join("card/DCIM/DSC_0002.NEF");
        std::fs::create_dir_all(loose.parent().unwrap()).unwrap();
        std::fs::write(&loose, "nef").unwrap();
        assert!(!cache_allowed(&root.path().join("card/DCIM/dng"), &loose));
        assert!(cache_allowed(&app, &loose));
        // And the decision refuses to write there.
        let calls = AtomicUsize::new(0);
        let t = tool(root.path());
        assert_eq!(base_for(&original, &lib.join("cache"), Some(&t), &fake_converter(&calls, "good"), &fake_decode).unwrap(), Base::Camera);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn an_io_error_never_reaches_the_converter() {
        let (lib, app) = (tempdir(), tempdir());
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let missing = lib.path().join("unplugged.NEF");
        let got = base_for(&missing, &app.path().join("dng"), Some(&t), &fake_converter(&calls, "good"), &fake_decode);
        assert!(matches!(got, Err(MediaError::Io(_))), "{got:?}");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a photo that cannot be read is not a format problem");
    }

    #[test]
    fn a_photo_that_vanishes_while_converting_is_not_remembered_as_failed() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "vanishes");
        let hash = content_hash(&original).unwrap();
        let t = tool(app.path());
        let vanishing = |_: &Path, original: &Path, _: &Path| -> Result<(), String> {
            std::fs::remove_file(original).unwrap();
            Err("could not read it".into())
        };
        let got = base_for(&original, &app.path().join("dng"), Some(&t), &vanishing, &fake_decode);
        assert!(matches!(got, Err(MediaError::Io(_))), "{got:?}");
        assert!(!has_failed(&hash), "the disk comes back; the next open tries again");
    }

    /// `n` bytes of finished DNG, last opened `age` seconds ago.
    fn cached(dir: &Path, name: &str, n: usize, age: u64) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(format!("{name}.dng"));
        std::fs::write(&p, vec![0u8; n]).unwrap();
        let when = std::time::SystemTime::now() - Duration::from_secs(age);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(when).unwrap();
        p
    }

    #[test]
    fn the_cache_is_trimmed_oldest_opened_first_to_the_cap() {
        let dir = tempdir();
        let (old, mid, new) = (cached(dir.path(), "old", 100, 300), cached(dir.path(), "mid", 100, 200), cached(dir.path(), "new", 100, 100));
        trim_cache(dir.path(), 250, &dir.path().join("none"));
        assert!(!old.exists(), "the oldest goes first");
        assert!(mid.exists() && new.exists(), "only as much as the cap needs");
        trim_cache(dir.path(), 200, &dir.path().join("none"));
        assert!(mid.exists() && new.exists(), "at the cap nothing goes");
    }

    #[test]
    fn the_photo_being_opened_is_never_trimmed() {
        let dir = tempdir();
        let being_opened = cached(dir.path(), "huge", 1000, 500);
        let other = cached(dir.path(), "other", 10, 10);
        trim_cache(dir.path(), 100, &being_opened);
        assert!(being_opened.exists(), "even when it alone is over the cap");
        assert!(!other.exists());
    }

    #[test]
    fn only_finished_dngs_are_counted_or_trimmed() {
        let dir = tempdir();
        cached(dir.path(), "a", 40, 10);
        cached(dir.path(), "b", 60, 10);
        let part = dir.path().join(format!(".part-x-{}-0", std::process::id()));
        std::fs::create_dir_all(&part).unwrap();
        std::fs::write(part.join("x.dng"), vec![0u8; 5000]).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not ours to touch").unwrap();
        assert_eq!(cache_use(dir.path()), CacheUse { bytes: 100, files: 2 });
        assert_eq!(cache_use(&dir.path().join("never made")), CacheUse { bytes: 0, files: 0 });
        trim_cache(dir.path(), 0, &dir.path().join("none"));
        assert!(part.join("x.dng").exists(), "a conversion in progress is left alone");
        assert!(dir.path().join("notes.txt").exists());
        assert_eq!(cache_use(dir.path()), CacheUse { bytes: 0, files: 0 });
    }

    #[test]
    fn a_hit_counts_as_opened() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "a hit is an open");
        let cache = app.path().join("dng");
        let hash = content_hash(&original).unwrap();
        let dng = cached(&cache, &hash, 4, 5000);
        std::fs::write(&dng, "good").unwrap();
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(5000);
        std::fs::File::options().write(true).open(&dng).unwrap().set_modified(long_ago).unwrap();
        assert!(matches!(base_for(&original, &cache, None, &|_, _, _| Ok(()), &fake_decode), Ok(Base::Sensor(_))));
        let opened = std::fs::metadata(&dng).unwrap().modified().unwrap();
        assert!(opened > long_ago + Duration::from_secs(4000), "the open moved it to the young end");
    }

    #[test]
    fn a_new_conversion_keeps_the_cache_under_the_cap() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "over the cap");
        let cache = app.path().join("dng");
        let old = cached(&cache, "old", 100, 900);
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let got = base_for_capped(&original, &cache, Some(&t), &fake_converter(&calls, "good"), &fake_decode, 50);
        assert!(matches!(got, Ok(Base::Sensor(_))));
        assert!(!old.exists(), "the new DNG pushed the oldest out");
        assert!(cached_dng(&cache, &content_hash(&original).unwrap()).exists());
    }

    #[test]
    fn part_folders_left_by_a_crash_are_cleared() {
        let dir = tempdir();
        let mine = dir.path().join(format!(".part-a-{}-0", std::process::id()));
        let crashed = dir.path().join(".part-b-1-0");
        for d in [&mine, &crashed] {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join("half.dng"), "half").unwrap();
        }
        let kept = cached(dir.path(), "kept", 4, 1);
        clear_crashed_parts(dir.path());
        assert!(!crashed.exists());
        assert!(mine.exists(), "this run's own conversion is left alone");
        assert!(kept.exists());
    }

    #[test]
    fn a_format_refusal_goes_to_the_converter() {
        let (lib, app) = (tempdir(), tempdir());
        // What rawler calls an unknown camera or a format with no decoder: bytes it can't place.
        let original = lib.path().join("new-body.ARW");
        std::fs::write(&original, vec![7u8; 4096]).unwrap();
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let conv = fake_converter(&calls, "good");
        let cache = app.path().join("dng");
        let got = super::super::sensor_or_stand_in(&original, &super::super::raw::decode, &|o| Some(base_for(o, &cache, Some(&t), &conv, &fake_decode))).unwrap();
        assert_eq!(got, Some(Linear::new(1, 1, [0.5; 3])), "the DNG's sensor data");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // No stand-in configured: the refusal stands. A converter that fails: the camera's JPEG.
        let unconfigured = super::super::sensor_or_stand_in(&original, &super::super::raw::decode, &|_| None);
        assert!(unconfigured.as_ref().err().is_some_and(super::super::raw::is_refused), "{unconfigured:?}");
        let camera = super::super::sensor_or_stand_in(&original, &super::super::raw::decode, &|_| Some(Ok(Base::Camera))).unwrap();
        assert_eq!(camera, None);
    }

    #[test]
    fn a_missing_library_keeps_the_cached_dngs() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "the library went away");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let calls = AtomicUsize::new(0);
        let conv = fake_converter(&calls, "good");
        assert!(matches!(base_for(&original, &cache, Some(&t), &conv, &fake_decode), Ok(Base::Sensor(_))));
        let no_library = |_: &Path| -> Result<Linear, MediaError> { Err(super::super::raw::no_library()) };
        assert_eq!(base_for(&original, &cache, Some(&t), &conv, &no_library).unwrap(), Base::Camera);
        let hash = content_hash(&original).unwrap();
        assert!(cached_dng(&cache, &hash).is_file(), "the DNG is not the library's fault");
        assert!(!has_failed(&hash), "and neither is the photo");
        assert!(matches!(base_for(&original, &cache, Some(&t), &conv, &fake_decode), Ok(Base::Sensor(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_missing_library_never_runs_the_converter() {
        let got = super::super::sensor_or_stand_in(Path::new("x.NEF"), &|_: &Path| Err(super::super::raw::no_library()), &|_: &Path| panic!("no conversion: nothing could develop its DNG"));
        assert!(got.is_err() || got.is_ok_and(|b| b.is_none()));
    }

    #[test]
    fn a_failed_hash_retries_next_launch() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "fails this launch only");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let failing = |_: &Path, _: &Path, _: &Path| -> Result<(), String> { Err("this version can't".into()) };
        assert!(matches!(base_for(&original, &cache, Some(&t), &failing, &fake_decode), Ok(Base::Camera)));
        let hash = content_hash(&original).unwrap();
        assert!(has_failed(&hash));
        // Nothing about the failure is on disk, so a new process has no way to remember it...
        let left: Vec<_> = std::fs::read_dir(&cache).map(|d| d.map(|e| e.unwrap().path()).collect()).unwrap_or_default();
        assert_eq!(left, Vec::<PathBuf>::new());
        // ...and with the memory gone, as it is after a restart, the photo converts.
        failed().as_mut().unwrap().remove(&hash);
        let calls = AtomicUsize::new(0);
        assert!(matches!(base_for(&original, &cache, Some(&t), &fake_converter(&calls, "good"), &fake_decode), Ok(Base::Sensor(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn clearing_the_cache_lets_a_failed_photo_try_again() {
        let (lib, app) = (tempdir(), tempdir());
        let original = nef(lib.path(), "fails, then the converter is updated");
        let cache = app.path().join("dng");
        let t = tool(app.path());
        let failing = |_: &Path, _: &Path, _: &Path| -> Result<(), String> { Err("this version can't".into()) };
        assert!(matches!(base_for(&original, &cache, Some(&t), &failing, &fake_decode), Ok(Base::Camera)));
        let other = cached(&cache, "other", 10, 1);
        clear_cache(&cache);
        assert!(!other.exists());
        let calls = AtomicUsize::new(0);
        assert!(matches!(base_for(&original, &cache, Some(&t), &fake_converter(&calls, "good"), &fake_decode), Ok(Base::Sensor(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// `LUMEN_HE_NEF=<a Nikon HE / HE* NEF>` (copied into a temp dir first; only the copy is
    /// read). With Adobe DNG Converter found it develops from the DNG, the second open from the
    /// cache; with `LUMEN_DNG_CONVERTER` pointing at nothing, from the camera JPEG. Times both.
    #[test]
    #[ignore = "needs LUMEN_HE_NEF"]
    fn a_real_he_nef_opens_in_develop() {
        use crate::develop::{load_base_as, Rendering};
        let given = PathBuf::from(std::env::var("LUMEN_HE_NEF").expect("LUMEN_HE_NEF"));
        let (work, app) = (tempdir(), tempdir());
        // Shaped like a library path: a space, a non-ASCII letter, and `/` joined onto `\`.
        let folder = work.path().join("Lumen Library ö").join("originals");
        std::fs::create_dir_all(folder.join("2024/05/01")).unwrap();
        let copy = folder.join(format!("2024/05/01/ab12__{}", given.file_name().unwrap().to_string_lossy()));
        std::fs::copy(&given, &copy).unwrap();
        configure(app.path().join("cache/dng"));
        let open = || {
            let t = Instant::now();
            let l = load_base_as(&copy, crate::MediaKind::Raw, None, crate::hdr::HdrKind::None, 1, true, Some(2048)).unwrap();
            (l, t.elapsed())
        };
        let (first, t1) = open();
        let (second, t2) = open();
        eprintln!("converter: {:?}; first open {t1:?} ({}x{}, full {}), second {t2:?}", converter(), first.linear.width, first.linear.height, first.full_long);
        assert_eq!(first, second, "the same base both times");
        assert_eq!(first.linear.long_edge(), 2048);
        if converter().is_some() {
            assert_eq!(first.rendering, Rendering::Sensor);
            let cached: Vec<_> = std::fs::read_dir(app.path().join("cache/dng")).unwrap().map(|e| e.unwrap().path()).collect();
            assert_eq!(cached.len(), 1, "{cached:?}");
            eprintln!("cached DNG: {} bytes", std::fs::metadata(&cached[0]).unwrap().len());
        } else {
            assert_eq!(first.rendering, Rendering::Camera);
            assert!(first.full_long >= 4000, "the full-size camera JPEG, not a thumbnail: {}", first.full_long);
            assert!(!app.path().join("cache/dng").exists() || std::fs::read_dir(app.path().join("cache/dng")).unwrap().count() == 0);
        }
        assert_eq!(std::fs::read_dir(copy.parent().unwrap()).unwrap().count(), 1, "nothing written beside the NEF");
        assert_eq!(std::fs::read_dir(work.path()).unwrap().count(), 1, "nor anywhere in its library");
    }

    #[test]
    fn the_converter_is_given_native_absolute_paths() {
        let mixed = Path::new("C:\\Lumen Library\\originals").join("2024/05/01/ab__DSC_0001.NEF");
        let n = native(&mixed);
        assert!(n.is_absolute());
        if cfg!(windows) {
            assert_eq!(n.to_string_lossy(), "C:\\Lumen Library\\originals\\2024\\05\\01\\ab__DSC_0001.NEF");
        }
        assert!(native(Path::new("relative/x.NEF")).is_absolute());
    }

    #[test]
    fn a_converter_that_cannot_start_is_an_error_not_a_panic() {
        let dir = tempdir();
        let e = run_converter_for(&dir.path().join("missing.exe"), &dir.path().join("a.NEF"), dir.path(), Duration::from_secs(5)).unwrap_err();
        assert!(e.contains("could not start"), "{e}");
    }
}
