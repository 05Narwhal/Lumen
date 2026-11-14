//! RAW decoding for Lumen, built alone as `photos_raw.dll` / `libphotos_raw.so` /
//! `libphotos_raw.dylib` and loaded at run time, so anyone can rebuild it or swap `rawler`
//! (LGPL-2.1) for their own copy. It depends on no other Lumen crate. README.md says how.
//!
//! Every export runs inside `catch_unwind`, and this library is built with `panic = "unwind"`
//! while Lumen's release build aborts: a panic in `rawler` comes back as a status. A stack
//! overflow or an abort is not contained.

// Each export's safety rule is the same one, stated once on `Output` and in the README.
#![allow(clippy::missing_safety_doc)]

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use image::DynamicImage;
use rawler::decoders::{FormatHint, RawDecodeParams};
use rawler::imgop::develop::{Intermediate, ProcessingStep, RawDevelop};

/// Raised whenever an export or `Output` changes. Lumen refuses a library of another version.
pub const PHOTOS_RAW_ABI: u32 = 1;

pub const STATUS_OK: i32 = 0;
/// `rawler` refuses this file because of what it is.
pub const STATUS_REFUSED: i32 = 1;
/// The file could not be read. Says nothing about its format.
pub const STATUS_IO: i32 = 2;
pub const STATUS_FAILED: i32 = 3;
pub const STATUS_PANICKED: i32 = 4;

pub const LAYOUT_NONE: u32 = 0;
/// `[f32; 3]` per pixel, linear sRGB.
pub const LAYOUT_LINEAR: u32 = 1;
pub const LAYOUT_LUMA8: u32 = 2;
pub const LAYOUT_LUMA_A8: u32 = 3;
pub const LAYOUT_RGB8: u32 = 4;
pub const LAYOUT_RGBA8: u32 = 5;
pub const LAYOUT_LUMA16: u32 = 6;
pub const LAYOUT_LUMA_A16: u32 = 7;
pub const LAYOUT_RGB16: u32 = 8;
pub const LAYOUT_RGBA16: u32 = 9;
pub const LAYOUT_JSON: u32 = 10;
/// A status other than `STATUS_OK`: why, in UTF-8.
pub const LAYOUT_TEXT: u32 = 11;

/// What a call gives back, written through `out`, whatever the status. The caller owns it until
/// it passes it to [`photos_raw_free`], exactly once. `data` is aligned for its layout.
#[repr(C)]
pub struct Output {
    pub data: *const u8,
    pub len: usize,
    pub owner: *mut Payload,
    pub width: u32,
    pub height: u32,
    pub layout: u32,
    /// With `STATUS_IO`: the OS's error code, 0 when it has none.
    pub os_error: i32,
}

/// Keeps a call's bytes alive, opaque to the caller.
pub enum Payload {
    Bytes(Vec<u8>),
    Pixels(Vec<[f32; 3]>),
    Words(Vec<u16>),
}

/// A path as the OS holds it: UTF-16 on Windows, bytes elsewhere. `path_len` counts these.
#[cfg(windows)]
pub type PathUnit = u16;
#[cfg(not(windows))]
pub type PathUnit = u8;

unsafe fn path_from(ptr: *const PathUnit, len: usize) -> PathBuf {
    let units = std::slice::from_raw_parts(ptr, len);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_wide(units))
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(units))
    }
}

enum Fail {
    Refused(String),
    Io(std::io::Error),
    Failed(String),
}

struct Done {
    payload: Option<Payload>,
    width: u32,
    height: u32,
    layout: u32,
}

impl Done {
    const NOTHING: Done = Done { payload: None, width: 0, height: 0, layout: LAYOUT_NONE };
}

/// `rawler`'s two kinds of error. `Unsupported` is a refusal. `DecoderFailed` is one too, as its
/// "not supported" for an HE NEF and its unknown DNG compressions are, unless it is the disk
/// talking: `rawler` words every I/O error as "I/O error…" or "I/O Error…".
fn fail(e: rawler::RawlerError) -> Fail {
    match e {
        rawler::RawlerError::DecoderFailed(m) if m.to_ascii_lowercase().contains("i/o error") => Fail::Failed(format!("raw: {m}")),
        e => Fail::Refused(e.to_string()),
    }
}

fn write(out: *mut Output, done: Done) -> i32 {
    let (data, len) = match &done.payload {
        Some(Payload::Bytes(b)) => (b.as_ptr(), b.len()),
        Some(Payload::Pixels(p)) => (p.as_ptr() as *const u8, p.len() * 12),
        Some(Payload::Words(w)) => (w.as_ptr() as *const u8, w.len() * 2),
        None => (std::ptr::null(), 0),
    };
    let owner = done.payload.map_or(std::ptr::null_mut(), |p| Box::into_raw(Box::new(p)));
    // A pointer into a Vec doesn't move when the Vec does.
    unsafe { out.write(Output { data, len, owner, width: done.width, height: done.height, layout: done.layout, os_error: 0 }) };
    STATUS_OK
}

fn write_failure(out: *mut Output, status: i32, why: String, os_error: i32) -> i32 {
    let bytes = why.into_bytes();
    let (data, len) = (bytes.as_ptr(), bytes.len());
    let owner = Box::into_raw(Box::new(Payload::Bytes(bytes)));
    unsafe { out.write(Output { data, len, owner, width: 0, height: 0, layout: LAYOUT_TEXT, os_error }) };
    status
}

fn panic_text(p: &(dyn Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".into())
}

fn guarded(interactive: bool, out: *mut Output, work: impl FnOnce() -> Result<Done, Fail> + Send) -> i32 {
    match catch_unwind(AssertUnwindSafe(|| pool(interactive).install(work))) {
        Ok(Ok(done)) => write(out, done),
        Ok(Err(Fail::Refused(why))) => write_failure(out, STATUS_REFUSED, why, 0),
        Ok(Err(Fail::Failed(why))) => write_failure(out, STATUS_FAILED, why, 0),
        Ok(Err(Fail::Io(e))) => {
            let code = e.raw_os_error().unwrap_or(0);
            write_failure(out, STATUS_IO, e.to_string(), code)
        }
        Err(p) => write_failure(out, STATUS_PANICKED, panic_text(p.as_ref()), 0),
    }
}

/// This library's copy of rayon has its own global pool, which would run a background import's
/// decode at normal priority beside the UI and an interactive one behind it. So there are two
/// pools, and the caller says which: a call waits for its work either way.
fn pool(interactive: bool) -> &'static rayon::ThreadPool {
    static NORMAL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    static LOW: OnceLock<rayon::ThreadPool> = OnceLock::new();
    if interactive {
        NORMAL.get_or_init(|| rayon::ThreadPoolBuilder::new().thread_name(|i| format!("photos-raw-normal-{i}")).build().expect("a thread pool"))
    } else {
        LOW.get_or_init(|| rayon::ThreadPoolBuilder::new().thread_name(|i| format!("photos-raw-low-{i}")).start_handler(|_| below_normal()).build().expect("a thread pool"))
    }
}

/// Lowers the calling thread's priority. Best effort, as Lumen's own background threads do it.
fn below_normal() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
}

/// What [`known_wrong`] needs to know of a file's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Cr2,
    /// Adobe DNG Converter's output among others: several bodies are wrong only in their own format.
    Dng,
    /// `rawler`'s decoder for raw dumps it recognises only by their size (CHDK on Canon compacts).
    Naked,
    Other,
}

/// Bodies `rawler` 0.8 decodes silently wrong, found by Lumen's Round 17 sweep
/// (`docs/RAW-FORMATS.md`): worse than a refusal, so they are refused, and Adobe's converter or
/// the camera's JPEG stands in. `cpp` is `rawler`'s values per pixel.
pub fn known_wrong(make: &str, model: &str, format: Format, cpp: usize) -> Option<&'static str> {
    match (make, model) {
        _ if format == Format::Naked => Some("CHDK raw dumps decode with the wrong colours"),
        ("Canon", _) if format == Format::Cr2 && cpp == 3 => Some("Canon sRAW / mRAW decodes with the wrong colours"),
        ("Canon", "PowerShot G9") => Some("the PowerShot G9 decodes with the wrong colours"),
        ("Nikon", "D1X") => Some("the D1X's pixels are not square"),
        // The F828's DNG has four colours, which rawler's demosaic panics on.
        ("Sony", "DSC-F828") => Some("this camera decodes with the wrong colours"),
        // Their DNGs from the converter decode right.
        ("Kodak", "DCS Pro 14n" | "DCS Pro 14nx" | "DCS Pro SLR/c" | "DCS Pro SLR/n" | "EasyShare Z981" | "EasyShare Z990" | "P712") | ("Olympus", "E-10") if format != Format::Dng => {
            Some("this camera decodes with the wrong colours")
        }
        ("Phase One", "H 25") if format != Format::Dng => Some("this camera's frame decodes garbled"),
        // Their DNGs, the converter's included, come out garish or all but white.
        (m, "SM-G973U" | "Galaxy S23+") if m.eq_ignore_ascii_case("samsung") => Some("this phone's DNG decodes with the wrong colours"),
        _ => None,
    }
}

/// Runs `develop` unless the sweep found this body wrong: the F828's four colours make `rawler`'s
/// demosaic panic, so the check comes before it, not after.
fn unless_known_wrong<T>(make: &str, model: &str, format: Format, cpp: usize, develop: impl FnOnce() -> Result<T, Fail>) -> Result<T, Fail> {
    match known_wrong(make, model, format, cpp) {
        Some(why) => Err(Fail::Refused(why.into())),
        None => develop(),
    }
}

/// Every step but `SRgb`: the result is Lumen's working space, linear sRGB.
const STEPS: [ProcessingStep; 7] = [
    ProcessingStep::Rescale,
    ProcessingStep::Demosaic,
    ProcessingStep::FujiRotate,
    ProcessingStep::CropActiveArea,
    ProcessingStep::WhiteBalance,
    ProcessingStep::Calibrate,
    ProcessingStep::CropDefault,
];

fn decode_linear(path: &Path) -> Result<Done, Fail> {
    // A file that cannot be opened says nothing about its format.
    let src = rawler::rawsource::RawSource::new(path).map_err(Fail::Io)?;
    let dec = rawler::get_decoder(&src).map_err(fail)?;
    let raw = dec.raw_image(&src, &RawDecodeParams::default(), false).map_err(fail)?;
    let format = match dec.format_hint() {
        FormatHint::CR2 => Format::Cr2,
        FormatHint::DNG => Format::Dng,
        FormatHint::Unknown => Format::Naked,
        _ => Format::Other,
    };
    let inter = unless_known_wrong(&raw.clean_make, &raw.clean_model, format, raw.cpp, || RawDevelop::new_with(&STEPS).develop_intermediate(&raw).map_err(fail))?;
    let (width, height, pixels) = match inter {
        Intermediate::ThreeColor(px) => (px.width, px.height, px.data),
        Intermediate::Monochrome(px) => (px.width, px.height, px.data.into_iter().map(|v| [v; 3]).collect()),
        // Four-colour data is mapped to RGB by calibration; left four-colour means no matrix.
        Intermediate::FourColor(px) => (px.width, px.height, px.data.into_iter().map(|v| [v[0], (v[1] + v[3]) / 2.0, v[2]]).collect()),
    };
    Ok(Done { payload: Some(Payload::Pixels(pixels)), width: width as u32, height: height as u32, layout: LAYOUT_LINEAR })
}

fn words(width: u32, height: u32, layout: u32, data: Vec<u16>) -> Done {
    Done { payload: Some(Payload::Words(data)), width, height, layout }
}

fn bytes(width: u32, height: u32, layout: u32, data: Vec<u8>) -> Done {
    Done { payload: Some(Payload::Bytes(data)), width, height, layout }
}

fn image_done(i: DynamicImage) -> Done {
    let (w, h) = (i.width(), i.height());
    match i {
        DynamicImage::ImageLuma8(b) => bytes(w, h, LAYOUT_LUMA8, b.into_raw()),
        DynamicImage::ImageLumaA8(b) => bytes(w, h, LAYOUT_LUMA_A8, b.into_raw()),
        DynamicImage::ImageRgb8(b) => bytes(w, h, LAYOUT_RGB8, b.into_raw()),
        DynamicImage::ImageRgba8(b) => bytes(w, h, LAYOUT_RGBA8, b.into_raw()),
        DynamicImage::ImageLuma16(b) => words(w, h, LAYOUT_LUMA16, b.into_raw()),
        DynamicImage::ImageLumaA16(b) => words(w, h, LAYOUT_LUMA_A16, b.into_raw()),
        DynamicImage::ImageRgb16(b) => words(w, h, LAYOUT_RGB16, b.into_raw()),
        DynamicImage::ImageRgba16(b) => words(w, h, LAYOUT_RGBA16, b.into_raw()),
        other => words(w, h, LAYOUT_RGBA16, other.to_rgba16().into_raw()),
    }
}

/// `rawler`'s own preview, full image or thumbnail. Nothing, with `STATUS_OK`, when it has none.
fn preview(path: &Path) -> Result<Done, Fail> {
    let src = rawler::rawsource::RawSource::new(path).map_err(Fail::Io)?;
    let Ok(dec) = rawler::get_decoder(&src) else { return Ok(Done::NOTHING) };
    let params = RawDecodeParams::default();
    // In this order, stopping at the first: `full_image` panics on some files whose preview is
    // fine (a DJI Osmo Action DNG).
    let found = dec
        .preview_image(&src, &params)
        .ok()
        .flatten()
        .or_else(|| dec.full_image(&src, &params).ok().flatten())
        .or_else(|| dec.thumbnail_image(&src, &params).ok().flatten());
    Ok(found.map_or(Done::NOTHING, image_done))
}

fn metadata(path: &Path) -> Result<Done, Fail> {
    let src = rawler::rawsource::RawSource::new(path).map_err(Fail::Io)?;
    let dec = rawler::get_decoder(&src).map_err(fail)?;
    let md = dec.raw_metadata(&src, &RawDecodeParams::default()).map_err(fail)?;
    let json = serde_json::json!({
        "make": md.make,
        "model": md.model,
        "serial": md.exif.serial_number,
        "date_time_original": md.exif.date_time_original,
    });
    Ok(bytes(0, 0, LAYOUT_JSON, json.to_string().into_bytes()))
}

#[no_mangle]
pub extern "C" fn photos_raw_abi() -> u32 {
    PHOTOS_RAW_ABI
}

#[no_mangle]
pub unsafe extern "C" fn photos_raw_decode_linear(path: *const PathUnit, path_len: usize, interactive: u8, out: *mut Output) -> i32 {
    let path = path_from(path, path_len);
    guarded(interactive != 0, out, move || decode_linear(&path))
}

#[no_mangle]
pub unsafe extern "C" fn photos_raw_preview(path: *const PathUnit, path_len: usize, interactive: u8, out: *mut Output) -> i32 {
    let path = path_from(path, path_len);
    guarded(interactive != 0, out, move || preview(&path))
}

#[no_mangle]
pub unsafe extern "C" fn photos_raw_metadata(path: *const PathUnit, path_len: usize, interactive: u8, out: *mut Output) -> i32 {
    let path = path_from(path, path_len);
    guarded(interactive != 0, out, move || metadata(&path))
}

#[no_mangle]
pub unsafe extern "C" fn photos_raw_free(out: *mut Output) {
    let owner = (*out).owner;
    if !owner.is_null() {
        drop(Box::from_raw(owner));
        (*out).owner = std::ptr::null_mut();
    }
}

/// For Lumen's tests of the boundary: panics inside the guard, as `rawler` can.
#[no_mangle]
pub unsafe extern "C" fn photos_raw_test_panic(out: *mut Output) -> i32 {
    guarded(false, out, || panic!("a test panic"))
}

/// For Lumen's tests of the pools: the thread the work ran on and whether it was below normal.
#[no_mangle]
pub unsafe extern "C" fn photos_raw_test_where(interactive: u8, out: *mut Output) -> i32 {
    guarded(interactive != 0, out, || {
        let json = serde_json::json!({ "thread": std::thread::current().name(), "below_normal": is_below_normal() });
        Ok(bytes(0, 0, LAYOUT_JSON, json.to_string().into_bytes()))
    })
}

fn is_below_normal() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_NORMAL};
        GetThreadPriority(GetCurrentThread()) < THREAD_PRIORITY_NORMAL
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::getpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t) > 0
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawler::RawlerError;

    fn unsupported(what: &str) -> RawlerError {
        RawlerError::Unsupported { what: what.into(), model: "X".into(), make: "Y".into(), mode: String::new() }
    }

    fn empty() -> Output {
        Output { data: std::ptr::null(), len: 0, owner: std::ptr::null_mut(), width: 0, height: 0, layout: LAYOUT_NONE, os_error: 0 }
    }

    #[test]
    fn what_rawler_says_about_a_format_is_a_refusal() {
        // Its "not supported" for an HE NEF comes as a plain decoder failure.
        assert!(matches!(fail(RawlerError::DecoderFailed("NEF compression Some(HighEfficencyStar) is not supported".into())), Fail::Refused(_)));
        assert!(matches!(fail(unsupported("Unknown camera")), Fail::Refused(_)));
        assert!(matches!(fail(unsupported("No decoder found")), Fail::Refused(_)));
        assert!(matches!(fail(RawlerError::DecoderFailed("TODO: Unsupported DNG compression".into())), Fail::Refused(_)));
    }

    #[test]
    fn what_rawler_says_about_the_disk_is_not_a_refusal() {
        for io in [RawlerError::DecoderFailed("I/O Error without context: the device is not ready".into()), RawlerError::DecoderFailed("I/O error in context 'x', y on file: z".into())] {
            assert!(matches!(fail(io), Fail::Failed(_)));
        }
    }

    #[test]
    fn a_panic_comes_back_as_a_status() {
        let mut out = empty();
        assert_eq!(unsafe { photos_raw_test_panic(&mut out) }, STATUS_PANICKED);
        assert_eq!(out.layout, LAYOUT_TEXT);
        unsafe { photos_raw_free(&mut out) };
        assert!(out.owner.is_null());
    }

    #[test]
    fn a_known_wrong_body_is_refused_before_rawler_develops_it() {
        let developed = std::cell::Cell::new(false);
        let got = unless_known_wrong("Sony", "DSC-F828", Format::Dng, 1, || {
            developed.set(true);
            Ok(())
        });
        assert!(matches!(got, Err(Fail::Refused(_))));
        assert!(!developed.get(), "its demosaic panics, so it must never run");
        assert!(unless_known_wrong("Nikon", "Z 6_2", Format::Other, 1, || Ok(())).is_ok());
    }

    #[test]
    fn the_bodies_rawler_gets_wrong_are_named() {
        use Format::*;
        // Canon sRAW / mRAW come out green: three values per pixel in a CR2.
        assert!(known_wrong("Canon", "EOS 6D", Cr2, 3).is_some());
        assert!(known_wrong("Canon", "EOS 6D", Cr2, 1).is_none(), "a plain CR2 is fine");
        assert!(known_wrong("Canon", "EOS 6D", Other, 3).is_none(), "the converter's linear DNG of it is fine");
        // The G9 comes out pink even through the converter; the D1X's pixels aren't square.
        assert!(known_wrong("Canon", "PowerShot G9", Cr2, 1).is_some());
        assert!(known_wrong("Canon", "PowerShot G9", Other, 1).is_some());
        assert!(known_wrong("Nikon", "D1X", Other, 1).is_some());
        assert!(known_wrong("Canon", "PowerShot G9 X", Cr2, 1).is_none());
        assert!(known_wrong("Nikon", "D1H", Other, 1).is_none());
        // CHDK's raw dumps, found by file size, come out green whatever the camera.
        assert!(known_wrong("Canon", "PowerShot A480", Naked, 1).is_some());
        // Found by eye on the sweep's contact sheets: casts, a garbled frame.
        for (make, model) in [("Kodak", "DCS Pro 14n"), ("Kodak", "DCS Pro SLR/c"), ("Kodak", "EasyShare Z990"), ("Kodak", "P712"), ("Sony", "DSC-F828"), ("Olympus", "E-10"), ("Phase One", "H 25")] {
            assert!(known_wrong(make, model, Other, 1).is_some(), "{model}");
        }
        // The converter's DNG of these decodes right, so it isn't refused...
        for (make, model) in [("Kodak", "DCS Pro 14n"), ("Kodak", "DCS Pro 14nx"), ("Kodak", "DCS Pro SLR/c"), ("Kodak", "DCS Pro SLR/n"), ("Kodak", "EasyShare Z981"), ("Kodak", "EasyShare Z990"), ("Kodak", "P712"), ("Olympus", "E-10"), ("Phase One", "H 25")] {
            assert!(known_wrong(make, model, Dng, 1).is_none(), "{model}");
        }
        // ...but the F828's DNG makes rawler's demosaic panic (four colours), and the G9's and
        // D1X's stay wrong, so those stay refused before rawler develops them.
        for (make, model) in [("Sony", "DSC-F828"), ("Canon", "PowerShot G9"), ("Nikon", "D1X")] {
            assert!(known_wrong(make, model, Dng, 1).is_some(), "{model}");
        }
        // Two Samsung phones' converted DNGs come out wrong (garish; all but white) without
        // tripping Lumen's white check; the converter writes the make in lower case.
        assert!(known_wrong("samsung", "SM-G973U", Dng, 1).is_some());
        assert!(known_wrong("Samsung", "Galaxy S23+", Dng, 1).is_some());
        assert!(known_wrong("samsung", "Galaxy S23", Dng, 1).is_none(), "its DNG decodes right");
        assert!(known_wrong("samsung", "SM-G973F", Dng, 1).is_none());
        assert!(known_wrong("Kodak", "DCS760C", Other, 1).is_none());
        assert!(known_wrong("Kodak", "P880", Other, 1).is_none());
    }
}
