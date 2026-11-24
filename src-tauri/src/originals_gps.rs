//! GPS into originals (Round 24 §8.7, §11): an estimated place written into an original file
//! that has **no** GPS, verified before it replaces the file, and removed the same verified way
//! on undo.
//!
//! Per file: the GPS is re-read from the file itself (EXIF, XMP and QuickTime — never the DB);
//! a file with any location is refused. The original is copied to a temp file beside it, ExifTool
//! writes `GPSLatitude/Ref`, `GPSLongitude/Ref` and the marker `GPSProcessingMethod =
//! ESTIMATED:<plugin id>` into the **temp** file, and the temp file is verified: the GPS reads
//! back within 1e-6°, every other tag is what the original had, every embedded image or strip
//! (thumbnail, preview, `JpgFromRaw`, raw data) is byte-identical, and the image still decodes
//! (a RAW also through `photos-raw` when the original did). Only then is it `fsync`ed and renamed
//! over the original. Any failure removes the temp file and leaves the original untouched.
//!
//! Formats written: JPEG, HEIC/HEIF, PNG, TIFF, DNG, NEF. Anything else is DB-only
//! ([`WriteOutcome::DbOnly`]). There is no HEIC fixture to test with (FFmpeg here has no HEIF
//! muxer), so HEIC is covered by the same verification at run time only.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use photos_media::exiftool::Session;
use photos_media::{ExifTool, Ffmpeg};
use serde_json::{Map, Value};

use crate::error::{AppError, AppResult};

/// Extensions whose originals take GPS (spec §8.7, this round).
pub const FILE_FORMATS: &[&str] = &["jpg", "jpeg", "jpe", "heic", "heif", "hif", "png", "tif", "tiff", "dng", "nef"];
/// What the marker tag starts with: estimated places stay distinguishable from camera GPS.
pub const MARKER_PREFIX: &str = "ESTIMATED:";

/// What writing one file did.
#[derive(Debug, Clone, PartialEq)]
pub enum WriteOutcome {
    /// The file now carries the GPS; `file_hash` is blake3 of its new bytes.
    Written { file_hash: String },
    /// A format this round does not write (videos, CR3, ARW, …): the place lives in Lumen only.
    DbOnly,
    /// Not written, the original untouched: `has_gps: <tag>`, `bad_coordinates`, `write: …` or
    /// `verify: …`.
    Refused(String),
}

/// What undoing one file did.
#[derive(Debug, Clone, PartialEq)]
pub enum UndoOutcome {
    /// The written GPS is gone; `file_hash` is blake3 of the file's bytes now.
    Removed { file_hash: String },
    /// Left alone: `unreachable`, `changed` (not the bytes Lumen wrote), `not_estimated` (the GPS
    /// is not Lumen's marker) or `verify: …`.
    Skipped(String),
}

/// Whether this round writes GPS into a file of this name (else DB-only).
pub fn writes_into_file(path: &Path) -> bool {
    FILE_FORMATS.contains(&photos_media::extension_of(path).as_str())
}

/// One ExifTool session for a batch of writes or undos.
pub struct GpsWriter {
    session: Session,
    ffmpeg: Option<Ffmpeg>,
    /// Tests: runs on the temp file between the write and the verification.
    #[cfg(test)]
    tamper: Option<fn(&Path)>,
}

/// The tags a write adds and an undo removes (group `GPS`).
const WRITTEN: [&str; 5] = ["GPSLatitude", "GPSLatitudeRef", "GPSLongitude", "GPSLongitudeRef", "GPSProcessingMethod"];
/// Any of these, in any group (EXIF, XMP, QuickTime keys), means the file already has a location
/// or a tag a write would replace.
const PRESENT: [&str; 7] = ["GPSLatitude", "GPSLongitude", "GPSLatitudeRef", "GPSLongitudeRef", "GPSPosition", "GPSCoordinates", "GPSProcessingMethod"];
/// ExifTool adds these to a new IFD0 (a PNG or JPEG without EXIF). They are not the camera's and
/// not the place, so the write deletes the ones the original lacked.
const MANDATORY_IFD0: [&str; 4] = ["XResolution", "YResolution", "ResolutionUnit", "YCbCrPositioning"];
/// How close the read-back GPS must be to what was written, in degrees.
const TOLERANCE: f64 = 1e-6;

/// A temp file beside the original, removed on drop unless it replaced the original.
struct Temp {
    path: PathBuf,
    armed: bool,
}

impl Temp {
    /// `<stem>.lumen-gps.<ext>`: the extension is kept because ExifTool tells TIFF-based RAWs
    /// apart by it.
    fn beside(original: &Path) -> Temp {
        let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let ext = original.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Temp { path: original.with_file_name(format!("{stem}.lumen-gps.{ext}")), armed: true }
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
            let mut leftover = self.path.clone().into_os_string();
            leftover.push("_exiftool_tmp");
            let _ = std::fs::remove_file(leftover);
        }
    }
}

fn media_err(e: photos_media::MediaError) -> AppError {
    AppError { code: "exiftool".into(), message: "ExifTool could not read or write the file.".into(), detail: Some(e.to_string()) }
}

fn group_and_name(key: &str) -> (&str, &str) {
    key.split_once(':').unwrap_or(("", key))
}

/// Keys that change with every copy: file system dates, names, ExifTool's own notes.
fn volatile(key: &str) -> bool {
    let (group, _) = group_and_name(key);
    key == "SourceFile" || group == "System" || group == "ExifTool"
}

/// A tag that says where something sits in the file (`StripOffsets`, `ThumbnailOffset`,
/// `PreviewImageStart`, `JpgFromRawStart`) and the tag that says how long it is.
fn length_key(key: &str) -> Option<String> {
    let (group, name) = group_and_name(key);
    let len = if let Some(base) = name.strip_suffix("Offsets") {
        format!("{base}ByteCounts")
    } else if let Some(base) = name.strip_suffix("Offset") {
        format!("{base}Length")
    } else {
        format!("{}Length", name.strip_suffix("Start")?)
    };
    Some(format!("{group}:{len}"))
}

fn numbers(v: &Value) -> Option<Vec<u64>> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| vec![n]),
        Value::String(s) => s.split_whitespace().map(|p| p.parse().ok()).collect(),
        _ => None,
    }
}

/// The bytes `offsets`/`lengths` point at in `path`.
fn pointed_bytes(path: &Path, offsets: &[u64], lengths: &[u64]) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    if offsets.len() != lengths.len() {
        return Err("offsets and lengths disagree".into());
    }
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    let mut out = Vec::new();
    for (&off, &len) in offsets.iter().zip(lengths) {
        if off.checked_add(len).is_none_or(|end| end > size) {
            return Err("points past the end of the file".into());
        }
        f.seek(SeekFrom::Start(off)).map_err(|e| e.to_string())?;
        let start = out.len();
        out.resize(start + len as usize, 0);
        f.read_exact(&mut out[start..]).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// `after` (of file `b`) holds exactly `before`'s tags (of file `a`) but for the ones `added` and
/// `removed` allow, and every embedded image or strip is byte-identical even where it moved.
fn same_tags(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
    a: &Path,
    b: &Path,
    added: &dyn Fn(&str) -> bool,
    removed: &dyn Fn(&str) -> bool,
) -> Result<(), String> {
    for (k, v) in before.iter().filter(|(k, _)| !volatile(k)) {
        let Some(w) = after.get(k) else {
            if removed(k) {
                continue;
            }
            return Err(format!("{k} was removed"));
        };
        let offsets = length_key(k).zip(numbers(v)).zip(numbers(w));
        if let Some(((len, off_a), off_b)) = offsets {
            let (Some(len_a), Some(len_b)) = (before.get(&len).and_then(numbers), after.get(&len).and_then(numbers)) else {
                if v == w {
                    continue;
                }
                return Err(format!("{k} moved and its length is unknown"));
            };
            if pointed_bytes(a, &off_a, &len_a)? != pointed_bytes(b, &off_b, &len_b)? {
                return Err(format!("the data at {k} changed"));
            }
        } else if v != w {
            return Err(format!("{k} changed"));
        }
    }
    for k in after.keys().filter(|k| !volatile(k) && !before.contains_key(*k)) {
        if !added(k) {
            return Err(format!("{k} was added"));
        }
    }
    Ok(())
}

/// The first tag of `t` saying the file has a location (or a tag a write would replace).
fn location_tag(t: &Map<String, Value>) -> Option<&str> {
    t.keys().map(String::as_str).find(|k| {
        let (group, name) = group_and_name(k);
        group != "Composite" && PRESENT.iter().any(|p| name == *p || name.ends_with(p))
    })
}

/// A signed coordinate from `GPS:<name>` and its `Ref` (`-n`: unsigned degrees and `N`/`S`).
fn coordinate(t: &Map<String, Value>, name: &str, negative: &str) -> Option<f64> {
    let v = t.get(&format!("GPS:{name}"))?.as_f64()?;
    let r = t.get(&format!("GPS:{name}Ref"))?.as_str()?;
    Some(if r.eq_ignore_ascii_case(negative) { -v.abs() } else { v.abs() })
}

fn sync_and_hash(path: &Path) -> AppResult<String> {
    // Windows flushes only a handle opened for writing.
    std::fs::OpenOptions::new().write(true).open(path)?.sync_all()?;
    photos_import::content_hash(path).map_err(|e| AppError { code: "io".into(), message: "Could not read the file back.".into(), detail: Some(e.to_string()) })
}

impl GpsWriter {
    pub fn new(exiftool: &ExifTool, ffmpeg: Option<Ffmpeg>) -> AppResult<Self> {
        Ok(GpsWriter {
            session: exiftool.session().map_err(media_err)?,
            ffmpeg,
            #[cfg(test)]
            tamper: None,
        })
    }

    fn read(&mut self, path: &Path) -> AppResult<Map<String, Value>> {
        Ok(self.session.read_json(path, &[]).map_err(media_err)?.as_object().cloned().unwrap_or_default())
    }

    /// Runs ExifTool on `tmp`; `Err(why)` when it did not update the file.
    fn edit(&mut self, mut args: Vec<OsString>, tmp: &Path) -> AppResult<Result<(), String>> {
        args.splice(0..0, ["-P".into(), "-overwrite_original".into()]);
        args.push(tmp.as_os_str().to_owned());
        let out = self.session.run(args).map_err(media_err)?;
        let errors = out.errors();
        if !errors.is_empty() || !out.stdout.contains("1 image files updated") {
            return Ok(Err(errors.first().map_or_else(|| out.stdout.trim().to_string(), |e| e.to_string())));
        }
        Ok(Ok(()))
    }

    /// The temp file still decodes as the photo (the decoder `decode_display_full` uses), and a RAW
    /// that `photos-raw` could develop before still develops.
    fn decodes(&self, original: &Path, tmp: &Path) -> Result<(), String> {
        let kind = photos_media::kind_for_path(original);
        crate::derivatives::decode_original_or_reduced(tmp, kind, self.ffmpeg.as_ref(), photos_media::HdrKind::None).map_err(|e| format!("does not decode: {e}"))?;
        if kind == photos_media::MediaKind::Raw && photos_media::develop::raw::decode(original).is_ok() {
            photos_media::develop::raw::decode(tmp).map_err(|e| format!("the RAW no longer develops: {e}"))?;
        }
        Ok(())
    }

    /// Writes `lat`, `lon` and the marker `ESTIMATED:<marker>` (the plugin id) into `path`.
    pub fn write_gps(&mut self, path: &Path, lat: f64, lon: f64, marker: &str) -> AppResult<WriteOutcome> {
        if !writes_into_file(path) {
            return Ok(WriteOutcome::DbOnly);
        }
        if !(lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0) {
            return Ok(WriteOutcome::Refused("bad_coordinates".into()));
        }
        // From the file itself, right before writing: never the catalogue's idea of it.
        let before = self.read(path)?;
        if let Some(tag) = location_tag(&before) {
            return Ok(WriteOutcome::Refused(format!("has_gps: {tag}")));
        }
        let tmp = Temp::beside(path);
        std::fs::copy(path, &tmp.path)?;
        let mut args: Vec<OsString> = vec![
            format!("-GPSLatitude={}", lat.abs()).into(),
            format!("-GPSLatitudeRef={}", if lat < 0.0 { "S" } else { "N" }).into(),
            format!("-GPSLongitude={}", lon.abs()).into(),
            format!("-GPSLongitudeRef={}", if lon < 0.0 { "W" } else { "E" }).into(),
            format!("-GPSProcessingMethod={MARKER_PREFIX}{marker}").into(),
        ];
        args.extend(MANDATORY_IFD0.iter().filter(|t| !before.contains_key(&format!("IFD0:{t}"))).map(|t| OsString::from(format!("-IFD0:{t}="))));
        if let Err(why) = self.edit(args, &tmp.path)? {
            return Ok(WriteOutcome::Refused(format!("write: {why}")));
        }
        #[cfg(test)]
        if let Some(tamper) = self.tamper {
            tamper(&tmp.path);
        }
        let after = self.read(&tmp.path)?;
        let marker_value = format!("{MARKER_PREFIX}{marker}");
        let verified = (|| {
            let (got_lat, got_lon) = (coordinate(&after, "GPSLatitude", "S"), coordinate(&after, "GPSLongitude", "W"));
            if !got_lat.is_some_and(|v| (v - lat).abs() <= TOLERANCE) || !got_lon.is_some_and(|v| (v - lon).abs() <= TOLERANCE) {
                return Err(format!("GPS read back as {got_lat:?}, {got_lon:?}"));
            }
            if after.get("GPS:GPSProcessingMethod").and_then(Value::as_str) != Some(marker_value.as_str()) {
                return Err("the marker did not read back".into());
            }
            let added = |k: &str| {
                let (group, name) = group_and_name(k);
                (group == "GPS" && (name == "GPSVersionID" || WRITTEN.contains(&name))) || (group == "Composite" && name.starts_with("GPS")) || k == "File:ExifByteOrder"
            };
            same_tags(&before, &after, path, &tmp.path, &added, &|_| false)?;
            self.decodes(path, &tmp.path)
        })();
        if let Err(why) = verified {
            tracing::info!(file = %path.display(), "GPS not written: {why}");
            return Ok(WriteOutcome::Refused(format!("verify: {why}")));
        }
        let file_hash = sync_and_hash(&tmp.path)?;
        std::fs::rename(&tmp.path, path)?;
        let mut tmp = tmp;
        tmp.armed = false;
        Ok(WriteOutcome::Written { file_hash })
    }

    /// Removes the GPS a [`write_gps`](Self::write_gps) put into `path`, when the file is still the
    /// bytes it wrote (`expected_hash`).
    pub fn remove_gps(&mut self, path: &Path, expected_hash: &str) -> AppResult<UndoOutcome> {
        if !path.is_file() {
            return Ok(UndoOutcome::Skipped("unreachable".into()));
        }
        match photos_import::content_hash(path) {
            Ok(h) if h == expected_hash => {}
            Ok(_) => return Ok(UndoOutcome::Skipped("changed".into())),
            Err(_) => return Ok(UndoOutcome::Skipped("unreachable".into())),
        }
        let before = self.read(path)?;
        if !before.get("GPS:GPSProcessingMethod").and_then(Value::as_str).is_some_and(|m| m.starts_with(MARKER_PREFIX)) {
            return Ok(UndoOutcome::Skipped("not_estimated".into()));
        }
        let tmp = Temp::beside(path);
        std::fs::copy(path, &tmp.path)?;
        let args = WRITTEN.iter().map(|t| OsString::from(format!("-GPS:{t}="))).collect();
        if let Err(why) = self.edit(args, &tmp.path)? {
            return Ok(UndoOutcome::Skipped(format!("verify: {why}")));
        }
        let after = self.read(&tmp.path)?;
        let verified = (|| {
            if let Some(tag) = location_tag(&after) {
                return Err(format!("{tag} is still there"));
            }
            let removed = |k: &str| {
                let (group, name) = group_and_name(k);
                (group == "GPS" && (name == "GPSVersionID" || WRITTEN.contains(&name))) || (group == "Composite" && name.starts_with("GPS"))
            };
            same_tags(&before, &after, path, &tmp.path, &|_| false, &removed)?;
            self.decodes(path, &tmp.path)
        })();
        if let Err(why) = verified {
            tracing::info!(file = %path.display(), "GPS not removed: {why}");
            return Ok(UndoOutcome::Skipped(format!("verify: {why}")));
        }
        let file_hash = sync_and_hash(&tmp.path)?;
        std::fs::rename(&tmp.path, path)?;
        let mut tmp = tmp;
        tmp.armed = false;
        Ok(UndoOutcome::Removed { file_hash })
    }
}

/// Records a write (or an undo) in the catalogue: `file_hash` is kept only while the bytes differ
/// from the import's `content_hash`.
pub fn mark_file_written(lib: &photos_core::Library, asset_id: i64, file_hash: &str) -> Result<(), photos_core::CoreError> {
    lib.db.with_conn(|c| {
        c.execute(
            "UPDATE assets SET file_hash = CASE WHEN content_hash = ?2 THEN NULL ELSE ?2 END, metadata_written_at = ?3 WHERE id = ?1",
            (asset_id, file_hash, photos_core::now_unix()),
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn exiftool() -> Option<ExifTool> {
        let e = ExifTool::locate();
        if e.is_none() {
            eprintln!("ExifTool not found: skipped (scripts/fetch-exiftool.*)");
        }
        e
    }

    fn run(et: &ExifTool, args: &[&str], path: &Path) {
        let mut s = et.session().unwrap();
        let mut a: Vec<OsString> = args.iter().map(OsString::from).collect();
        a.push(path.as_os_str().to_owned());
        let out = s.run(a).unwrap();
        assert!(out.errors().is_empty(), "{out:?}");
    }

    fn tags(et: &ExifTool, path: &Path) -> Map<String, Value> {
        et.session().unwrap().read_json(path, &[]).unwrap().as_object().cloned().unwrap()
    }

    fn gradient(path: &Path) {
        ImageBuffer::from_fn(64, 48, |x, y| Rgb([(x * 4) as u8, (y * 5) as u8, 90])).save(path).unwrap();
    }

    /// A camera-like JPEG: EXIF with make, model, date and a thumbnail.
    fn camera_jpeg(et: &ExifTool, dir: &Path) -> PathBuf {
        let thumb = dir.join("thumb.jpg");
        ImageBuffer::from_fn(16, 12, |x, _| Rgb([x as u8 * 9, 40, 40])).save(&thumb).unwrap();
        let p = dir.join("DSC_0001.jpg");
        gradient(&p);
        let thumb_arg = format!("-ThumbnailImage<={}", thumb.display());
        run(et, &["-overwrite_original", "-Make=NIKON CORPORATION", "-Model=NIKON D850", "-DateTimeOriginal=2024:03:12 14:02:00", "-Artist=Julian", &thumb_arg], &p);
        std::fs::remove_file(thumb).unwrap();
        p
    }

    fn writer(et: &ExifTool) -> GpsWriter {
        GpsWriter::new(et, None).unwrap()
    }

    fn written_hash(o: WriteOutcome) -> String {
        match o {
            WriteOutcome::Written { file_hash } => file_hash,
            other => panic!("not written: {other:?}"),
        }
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// Volatile and GPS keys out: what must not change.
    fn rest(t: &Map<String, Value>) -> Vec<(String, Value)> {
        let mut v: Vec<(String, Value)> = t
            .iter()
            .filter(|(k, _)| {
                let (group, name) = k.split_once(':').unwrap_or(("", k));
                !(k.as_str() == "SourceFile" || group == "System" || group == "ExifTool" || group == "GPS" || name.starts_with("GPS") || name.ends_with("Offset") || k.as_str() == "File:ExifByteOrder")
            })
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn writes_gps_to_file_without_any() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.jpg", "b.png", "c.tif"] {
            let p = dir.path().join(name);
            gradient(&p);
            let hash = written_hash(writer(&et).write_gps(&p, -33.8567844, 151.2152967, "jl.wayfinder").unwrap());
            assert_eq!(hash, photos_import::content_hash(&p).unwrap(), "{name}");
            let t = tags(&et, &p);
            assert!((t["Composite:GPSLatitude"].as_f64().unwrap() + 33.8567844).abs() < 1e-6, "{name}: {t:?}");
            assert!((t["Composite:GPSLongitude"].as_f64().unwrap() - 151.2152967).abs() < 1e-6, "{name}");
            assert_eq!(t["GPS:GPSProcessingMethod"], "ESTIMATED:jl.wayfinder", "{name}");
        }
        assert_eq!(files_in(dir.path()), ["a.jpg", "b.png", "c.tif"], "no temp file left");
    }

    #[test]
    fn refuses_file_with_exif_gps() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jpg");
        gradient(&p);
        run(&et, &["-overwrite_original", "-GPSLatitude=46.05", "-GPSLatitudeRef=N", "-GPSLongitude=14.5", "-GPSLongitudeRef=E"], &p);
        let before = std::fs::read(&p).unwrap();
        match writer(&et).write_gps(&p, 1.0, 2.0, "jl.wayfinder").unwrap() {
            WriteOutcome::Refused(why) => assert!(why.starts_with("has_gps"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    #[test]
    fn refuses_file_with_xmp_gps() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jpg");
        gradient(&p);
        run(&et, &["-overwrite_original", "-XMP-exif:GPSLatitude=46.05", "-XMP-exif:GPSLongitude=14.5"], &p);
        assert!(tags(&et, &p).keys().any(|k| k.starts_with("XMP-exif:GPS")), "fixture has XMP GPS");
        let before = std::fs::read(&p).unwrap();
        match writer(&et).write_gps(&p, 1.0, 2.0, "jl.wayfinder").unwrap() {
            WriteOutcome::Refused(why) => assert!(why.starts_with("has_gps"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    #[test]
    fn other_tags_unchanged() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = camera_jpeg(&et, dir.path());
        let png = dir.path().join("b.png");
        gradient(&png);
        for p in [p, png] {
            let before = tags(&et, &p);
            written_hash(writer(&et).write_gps(&p, 46.0569465, 14.5057515, "jl.wayfinder").unwrap());
            let after = tags(&et, &p);
            assert_eq!(rest(&after), rest(&before), "{}", p.display());
        }
    }

    #[test]
    fn still_decodes() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = camera_jpeg(&et, dir.path());
        written_hash(writer(&et).write_gps(&p, 10.0, 20.0, "jl.wayfinder").unwrap());
        let (img, _) = crate::derivatives::decode_original_or_reduced(&p, photos_media::MediaKind::Image, None, photos_media::HdrKind::None).unwrap();
        assert_eq!((img.width(), img.height()), (64, 48));
    }

    #[test]
    fn failed_verify_leaves_original() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = camera_jpeg(&et, dir.path());
        let before = std::fs::read(&p).unwrap();
        let mut w = writer(&et);
        w.tamper = Some(|tmp| run(&ExifTool::locate().unwrap(), &["-overwrite_original", "-Artist=Someone else"], tmp));
        match w.write_gps(&p, 10.0, 20.0, "jl.wayfinder").unwrap() {
            WriteOutcome::Refused(why) => assert!(why.starts_with("verify"), "{why}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(std::fs::read(&p).unwrap(), before, "original byte-identical");
        assert_eq!(files_in(dir.path()), ["DSC_0001.jpg"], "temp gone");
    }

    /// A NEF-shaped TIFF: a 4×4 thumbnail in IFD0, Nikon make, and a SubIFD holding the camera's
    /// JPEG (`JpgFromRaw`), the way a NEF carries its preview.
    fn tiny_nef(path: &Path, jpeg: &[u8]) {
        let mut b: Vec<u8> = Vec::new();
        b.extend(b"II");
        b.extend(42u16.to_le_bytes());
        b.extend(8u32.to_le_bytes());
        let (make, model) = (b"NIKON CORPORATION\0", b"NIKON D850\0");
        let (make_at, model_at, strip_at, sub_at) = (170u32, 188u32, 200u32, 216u32);
        let jpeg_at = sub_at + 2 + 4 * 12 + 4;
        let entry = |b: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: u32| {
            b.extend(tag.to_le_bytes());
            b.extend(typ.to_le_bytes());
            b.extend(count.to_le_bytes());
            b.extend(value.to_le_bytes());
        };
        let (short, long, ascii) = (3u16, 4u16, 2u16);
        b.extend(13u16.to_le_bytes());
        entry(&mut b, 0x00FE, long, 1, 1);
        entry(&mut b, 0x0100, short, 1, 4);
        entry(&mut b, 0x0101, short, 1, 4);
        entry(&mut b, 0x0102, short, 1, 8);
        entry(&mut b, 0x0103, short, 1, 1);
        entry(&mut b, 0x0106, short, 1, 1);
        entry(&mut b, 0x010F, ascii, make.len() as u32, make_at);
        entry(&mut b, 0x0110, ascii, model.len() as u32, model_at);
        entry(&mut b, 0x0111, long, 1, strip_at);
        entry(&mut b, 0x0115, short, 1, 1);
        entry(&mut b, 0x0116, short, 1, 4);
        entry(&mut b, 0x0117, long, 1, 16);
        entry(&mut b, 0x014A, long, 1, sub_at);
        b.extend(0u32.to_le_bytes());
        assert_eq!(b.len() as u32, make_at);
        b.extend(make);
        b.extend(model);
        b.resize(strip_at as usize, 0);
        b.extend((0u8..16).map(|i| i * 15));
        assert_eq!(b.len() as u32, sub_at);
        b.extend(4u16.to_le_bytes());
        entry(&mut b, 0x00FE, long, 1, 0);
        entry(&mut b, 0x0103, short, 1, 6);
        entry(&mut b, 0x0201, long, 1, jpeg_at);
        entry(&mut b, 0x0202, long, 1, jpeg.len() as u32);
        b.extend(0u32.to_le_bytes());
        assert_eq!(b.len() as u32, jpeg_at);
        b.extend(jpeg);
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn writes_nef_preview_intact() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let preview = dir.path().join("preview.jpg");
        gradient(&preview);
        let jpeg = std::fs::read(&preview).unwrap();
        std::fs::remove_file(&preview).unwrap();
        let p = dir.path().join("DSC_0002.NEF");
        tiny_nef(&p, &jpeg);
        assert_eq!(tags(&et, &p)["File:FileType"], "NEF", "fixture reads as a NEF");
        written_hash(writer(&et).write_gps(&p, 46.0, 14.5, "jl.wayfinder").unwrap());
        let after = std::fs::read(&p).unwrap();
        assert!(after.windows(jpeg.len()).any(|w| w == jpeg.as_slice()), "embedded preview byte-identical");
        let t = tags(&et, &p);
        assert!((t["Composite:GPSLatitude"].as_f64().unwrap() - 46.0).abs() < 1e-6);
        assert_eq!(t["IFD0:Make"], "NIKON CORPORATION");
    }

    #[test]
    fn unsupported_format_db_only() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        for name in ["IMG_0001.CR3", "IMG_0002.MOV"] {
            let p = dir.path().join(name);
            std::fs::write(&p, b"not touched").unwrap();
            assert_eq!(writer(&et).write_gps(&p, 1.0, 2.0, "jl.wayfinder").unwrap(), WriteOutcome::DbOnly);
            assert_eq!(std::fs::read(&p).unwrap(), b"not touched");
        }
    }

    #[test]
    fn undo_removes_written_gps() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = camera_jpeg(&et, dir.path());
        let before = tags(&et, &p);
        let mut w = writer(&et);
        let hash = written_hash(w.write_gps(&p, 46.0569465, 14.5057515, "jl.wayfinder").unwrap());
        let undone = match w.remove_gps(&p, &hash).unwrap() {
            UndoOutcome::Removed { file_hash } => file_hash,
            other => panic!("{other:?}"),
        };
        assert_eq!(undone, photos_import::content_hash(&p).unwrap());
        let after = tags(&et, &p);
        assert!(!after.keys().any(|k| k.contains("GPSLatitude") || k.contains("GPSLongitude") || k.contains("GPSProcessingMethod")), "{after:?}");
        assert_eq!(rest(&after), rest(&before));
        assert_eq!(files_in(dir.path()), ["DSC_0001.jpg"]);
    }

    #[test]
    fn undo_skips_changed_file() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = camera_jpeg(&et, dir.path());
        let mut w = writer(&et);
        let hash = written_hash(w.write_gps(&p, 10.0, 20.0, "jl.wayfinder").unwrap());
        run(&et, &["-overwrite_original", "-Rating=5"], &p);
        let changed = std::fs::read(&p).unwrap();
        assert_eq!(w.remove_gps(&p, &hash).unwrap(), UndoOutcome::Skipped("changed".into()));
        assert_eq!(std::fs::read(&p).unwrap(), changed);
        assert_eq!(w.remove_gps(&dir.path().join("gone.jpg"), &hash).unwrap(), UndoOutcome::Skipped("unreachable".into()));
    }

    #[test]
    fn undo_leaves_camera_gps() {
        let Some(et) = exiftool() else { return };
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jpg");
        gradient(&p);
        run(&et, &["-overwrite_original", "-GPSLatitude=46.05", "-GPSLatitudeRef=N", "-GPSLongitude=14.5", "-GPSLongitudeRef=E"], &p);
        let hash = photos_import::content_hash(&p).unwrap();
        assert_eq!(writer(&et).remove_gps(&p, &hash).unwrap(), UndoOutcome::Skipped("not_estimated".into()));
        assert_eq!(photos_import::content_hash(&p).unwrap(), hash);
    }

    // -- Step 4: a written library file is healthy, the camera's file is still a duplicate -------

    fn library_with_one_photo(et: &ExifTool, tmp: &Path) -> (photos_core::Library, PathBuf, i64, PathBuf) {
        let src = tmp.join("card");
        std::fs::create_dir_all(&src).unwrap();
        camera_jpeg(et, &src);
        let lib = photos_core::Library::create(&tmp.join("lib")).unwrap();
        import(&lib, &src);
        let (id, rel): (i64, String) = lib.db.with_conn(|c| Ok(c.query_row("SELECT id, relative_path FROM assets", [], |r| Ok((r.get(0)?, r.get(1)?)))?)).unwrap();
        let original = lib.originals_dir().join(rel);
        (lib, src, id, original)
    }

    fn import(lib: &photos_core::Library, src: &Path) -> photos_import::ImportSummary {
        photos_import::run_import(lib, &photos_import::FolderSource::new(src), &photos_import::ImportOptions::default(), &photos_jobs::CancelToken::new(), &mut |_| {}, &mut |_| {}).unwrap()
    }

    #[test]
    fn check_library_accepts_gps_written_file() {
        let Some(et) = exiftool() else { return };
        let tmp = tempfile::tempdir().unwrap();
        let (lib, src, id, original) = library_with_one_photo(&et, tmp.path());
        let hash = written_hash(writer(&et).write_gps(&original, 46.0, 14.5, "jl.wayfinder").unwrap());
        let quiet = |_: photos_import::check::CheckProgress| {};
        let unmarked = photos_import::check::check_library(&lib, &photos_jobs::CancelToken::new(), &quiet).unwrap();
        assert_eq!(unmarked.problems.len(), 1, "a changed file without file_hash is still caught");
        mark_file_written(&lib, id, &hash).unwrap();
        let r = photos_import::check::check_library(&lib, &photos_jobs::CancelToken::new(), &quiet).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        // The camera's file on the card is still judged against the library's (written) copy.
        let f = photos_import::check::check_folder(&lib, &src, &photos_jobs::CancelToken::new(), &|_| {}).unwrap();
        assert_eq!(f.safe, 1, "{:?}", f.items);
    }

    #[test]
    fn reimport_of_camera_file_skipped() {
        let Some(et) = exiftool() else { return };
        let tmp = tempfile::tempdir().unwrap();
        let (lib, src, id, original) = library_with_one_photo(&et, tmp.path());
        let hash = written_hash(writer(&et).write_gps(&original, 46.0, 14.5, "jl.wayfinder").unwrap());
        mark_file_written(&lib, id, &hash).unwrap();
        let again = import(&lib, &src);
        assert_eq!((again.imported, again.skipped), (0, 1), "{again:?}");
    }
}
