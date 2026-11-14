//! Loads `photos-raw`, the separate library holding all of Lumen's `rawler` use (LGPL-2.1), and
//! calls it. A library that's missing, of another ABI or not ours at all is a state: RAW files
//! keep their camera JPEG, metadata comes from EXIF. Loading is by absolute path only, so a DLL
//! planted anywhere else is never picked up.

use std::ffi::{c_void, OsString};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use image::{DynamicImage, ImageBuffer};

use crate::MediaError;

/// The one version of the interface this build speaks (`photos-raw`'s `PHOTOS_RAW_ABI`).
pub const ABI: u32 = 1;

#[cfg(windows)]
pub const FILE_NAME: &str = "photos_raw.dll";
#[cfg(target_os = "macos")]
pub const FILE_NAME: &str = "libphotos_raw.dylib";
#[cfg(not(any(windows, target_os = "macos")))]
pub const FILE_NAME: &str = "libphotos_raw.so";

const STATUS_OK: i32 = 0;
const STATUS_REFUSED: i32 = 1;
const STATUS_IO: i32 = 2;
const STATUS_PANICKED: i32 = 4;

const LAYOUT_NONE: u32 = 0;
const LAYOUT_LINEAR: u32 = 1;
const LAYOUT_LUMA8: u32 = 2;
const LAYOUT_LUMA_A8: u32 = 3;
const LAYOUT_RGB8: u32 = 4;
const LAYOUT_RGBA8: u32 = 5;
const LAYOUT_LUMA16: u32 = 6;
const LAYOUT_LUMA_A16: u32 = 7;
const LAYOUT_RGB16: u32 = 8;
const LAYOUT_RGBA16: u32 = 9;

#[repr(C)]
struct Output {
    data: *const u8,
    len: usize,
    owner: *mut c_void,
    width: u32,
    height: u32,
    layout: u32,
    os_error: i32,
}

impl Output {
    fn empty() -> Self {
        Output { data: std::ptr::null(), len: 0, owner: std::ptr::null_mut(), width: 0, height: 0, layout: LAYOUT_NONE, os_error: 0 }
    }
}

#[cfg(windows)]
type PathUnit = u16;
#[cfg(not(windows))]
type PathUnit = u8;

type Entry = unsafe extern "C" fn(*const PathUnit, usize, u8, *mut Output) -> i32;
type Free = unsafe extern "C" fn(*mut Output);
type TestPanic = unsafe extern "C" fn(*mut Output) -> i32;
type TestWhere = unsafe extern "C" fn(u8, *mut Output) -> i32;

fn path_units(path: &Path) -> Vec<PathUnit> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
}

/// Why there is no library to call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    Missing,
    /// It would not load, or doesn't have this interface's exports.
    Foreign(String),
    OtherAbi(u32),
}

/// A loaded `photos-raw`. Kept for the life of the process.
pub struct Library {
    decode_linear: Entry,
    preview: Entry,
    metadata: Entry,
    free: Free,
    test_panic: TestPanic,
    test_where: TestWhere,
    _lib: libloading::Library,
}

/// What came back from a call, freed when dropped.
struct Reply<'a> {
    out: Output,
    lib: &'a Library,
}

impl Drop for Reply<'_> {
    fn drop(&mut self) {
        unsafe { (self.lib.free)(&mut self.out) };
    }
}

impl Reply<'_> {
    fn bytes(&self) -> &[u8] {
        if self.out.data.is_null() {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.out.data, self.out.len) }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(self.bytes()).into_owned()
    }
}

/// The caller's status and reply as a media error, unless it succeeded.
fn checked<'a>(status: i32, reply: Reply<'a>) -> Result<Reply<'a>, MediaError> {
    match status {
        STATUS_OK => Ok(reply),
        STATUS_REFUSED => Err(crate::develop::raw::refused(reply.text())),
        STATUS_IO => Err(MediaError::Io(match reply.out.os_error {
            0 => std::io::Error::other(reply.text()),
            code => std::io::Error::from_raw_os_error(code),
        })),
        STATUS_PANICKED => Err(crate::develop::raw::refused("the decoder gave up on this file")),
        _ => Err(MediaError::Decode(reply.text())),
    }
}

/// A background thread's decode runs below normal in the library as it does in Lumen; only
/// work on the interactive pool (`photos_jobs::interactive`) runs at normal priority.
fn interactive() -> u8 {
    std::thread::current().name().is_some_and(|n| n.starts_with("lumen-render")) as u8
}

impl Library {
    /// The library at the absolute path `path`, checked to be this interface at version `abi`.
    pub fn open(path: &Path, abi: u32) -> Result<Library, Unavailable> {
        if !path.is_absolute() {
            return Err(Unavailable::Foreign(format!("{} is not an absolute path", path.display())));
        }
        if !path.is_file() {
            return Err(Unavailable::Missing);
        }
        // Loading runs a foreign file's start-up code: the price of letting people swap it,
        // and only ever for the one path Lumen was told about.
        let lib = unsafe { libloading::Library::new(path) }.map_err(|e| Unavailable::Foreign(e.to_string()))?;
        unsafe {
            let found: u32 = {
                let get: libloading::Symbol<unsafe extern "C" fn() -> u32> = lib.get(b"photos_raw_abi\0").map_err(|e| Unavailable::Foreign(e.to_string()))?;
                get()
            };
            if found != abi {
                return Err(Unavailable::OtherAbi(found));
            }
            fn sym<T: Copy>(lib: &libloading::Library, name: &[u8]) -> Result<T, Unavailable> {
                unsafe { lib.get::<T>(name) }.map(|s| *s).map_err(|e| Unavailable::Foreign(e.to_string()))
            }
            Ok(Library {
                decode_linear: sym(&lib, b"photos_raw_decode_linear\0")?,
                preview: sym(&lib, b"photos_raw_preview\0")?,
                metadata: sym(&lib, b"photos_raw_metadata\0")?,
                free: sym(&lib, b"photos_raw_free\0")?,
                test_panic: sym(&lib, b"photos_raw_test_panic\0")?,
                test_where: sym(&lib, b"photos_raw_test_where\0")?,
                _lib: lib,
            })
        }
    }

    fn call(&self, entry: Entry, path: &Path) -> Result<Reply<'_>, MediaError> {
        let units = path_units(path);
        let mut out = Output::empty();
        let status = unsafe { entry(units.as_ptr(), units.len(), interactive(), &mut out) };
        checked(status, Reply { out, lib: self })
    }

    /// Calibrated sensor data: width, height and linear sRGB pixels.
    pub fn decode_linear(&self, path: &Path) -> Result<(u32, u32, Vec<[f32; 3]>), MediaError> {
        let r = self.call(self.decode_linear, path)?;
        let (w, h) = (r.out.width, r.out.height);
        let aligned = (r.out.data as usize).is_multiple_of(std::mem::align_of::<[f32; 3]>());
        if r.out.layout != LAYOUT_LINEAR || !aligned || r.out.len != w as usize * h as usize * 12 {
            return Err(MediaError::Decode("raw: the RAW library returned a malformed image".into()));
        }
        let from = unsafe { std::slice::from_raw_parts(r.out.data as *const [f32; 3], r.out.len / 12) };
        Ok((w, h, parallel_copy(from)))
    }

    /// The library's preview of the file, `None` when it has none or panicked on it. Only a
    /// file that can't be read is an error.
    pub fn preview(&self, path: &Path) -> Result<Option<DynamicImage>, MediaError> {
        match self.call(self.preview, path) {
            Ok(r) => Ok(image_of(&r)),
            Err(e @ MediaError::Io(_)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    /// What the library reads of the file. `None` for a file it can't place.
    pub fn metadata(&self, path: &Path) -> Option<RawMetadata> {
        let r = self.call(self.metadata, path).ok()?;
        serde_json::from_slice(r.bytes()).ok()
    }

    /// Panics inside the library, for tests: the status it comes back as.
    pub fn test_panic_status(&self) -> i32 {
        let mut out = Output::empty();
        let status = unsafe { (self.test_panic)(&mut out) };
        drop(Reply { out, lib: self });
        status
    }

    /// `(thread name, below normal)` of where the library's work ran, for tests.
    pub fn test_where(&self, interactive: bool) -> (String, bool) {
        let mut out = Output::empty();
        let status = unsafe { (self.test_where)(interactive as u8, &mut out) };
        let r = Reply { out, lib: self };
        assert_eq!(status, STATUS_OK);
        let v: serde_json::Value = serde_json::from_slice(r.bytes()).unwrap();
        (v["thread"].as_str().unwrap_or_default().to_owned(), v["below_normal"].as_bool().unwrap_or(false))
    }
}

/// The library's memory can't become Lumen's (a replaced library may allocate any way it likes),
/// so pixels are copied once. Over a 24 MP image's 288 MB, on one thread, that was all of the
/// move's 11 % cost in a decode. The zeroed pages are only touched by the workers.
fn parallel_copy(from: &[[f32; 3]]) -> Vec<[f32; 3]> {
    use rayon::prelude::*;
    let mut to = vec![[0f32; 3]; from.len()];
    to.par_chunks_mut(1 << 18).zip(from.par_chunks(1 << 18)).for_each(|(t, f)| t.copy_from_slice(f));
    to
}

/// What `photos_raw_metadata` gives.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RawMetadata {
    #[serde(default)]
    pub make: String,
    #[serde(default)]
    pub model: String,
    pub serial: Option<String>,
    pub date_time_original: Option<String>,
}

fn image_of(r: &Reply<'_>) -> Option<DynamicImage> {
    let (w, h) = (r.out.width, r.out.height);
    let b = r.bytes();
    // The buffers' `from_raw` refuses a size that doesn't fit the dimensions.
    let words = || b.as_chunks::<2>().0.iter().map(|c| u16::from_ne_bytes(*c)).collect::<Vec<u16>>();
    match r.out.layout {
        LAYOUT_LUMA8 => ImageBuffer::from_raw(w, h, b.to_vec()).map(DynamicImage::ImageLuma8),
        LAYOUT_LUMA_A8 => ImageBuffer::from_raw(w, h, b.to_vec()).map(DynamicImage::ImageLumaA8),
        LAYOUT_RGB8 => ImageBuffer::from_raw(w, h, b.to_vec()).map(DynamicImage::ImageRgb8),
        LAYOUT_RGBA8 => ImageBuffer::from_raw(w, h, b.to_vec()).map(DynamicImage::ImageRgba8),
        LAYOUT_LUMA16 => ImageBuffer::from_raw(w, h, words()).map(DynamicImage::ImageLuma16),
        LAYOUT_LUMA_A16 => ImageBuffer::from_raw(w, h, words()).map(DynamicImage::ImageLumaA16),
        LAYOUT_RGB16 => ImageBuffer::from_raw(w, h, words()).map(DynamicImage::ImageRgb16),
        LAYOUT_RGBA16 => ImageBuffer::from_raw(w, h, words()).map(DynamicImage::ImageRgba16),
        _ => None,
    }
}

static CONFIGURED: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Where the app installed the library (Tauri's resources folder). Set before the first RAW.
pub fn configure(path: impl Into<PathBuf>) {
    *CONFIGURED.write().unwrap_or_else(|p| p.into_inner()) = Some(path.into());
}

/// The paths to try, in order. `LUMEN_PHOTOS_RAW`, when set, is the only one: pointing it at
/// nothing hides the library. Then the configured path, beside the executable, and in a debug
/// build the build's own output, for tests.
pub fn candidates(env: Option<OsString>, configured: Option<PathBuf>, exe_dir: Option<PathBuf>, built: Option<&str>) -> Vec<PathBuf> {
    if let Some(v) = env.filter(|v| !v.is_empty()) {
        return vec![PathBuf::from(v)];
    }
    let mut all: Vec<PathBuf> = configured.into_iter().collect();
    all.extend(exe_dir.map(|d| d.join(FILE_NAME)));
    all.extend(built.map(PathBuf::from));
    all.retain(|p| p.is_absolute());
    all
}

/// The library for this process, loaded on first use; `None` is a state.
pub fn current() -> Option<&'static Library> {
    static LOADED: OnceLock<Option<Library>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
            let built = if cfg!(debug_assertions) { option_env!("PHOTOS_RAW_BUILT") } else { None };
            let configured = CONFIGURED.read().unwrap_or_else(|p| p.into_inner()).clone();
            let mut why = Unavailable::Missing;
            for path in candidates(std::env::var_os("LUMEN_PHOTOS_RAW"), configured, exe_dir, built) {
                match Library::open(&path, ABI) {
                    Ok(lib) => {
                        tracing::info!(path = %path.display(), "RAW library loaded");
                        return Some(lib);
                    }
                    Err(Unavailable::Missing) => {}
                    Err(e) => {
                        tracing::warn!(path = %path.display(), "RAW library not usable: {e:?}");
                        why = e;
                    }
                }
            }
            tracing::warn!("no RAW library ({why:?}): RAW files keep their camera JPEG");
            None
        })
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> &'static Path {
        Path::new(env!("PHOTOS_RAW_BUILT"))
    }

    fn real() -> Library {
        Library::open(built(), ABI).expect("the library built by build.rs")
    }

    #[test]
    fn the_variable_is_the_only_path_when_set() {
        let conf = PathBuf::from(if cfg!(windows) { "C:/a/x.dll" } else { "/a/x.so" });
        assert_eq!(candidates(Some("/nothing".into()), Some(conf.clone()), None, None), vec![PathBuf::from("/nothing")], "pointing it at nothing hides the library");
        assert_eq!(candidates(Some(OsString::new()), Some(conf.clone()), None, None), vec![conf.clone()], "empty is unset");
    }

    #[test]
    fn a_relative_path_is_never_tried() {
        assert!(candidates(None, Some(PathBuf::from("photos_raw.dll")), None, Some("target/x.dll")).is_empty());
        let got = Library::open(Path::new("photos_raw.dll"), ABI);
        assert!(matches!(got, Err(Unavailable::Foreign(_))), "a planted DLL in the current folder is never loaded");
    }

    #[test]
    fn a_missing_library_is_a_state() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(Library::open(&dir.path().join(FILE_NAME), ABI), Err(Unavailable::Missing)));
    }

    #[test]
    fn a_library_of_another_abi_is_a_state() {
        assert!(matches!(Library::open(built(), ABI + 1), Err(Unavailable::OtherAbi(ABI))));
    }

    #[test]
    fn a_foreign_dll_is_a_state() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join(FILE_NAME);
        std::fs::write(&text, "not a library at all").unwrap();
        assert!(matches!(Library::open(&text, ABI), Err(Unavailable::Foreign(_))));
        // A real library that isn't this one.
        #[cfg(windows)]
        assert!(matches!(Library::open(Path::new(r"C:\Windows\System32\version.dll"), ABI), Err(Unavailable::Foreign(_))));
    }

    #[test]
    fn a_panic_inside_the_library_is_returned() {
        assert_eq!(real().test_panic_status(), STATUS_PANICKED);
    }

    #[test]
    fn interactive_decodes_run_on_the_normal_pool() {
        let (thread, low) = real().test_where(true);
        assert!(thread.starts_with("photos-raw-normal"), "{thread}");
        assert!(!low);
    }

    #[test]
    fn background_decodes_below_normal() {
        let (thread, low) = real().test_where(false);
        assert!(thread.starts_with("photos-raw-low"), "{thread}");
        #[cfg(any(windows, target_os = "linux"))]
        assert!(low);
        let _ = low;
    }

    #[test]
    fn only_a_thread_of_the_interactive_pool_asks_for_normal_priority() {
        assert_eq!(interactive(), 0);
        let on_render = std::thread::Builder::new().name("lumen-render-0".into()).spawn(interactive).unwrap().join().unwrap();
        assert_eq!(on_render, 1);
    }

    #[test]
    fn the_copy_keeps_every_pixel_in_place() {
        let from: Vec<[f32; 3]> = (0..(1 << 19) + 7).map(|i| [i as f32, i as f32 + 0.5, -(i as f32)]).collect();
        assert_eq!(parallel_copy(&from), from);
        assert!(parallel_copy(&[]).is_empty());
    }

    #[test]
    fn what_the_library_refuses_comes_back_as_a_refusal() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new-format.NEF");
        std::fs::write(&p, vec![7u8; 4096]).unwrap();
        let got = real().decode_linear(&p);
        assert!(got.as_ref().err().is_some_and(crate::develop::raw::is_refused), "{:?}", got.as_ref().err());
        assert!(real().preview(&p).unwrap().is_none());
        assert!(real().metadata(&p).is_none());
    }

    #[test]
    fn a_file_that_cant_be_read_is_an_io_error_with_its_kind() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("unplugged.NEF");
        for got in [real().decode_linear(&p).map(|_| ()), real().preview(&p).map(|_| ())] {
            assert!(matches!(&got, Err(MediaError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound), "{got:?}");
        }
    }
}
