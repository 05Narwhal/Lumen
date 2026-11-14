//! Carry an original's metadata into an export, correct what the new pixels changed, and
//! prove it by reading both back (Phase 8 spec §5).
//!
//! One ExifTool command does the carrying: `-TagsFromFile <original> -all:all` brings EXIF
//! (maker notes included), XMP, IPTC, GPS and QuickTime keys; Lumen's own IPTC → XMP table
//! comes first for formats that have no IPTC, so a real XMP value still wins; then the
//! corrections and Lumen's facts. The original is only ever the `-TagsFromFile` source;
//! `-overwrite_original` touches nothing but the export being written.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::Path;

use photos_core::export::{ExportMeta, LostTag};
use serde_json::Value;

use crate::exiftool::Session;
use crate::MediaError;

/// What is being written, for the choices that differ by container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Jpeg,
    Png,
    Webp,
    Avif,
    Heic,
    Tiff,
    Video,
}

impl Target {
    /// Formats with an IPTC-IIM block. Everything else gets IPTC as XMP.
    fn has_iptc(self) -> bool {
        matches!(self, Target::Jpeg)
    }
    /// Formats whose colour Lumen declares with an ICC profile of its own.
    fn has_our_icc(self) -> bool {
        matches!(self, Target::Jpeg | Target::Png | Target::Webp | Target::Heic)
    }
    fn is_image(self) -> bool {
        self != Target::Video
    }
}

/// IPTC-IIM → XMP, for targets without IPTC: the mapping the IPTC Photo Metadata standard
/// defines between the two (the same one ExifTool's `iptc2xmp.args` encodes).
pub const IPTC_TO_XMP: &[(&str, &str)] = &[
    ("XMP-dc:Subject", "IPTC:Keywords"),
    ("XMP-dc:Title", "IPTC:ObjectName"),
    ("XMP-dc:Description", "IPTC:Caption-Abstract"),
    ("XMP-dc:Creator", "IPTC:By-line"),
    ("XMP-dc:Rights", "IPTC:CopyrightNotice"),
    ("XMP-photoshop:AuthorsPosition", "IPTC:By-lineTitle"),
    ("XMP-photoshop:CaptionWriter", "IPTC:Writer-Editor"),
    ("XMP-photoshop:Category", "IPTC:Category"),
    ("XMP-photoshop:City", "IPTC:City"),
    ("XMP-photoshop:Country", "IPTC:Country-PrimaryLocationName"),
    ("XMP-photoshop:Credit", "IPTC:Credit"),
    ("XMP-photoshop:DateCreated", "IPTC:DateCreated"),
    ("XMP-photoshop:Headline", "IPTC:Headline"),
    ("XMP-photoshop:Instructions", "IPTC:SpecialInstructions"),
    ("XMP-photoshop:Source", "IPTC:Source"),
    ("XMP-photoshop:State", "IPTC:Province-State"),
    ("XMP-photoshop:SupplementalCategories", "IPTC:SupplementalCategories"),
    ("XMP-photoshop:TransmissionReference", "IPTC:OriginalTransmissionReference"),
    ("XMP-photoshop:Urgency", "IPTC:Urgency"),
    ("XMP-iptcCore:CountryCode", "IPTC:Country-PrimaryLocationCode"),
    ("XMP-iptcCore:Location", "IPTC:Sub-location"),
];

/// The colour labels as Lightroom and Bridge name them in `xmp:Label`.
const LABELS: [&str; 5] = ["Red", "Yellow", "Green", "Blue", "Purple"];

/// Tags the new pixels make wrong, so they are never copied: the embedded previews show the
/// unedited photo, the source's profile no longer describes the pixels, the orientation is
/// baked in, and the sizes are the old ones.
const NOT_COPIED: &[&str] = &[
    "--ICC_Profile:all",
    "--ThumbnailImage",
    "--PreviewImage",
    "--JpgFromRaw",
    "--OtherImage",
    "--Orientation",
    "--XMP-tiff:ImageWidth",
    "--XMP-tiff:ImageLength",
    "--XMP-exif:PixelXDimension",
    "--XMP-exif:PixelYDimension",
];

/// The ExifTool arguments for one transfer, the output path last.
pub fn transfer_args(original: &Path, output: &Path, target: Target, meta: &ExportMeta, size: Option<(u32, u32)>) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["-overwrite_original".into(), "-m".into(), "-TagsFromFile".into(), original.as_os_str().to_owned()];
    // Before -all:all, so a value the original already has in XMP wins over the IPTC copy.
    if !target.has_iptc() {
        a.extend(IPTC_TO_XMP.iter().map(|(xmp, iptc)| OsString::from(format!("-{xmp}<{iptc}"))));
    }
    a.push("-all:all".into());
    a.extend(NOT_COPIED.iter().map(OsString::from));
    if target.is_image() {
        a.push("-IFD0:Orientation#=1".into());
        if let Some((w, h)) = size {
            a.push(format!("-ExifIFD:ExifImageWidth={w}").into());
            a.push(format!("-ExifIFD:ExifImageHeight={h}").into());
        }
        a.push("-IFD0:Software=Lumen".into());
        a.push("-IFD0:ModifyDate=now".into());
    }
    a.push("-XMP-xmp:CreatorTool=Lumen".into());
    a.push("-XMP-xmp:ModifyDate=now".into());
    a.push("-XMP-xmp:MetadataDate=now".into());
    // Lumen's own facts. Keywords are added (−= then += keeps them unique), not replaced.
    if meta.rating > 0 {
        a.push(format!("-XMP-xmp:Rating={}", meta.rating.min(5)).into());
    }
    if let Some(label) = LABELS.get((meta.color_label - 1).max(0) as usize).filter(|_| meta.color_label > 0) {
        a.push(format!("-XMP-xmp:Label={label}").into());
    }
    if let Some(t) = meta.title.as_deref().filter(|t| !t.trim().is_empty()) {
        a.push(format!("-XMP-dc:Title={t}").into());
    }
    if let Some(c) = meta.caption.as_deref().filter(|c| !c.trim().is_empty()) {
        a.push(format!("-XMP-dc:Description={c}").into());
    }
    for k in meta.keywords.iter().filter(|k| !k.trim().is_empty()) {
        a.push(format!("-XMP-dc:Subject-={k}").into());
        a.push(format!("-XMP-dc:Subject+={k}").into());
    }
    a.push(output.as_os_str().to_owned());
    a
}

/// Copy, correct and add. Errors when ExifTool reports one; warnings are normal (a source
/// with a tag the target cannot hold) and are left to the verification to judge.
pub fn transfer(session: &mut Session, original: &Path, output: &Path, target: Target, meta: &ExportMeta, size: Option<(u32, u32)>) -> Result<(), MediaError> {
    let out = session.run(transfer_args(original, output, target, meta, size))?;
    let errors = out.errors();
    if !errors.is_empty() {
        return Err(MediaError::Decode(format!("exiftool: {}", errors.join("; "))));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Verification

/// The facts that must survive, each read from whichever group holds it.
const CHECKED: &[&str] = &[
    "DateTimeOriginal",
    "OffsetTimeOriginal",
    "SubSecTimeOriginal",
    "CreateDate",
    "GPSLatitude",
    "GPSLongitude",
    "GPSAltitude",
    "Make",
    "Model",
    "SerialNumber",
    "LensModel",
    "ExposureTime",
    "FNumber",
    "ISO",
    "FocalLength",
    "Artist",
    "Creator",
    "Copyright",
    "Rights",
];

/// What to read from each file: the checked facts, the lists and texts that live under two
/// names, and the corrected tags.
fn read_tags() -> Vec<&'static str> {
    let mut t = CHECKED.to_vec();
    t.extend(["Keywords", "Subject", "Title", "ObjectName", "Description", "Caption-Abstract", "Rating", "Label", "Orientation", "ExifImageWidth", "ExifImageHeight", "ProfileDescription"]);
    t
}

/// A tag's value from `-j -n -G1` output, whatever group it is in. GPS prefers the signed
/// `Composite` form; otherwise the first group that has it.
fn value<'a>(v: &'a Value, tag: &str) -> Option<&'a Value> {
    let obj = v.as_object()?;
    if let Some(c) = obj.get(&format!("Composite:{tag}")) {
        return Some(c);
    }
    obj.iter().find(|(k, _)| k.rsplit(':').next() == Some(tag)).map(|(_, v)| v)
}

fn words(v: Option<&Value>) -> BTreeSet<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(plain).filter(|s| !s.is_empty()).collect(),
        Some(x) => {
            let s = plain(x);
            if s.is_empty() {
                BTreeSet::new()
            } else {
                [s].into()
            }
        }
        None => BTreeSet::new(),
    }
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Equal as numbers (to a millionth, relative) when both are numbers, else as trimmed text.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1.0),
        _ => plain(a) == plain(b),
    }
}

fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// What the export should say, besides what the original said.
#[derive(Debug, Clone, Default)]
pub struct Expect {
    pub size: Option<(u32, u32)>,
    pub target: Option<Target>,
    pub meta: ExportMeta,
}

/// Compare `before` (the original) with `after` (the export), both `-j -n -G1` objects.
pub fn compare(before: &Value, after: &Value, expect: &Expect) -> Vec<LostTag> {
    let mut lost = Vec::new();
    let mut miss = |tag: &str, b: String, a: String| lost.push(LostTag { tag: tag.into(), before: b, after: a });
    for tag in CHECKED {
        let Some(b) = value(before, tag).filter(|v| !plain(v).is_empty()) else { continue };
        match value(after, tag) {
            Some(a) if same(b, a) => {}
            a => miss(tag, plain(b), a.map(plain).unwrap_or_default()),
        }
    }
    // Lists and texts that IPTC and XMP both hold count once, whichever carries them.
    let pair = |v: &Value, x: &str, y: &str| {
        let mut s = words(value(v, x));
        s.extend(words(value(v, y)));
        s
    };
    let mut want_keywords = pair(before, "Keywords", "Subject");
    want_keywords.extend(expect.meta.keywords.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()));
    let have_keywords = pair(after, "Keywords", "Subject");
    if !want_keywords.is_subset(&have_keywords) {
        miss("Keywords", join(&want_keywords), join(&have_keywords));
    }
    for (name, x, y, ours) in [("Title", "Title", "ObjectName", &expect.meta.title), ("Caption", "Description", "Caption-Abstract", &expect.meta.caption)] {
        let want = match ours.as_deref().filter(|t| !t.trim().is_empty()) {
            Some(t) => [t.trim().to_string()].into(),
            None => pair(before, x, y),
        };
        let have = pair(after, x, y);
        if !want.is_empty() && !want.is_subset(&have) {
            miss(name, join(&want), join(&have));
        }
    }
    // Lumen's facts.
    if expect.meta.rating > 0 && value(after, "Rating").and_then(Value::as_i64) != Some(expect.meta.rating.min(5)) {
        miss("Rating", expect.meta.rating.to_string(), value(after, "Rating").map(plain).unwrap_or_default());
    }
    if expect.meta.color_label > 0 {
        let want = LABELS[(expect.meta.color_label.clamp(1, 5) - 1) as usize];
        if value(after, "Label").map(plain).as_deref() != Some(want) {
            miss("Label", want.into(), value(after, "Label").map(plain).unwrap_or_default());
        }
    }
    // The corrections.
    let image = expect.target.is_none_or(Target::is_image);
    if image {
        if let Some(o) = value(after, "Orientation").and_then(Value::as_i64).filter(|o| *o != 1) {
            miss("Orientation", "1".into(), o.to_string());
        }
        if let Some((w, h)) = expect.size {
            for (tag, want) in [("ExifImageWidth", w), ("ExifImageHeight", h)] {
                if value(after, tag).and_then(Value::as_u64) != Some(want as u64) {
                    miss(tag, want.to_string(), value(after, tag).map(plain).unwrap_or_default());
                }
            }
        }
    }
    if expect.target.is_some_and(Target::has_our_icc) && value(after, "ProfileDescription").map(plain).as_deref() != Some(crate::icc::SRGB_DESCRIPTION) {
        miss("ProfileDescription", crate::icc::SRGB_DESCRIPTION.into(), value(after, "ProfileDescription").map(plain).unwrap_or_default());
    }
    lost
}

/// Read both files and compare.
pub fn verify(session: &mut Session, original: &Path, output: &Path, expect: &Expect) -> Result<Vec<LostTag>, MediaError> {
    let tags = read_tags();
    let before = session.read_json(original, &tags)?;
    let after = session.read_json(output, &tags)?;
    Ok(compare(&before, &after, expect))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn args_text(a: &[OsString]) -> Vec<String> {
        a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn the_original_is_only_ever_the_source_and_the_output_comes_last() {
        let a = args_text(&transfer_args(&PathBuf::from("orig.NEF"), &PathBuf::from("out.part"), Target::Jpeg, &ExportMeta::default(), Some((10, 20))));
        assert_eq!(a[..4], ["-overwrite_original", "-m", "-TagsFromFile", "orig.NEF"]);
        assert_eq!(a.last().unwrap(), "out.part");
        assert_eq!(a.iter().filter(|s| s.as_str() == "orig.NEF").count(), 1);
        assert!(a.contains(&"-ExifIFD:ExifImageWidth=10".into()) && a.contains(&"-IFD0:Orientation#=1".into()));
        assert!(a.contains(&"--ICC_Profile:all".into()) && a.contains(&"--ThumbnailImage".into()));
        assert!(!a.iter().any(|s| s.contains("<IPTC:")), "JPEG keeps IPTC as IPTC");
    }

    #[test]
    fn formats_without_iptc_get_it_as_xmp_before_the_copy() {
        let a = args_text(&transfer_args(&PathBuf::from("o.jpg"), &PathBuf::from("x.part"), Target::Webp, &ExportMeta::default(), None));
        let map = a.iter().position(|s| s == "-XMP-dc:Subject<IPTC:Keywords").unwrap();
        let all = a.iter().position(|s| s == "-all:all").unwrap();
        assert!(map < all, "a real XMP value copied by -all:all must win");
    }

    #[test]
    fn lumens_facts_are_added_and_keywords_kept_unique() {
        let meta = ExportMeta { rating: 4, color_label: 3, title: Some("Harbour".into()), caption: None, keywords: vec!["boat".into(), " ".into()] };
        let a = args_text(&transfer_args(&PathBuf::from("o.jpg"), &PathBuf::from("x"), Target::Heic, &meta, None));
        for want in ["-XMP-xmp:Rating=4", "-XMP-xmp:Label=Green", "-XMP-dc:Title=Harbour", "-XMP-dc:Subject-=boat", "-XMP-dc:Subject+=boat"] {
            assert!(a.contains(&want.into()), "{want}");
        }
        assert!(!a.iter().any(|s| s.ends_with("Subject+= ")), "blank keywords are skipped");
        let video = args_text(&transfer_args(&PathBuf::from("o.mov"), &PathBuf::from("x"), Target::Video, &ExportMeta::default(), None));
        assert!(!video.iter().any(|s| s.contains("Orientation#=")), "a video's orientation is its own matter");
    }

    fn orig() -> Value {
        json!({
            "ExifIFD:DateTimeOriginal": "2024:05:01 10:00:00",
            "ExifIFD:OffsetTimeOriginal": "+02:00",
            "Composite:GPSLatitude": 46.0512,
            "GPS:GPSLatitude": 46.0512,
            "IFD0:Make": "NIKON CORPORATION",
            "ExifIFD:FNumber": 2.8,
            "IPTC:Keywords": ["beach", "Anna"],
            "IPTC:ObjectName": "Harbour",
            "IFD0:Orientation": 6
        })
    }

    #[test]
    fn a_faithful_export_loses_nothing_even_across_families() {
        let after = json!({
            "ExifIFD:DateTimeOriginal": "2024:05:01 10:00:00",
            "ExifIFD:OffsetTimeOriginal": "+02:00",
            "Composite:GPSLatitude": 46.0512000001,
            "IFD0:Make": "NIKON CORPORATION",
            "ExifIFD:FNumber": 2.8,
            "XMP-dc:Subject": ["Anna", "beach"],
            "XMP-dc:Title": "Harbour",
            "IFD0:Orientation": 1,
            "ExifIFD:ExifImageWidth": 3000,
            "ExifIFD:ExifImageHeight": 2000,
            "ICC_Profile:ProfileDescription": crate::icc::SRGB_DESCRIPTION
        });
        let lost = compare(&orig(), &after, &Expect { size: Some((3000, 2000)), target: Some(Target::Png), meta: ExportMeta::default() });
        assert!(lost.is_empty(), "{lost:?}");
    }

    #[test]
    fn missing_changed_and_uncorrected_tags_are_reported() {
        let after = json!({
            "ExifIFD:DateTimeOriginal": "2024:05:01 11:00:00",
            "IFD0:Make": "NIKON CORPORATION",
            "XMP-dc:Subject": ["beach"],
            "IFD0:Orientation": 6,
            "ExifIFD:ExifImageWidth": 6000
        });
        let expect = Expect { size: Some((3000, 2000)), target: Some(Target::Jpeg), meta: ExportMeta { rating: 3, ..Default::default() } };
        let lost: Vec<String> = compare(&orig(), &after, &expect).into_iter().map(|l| l.tag).collect();
        for tag in ["DateTimeOriginal", "OffsetTimeOriginal", "GPSLatitude", "FNumber", "Keywords", "Title", "Rating", "Orientation", "ExifImageWidth", "ExifImageHeight", "ProfileDescription"] {
            assert!(lost.contains(&tag.to_string()), "{tag} in {lost:?}");
        }
        assert!(!lost.contains(&"Make".to_string()));
    }

    // -----------------------------------------------------------------------------------
    // Round trips through the real ExifTool (and FFmpeg for AVIF/HEIC).

    fn session() -> Session {
        crate::ExifTool::locate().expect("ExifTool: run scripts/fetch-exiftool").session().unwrap()
    }

    /// A photo carrying EXIF (camera, exposure, dates with offset and sub-seconds, a turned
    /// orientation), GPS, IPTC and XMP — what a camera plus a Lightroom session leave.
    fn fixture(s: &mut Session, dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(640, 480, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])));
        img.save(&path).unwrap();
        let args = [
            "-overwrite_original",
            "-IFD0:Make=NIKON CORPORATION",
            "-IFD0:Model=NIKON Z 6_2",
            "-IFD0:Orientation#=6",
            "-IFD0:Artist=Julian",
            "-IFD0:Copyright=(c) 2024 Julian",
            "-ExifIFD:SerialNumber=6012345",
            "-ExifIFD:LensModel=NIKKOR Z 24-70mm f/4 S",
            "-ExifIFD:ExposureTime=1/250",
            "-ExifIFD:FNumber=4",
            "-ExifIFD:ISO=200",
            "-ExifIFD:FocalLength=35",
            "-ExifIFD:DateTimeOriginal=2024:05:01 10:00:00",
            "-ExifIFD:CreateDate=2024:05:01 10:00:00",
            "-ExifIFD:OffsetTimeOriginal=+02:00",
            "-ExifIFD:SubSecTimeOriginal=42",
            "-GPSLatitude=46.0512",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=14.5058",
            "-GPSLongitudeRef=E",
            "-GPSAltitude=295",
            "-GPSAltitudeRef=0",
            "-XMP-xmp:Rating=2",
        ];
        let mut all: Vec<OsString> = args.iter().map(OsString::from).collect();
        if name.ends_with(".jpg") {
            // A JPEG carries IPTC; the other fixtures hold the same facts in XMP.
            all.extend(["-IPTC:Keywords=beach", "-IPTC:Keywords=Anna", "-IPTC:ObjectName=Harbour", "-IPTC:Caption-Abstract=Morning at the harbour", "-IPTC:City=Piran"].map(OsString::from));
        } else {
            all.extend(["-XMP-dc:Subject=beach", "-XMP-dc:Subject=Anna", "-XMP-dc:Title=Harbour", "-XMP-dc:Description=Morning at the harbour"].map(OsString::from));
        }
        all.push(path.clone().into_os_string());
        let out = s.run(all).unwrap();
        assert!(out.errors().is_empty(), "{out:?}");
        path
    }

    fn stamp(p: &Path) -> (Vec<u8>, std::time::SystemTime) {
        (std::fs::read(p).unwrap(), std::fs::metadata(p).unwrap().modified().unwrap())
    }

    /// Phase 8e: a photo with masks (and a turn and a crop) exports in every format with the
    /// pixels its preview showed (≥ 32 dB, as 8c's preview-vs-export rule) and its metadata
    /// carried and verified.
    #[test]
    #[ignore = "needs ExifTool, and FFmpeg for AVIF and HEIC"]
    fn every_format_carries_a_masked_photo() {
        use crate::develop::{process, Ctx, Linear};
        use crate::encode::{encode_file, EncodeOptions, Format};
        use photos_core::edits::{Basic, Crop, Develop, LocalAdjust, Mask, MaskShape, Recipe};
        let ff = crate::Ffmpeg::locate();
        let dir = tempfile::tempdir().unwrap();
        let mut s = session();
        let meta = ExportMeta { rating: 3, color_label: 0, title: None, caption: None, keywords: vec!["Masks".into()] };
        let original = fixture(&mut s, dir.path(), "camera.jpg");
        // A smooth scene, so the lossy formats are judged on the develop, not on noise.
        let (w, h) = (1200u32, 800u32);
        let mut full = Linear::new(w, h, [0.0; 3]);
        for (i, p) in full.data.iter_mut().enumerate() {
            let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
            let l = 0.18 * 2f32.powf(-1.5 + 2.5 * x + (y * 6.0).sin() * 0.4);
            *p = [l * (0.8 + 0.4 * y), l, l * (1.2 - 0.4 * x)];
        }
        let mask = |id, shapes, adjust| Mask { id, name: format!("Mask {id}"), shapes, invert: false, opacity: 1.0, adjust, visible: true };
        let recipe = Recipe {
            rotate: 90,
            crop: Some(Crop { x: 0.05, y: 0.1, w: 0.9, h: 0.8 }),
            develop: Some(Develop {
                basic: Basic { contrast: 0.15, ..Default::default() },
                masks: vec![
                    mask(1, vec![MaskShape::Radial { add: true, center: [0.35, 0.5], radius: [0.2, 0.3], angle: 20.0, feather: 0.6, inside: true }], LocalAdjust { exposure: 1.0, ..Default::default() }),
                    mask(2, vec![MaskShape::Linear { add: true, from: [0.5, 0.0], to: [0.5, 0.5] }], LocalAdjust { exposure: -0.7, saturation: -0.5, ..Default::default() }),
                    mask(3, vec![MaskShape::Brush { add: true, radius: 0.05, feather: 0.6, flow: 0.8, points: vec![[2600, 1000], [3400, 2400], [3000, 3600]] }], LocalAdjust { exposure: 0.6, temperature: 30.0, ..Default::default() }),
                ],
                ..Default::default()
            }),
            ..Recipe::identity()
        };
        let exported = process(&full, &recipe, &Ctx::new(1.0, 7)).to_rgb8();
        let preview = process(&full.resize(450, 300), &recipe, &Ctx::new(450.0 / 1200.0, 7)).to_rgb8();
        let unmasked = Recipe { develop: Some(Develop { masks: vec![], ..recipe.develop.clone().unwrap() }), ..recipe.clone() };
        let plain = process(&full.resize(450, 300), &unmasked, &Ctx::new(450.0 / 1200.0, 7)).to_rgb8();
        let psnr = |a: &image::RgbImage, b: &image::RgbImage| {
            let mse = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.as_raw().len() as f64;
            10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
        };
        assert!(psnr(&preview, &plain) < 25.0, "the masks show in the preview");
        let (ew, eh) = exported.dimensions();
        for (format, target, ext) in [(Format::Jpeg, Target::Jpeg, "jpg"), (Format::Png, Target::Png, "png"), (Format::Webp, Target::Webp, "webp"), (Format::Avif, Target::Avif, "avif"), (Format::Heic, Target::Heic, "heic")] {
            if format.needs_ffmpeg() && ff.is_none() {
                eprintln!("skipping {ext}: no FFmpeg");
                continue;
            }
            let out = dir.path().join(format!("masked.{ext}"));
            encode_file(&image::DynamicImage::ImageRgb8(exported.clone()), &EncodeOptions { format, quality: 90, lossless: false }, ff.as_ref(), &out).unwrap();
            transfer(&mut s, &original, &out, target, &meta, Some((ew, eh))).unwrap();
            let lost = verify(&mut s, &original, &out, &Expect { size: Some((ew, eh)), target: Some(target), meta: meta.clone() }).unwrap();
            assert!(lost.is_empty(), "{ext}: {lost:#?}");
            let back = crate::decode_preview_hdr(&out, crate::kind_for_extension(ext), ff.as_ref(), crate::hdr::HdrKind::None).unwrap().to_rgb8();
            assert_eq!(back.dimensions(), (ew, eh), "{ext}");
            let small = image::imageops::resize(&back, preview.width(), preview.height(), image::imageops::FilterType::Triangle);
            let db = psnr(&small, &preview);
            eprintln!("{ext}: {db:.1} dB against the preview");
            assert!(db >= 32.0, "{ext}: the export is {db:.1} dB from its preview");
        }
    }

    #[test]
    #[ignore = "needs ExifTool, and FFmpeg for AVIF and HEIC"]
    fn every_format_carries_the_original_and_proves_it() {
        use crate::encode::{encode_file, EncodeOptions, Format};
        let ff = crate::Ffmpeg::locate();
        let dir = tempfile::tempdir().unwrap();
        let mut s = session();
        let meta = ExportMeta { rating: 4, color_label: 3, title: None, caption: None, keywords: vec!["Slovenia".into()] };
        for source in ["camera.jpg", "exported.png"] {
            let original = fixture(&mut s, dir.path(), source);
            let before = stamp(&original);
            for (format, target, ext) in [(Format::Jpeg, Target::Jpeg, "jpg"), (Format::Png, Target::Png, "png"), (Format::Webp, Target::Webp, "webp"), (Format::Avif, Target::Avif, "avif"), (Format::Heic, Target::Heic, "heic")] {
                if format.needs_ffmpeg() && ff.is_none() {
                    eprintln!("skipping {ext}: no FFmpeg");
                    continue;
                }
                let out = dir.path().join(format!("{source}.{ext}"));
                // The export is the photo turned upright: 480 × 640.
                let pixels = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(480, 640, image::Rgb([90, 120, 150])));
                encode_file(&pixels, &EncodeOptions { format, quality: 85, lossless: false }, ff.as_ref(), &out).unwrap();
                // Before the transfer the verification must see everything missing, or its
                // empty answer afterwards would prove nothing.
                let bare: Vec<String> = verify(&mut s, &original, &out, &Expect { size: Some((480, 640)), target: Some(target), meta: meta.clone() }).unwrap().into_iter().map(|l| l.tag).collect();
                for tag in ["DateTimeOriginal", "GPSLatitude", "Model", "LensModel", "Keywords", "Title", "Rating"] {
                    assert!(bare.contains(&tag.to_string()), "{source} → {ext}: a bare export should lack {tag}: {bare:?}");
                }
                transfer(&mut s, &original, &out, target, &meta, Some((480, 640))).unwrap();
                let lost = verify(&mut s, &original, &out, &Expect { size: Some((480, 640)), target: Some(target), meta: meta.clone() }).unwrap();
                assert!(lost.is_empty(), "{source} → {ext}: {lost:#?}");
            }
            assert_eq!(stamp(&original), before, "{source} was not touched");
        }
    }
}
