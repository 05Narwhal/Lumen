use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use exif::{In, Tag, Value};
use serde::{Deserialize, Serialize};

use crate::ffmpeg::{Ffmpeg, Probe};
use crate::kind::MediaKind;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub orientation: u16,
    /// EXIF local wall time as a unix timestamp (the spec's `captured_at`).
    pub captured_at: Option<i64>,
    /// Which tag `captured_at` came from. `None` when the file offered no date at all.
    #[serde(default)]
    pub capture_source: Option<CaptureSource>,
    /// The date strings the container held, kept so XMP can be weighed against them
    /// without reading the file twice.
    #[serde(skip)]
    pub exif_dates: RawDates,
    pub tz_offset_minutes: Option<i32>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
    pub gps_altitude: Option<f64>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub body_serial: Option<String>,
    pub lens: Option<String>,
    pub exposure: Option<String>,
    pub aperture: Option<f64>,
    pub iso: Option<u32>,
    pub focal_length_mm: Option<f64>,
    /// The app that wrote the file (EXIF `Software`, else XMP `CreatorTool`): *Exported
    /// from* in search (Round 7).
    #[serde(default)]
    pub software: Option<String>,
    /// "Auto" or "Manual".
    #[serde(default)]
    pub white_balance: Option<String>,
    /// "Fired" or "Did not fire".
    #[serde(default)]
    pub flash: Option<String>,
    /// "Pattern", "Center-weighted average", "Spot"…
    #[serde(default)]
    pub metering: Option<String>,
    /// "Aperture priority", "Manual", "Program"…
    #[serde(default)]
    pub exposure_program: Option<String>,
    /// "-0.7 EV".
    #[serde(default)]
    pub exposure_bias: Option<String>,
    /// EXIF `DateTimeDigitized` as local wall seconds, like `captured_at` (Round 9A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digitized_at: Option<i64>,
    /// EXIF `LensMake`: "Sigma", "Apple" (Round 9A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens_make: Option<String>,
    /// "sRGB", "Display P3", "Adobe RGB": the embedded ICC profile's, else EXIF `ColorSpace`
    /// (Round 9A).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_space: Option<String>,
    /// The milliseconds of `captured_at` (EXIF `SubSecTime*` of the tag the date came from),
    /// so a burst sorts within its second (Round 19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_subsec_ms: Option<u16>,
    /// EXIF `GPSImgDirection`: degrees clockwise from north the camera pointed (Round 19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gps_direction: Option<f64>,
    /// Which reading of the file this is ([`METADATA_VERSION`]); older rows lack the fields
    /// above until the maintenance pass reads them again.
    #[serde(default)]
    pub exif: u8,
    /// Video only: what ffprobe reported (None without FFmpeg).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
    /// Video only, from the probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

/// The current reading of a file's metadata (Round 7 added the software and the exposure
/// details). Stored as `metadata_json.exif`.
pub const METADATA_VERSION: u8 = 5;

/// Best-effort, without FFmpeg. Returns defaults (orientation 1) when there is no EXIF.
pub fn read_metadata(path: &Path, kind: MediaKind) -> Metadata {
    read_metadata_with(path, kind, None)
}

/// [`read_metadata`] plus, for video with FFmpeg present, the ffprobe facts: display
/// size (rotation applied), duration, creation time and the phone's make/model.
pub fn read_metadata_with(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>) -> Metadata {
    let mut m = read_exif_metadata(path, kind);
    if kind == MediaKind::Video {
        if let Some(p) = ffmpeg.and_then(|f| f.probe(path).ok()) {
            apply_probe(&mut m, p);
        }
    }
    m
}

/// Pure: merge a probe into EXIF-derived metadata. The poster frame comes out of FFmpeg
/// already rotated, so the stored orientation is 1 and the size is the display size.
pub fn apply_probe(m: &mut Metadata, p: Probe) {
    let (w, h) = p.display_size();
    m.width = w.or(m.width);
    m.height = h.or(m.height);
    m.orientation = 1;
    m.duration_ms = p.duration_ms;
    if m.captured_at.is_none() {
        m.captured_at = p.creation_local.or(p.creation_time);
        m.tz_offset_minutes = m.tz_offset_minutes.or(p.tz_offset_minutes);
    }
    if m.make.is_none() {
        m.make = p.make.clone();
    }
    if m.model.is_none() {
        m.model = p.model.clone();
    }
    if m.gps_lat.is_none() || m.gps_lon.is_none() {
        (m.gps_lat, m.gps_lon) = (p.gps_lat, p.gps_lon);
        m.gps_altitude = m.gps_altitude.or(p.gps_altitude);
    }
    m.probe = Some(p);
}

fn read_exif_metadata(path: &Path, kind: MediaKind) -> Metadata {
    let mut m = Metadata { orientation: 1, exif: METADATA_VERSION, ..Default::default() };
    if let Ok(f) = File::open(path) {
        if let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(f)) {
            fill_from_exif(&mut m, &exif);
        }
    }
    // The profile names the colour space better than EXIF can (an iPhone's Display P3 is
    // "uncalibrated" there). It sits in the file's first bytes in every format that has one.
    if matches!(kind, MediaKind::Image | MediaKind::Raw | MediaKind::Heic | MediaKind::Jxl) {
        if let Some(cs) = read_head(path, ICC_HEAD).as_deref().and_then(crate::icc::color_space_in) {
            m.color_space = Some(cs);
        }
    }
    // An export from Lightroom or Capture One may name itself only in XMP.
    if m.software.is_none() {
        m.software = crate::xmp::creator_tool(path);
    }
    if (m.width.is_none() || m.height.is_none()) && kind == MediaKind::Image {
        if let Ok(reader) = image::ImageReader::open(path) {
            if let Ok(reader) = reader.with_guessed_format() {
                if let Ok((w, h)) = reader.into_dimensions() {
                    m.width = Some(w);
                    m.height = Some(h);
                }
            }
        }
    }
    if kind == MediaKind::Raw && (m.make.is_none() || m.captured_at.is_none()) {
        fill_from_rawler(&mut m, path);
    }
    // Lightroom keeps the true date in XMP even when it has rewritten the EXIF edit time,
    // and a PNG export carries no EXIF dates at all.
    if m.capture_source.is_none() || m.capture_source >= Some(CaptureSource::Xmp) {
        if let Some(found) = crate::xmp::capture_date(path) {
            let dates = RawDates { xmp: Some(found), ..m.exif_dates.clone() };
            apply_capture(&mut m, &dates);
            m.exif_dates = dates;
            if m.capture_source == Some(CaptureSource::Xmp) {
                m.capture_subsec_ms = None; // the EXIF fraction belonged to another date
            }
        }
    }
    m
}

/// How much of a file is searched for its ICC profile.
const ICC_HEAD: usize = 512 * 1024;

/// Up to `n` bytes from the start of `path`.
fn read_head(path: &Path, n: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(n.min(64 * 1024));
    File::open(path).ok()?.take(n as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn ascii(exif: &exif::Exif, tag: Tag) -> Option<String> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Ascii(v) if !v.is_empty() => {
            let s = String::from_utf8_lossy(&v[0]).trim().trim_end_matches('\0').to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
        _ => None,
    }
}

fn uint(exif: &exif::Exif, tag: Tag) -> Option<u32> {
    exif.get_field(tag, In::PRIMARY)?.value.get_uint(0)
}

fn rational(exif: &exif::Exif, tag: Tag) -> Option<f64> {
    match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(v) if !v.is_empty() => Some(v[0].to_f64()),
        _ => None,
    }
}

fn dms(exif: &exif::Exif, tag: Tag, ref_tag: Tag) -> Option<f64> {
    let v = match &exif.get_field(tag, In::PRIMARY)?.value {
        Value::Rational(v) if v.len() >= 3 => v.clone(),
        _ => return None,
    };
    let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
    let neg = matches!(ascii(exif, ref_tag).as_deref(), Some("S") | Some("W"));
    Some(if neg { -deg } else { deg })
}

/// An exposure compensation as photographers write it: "+0.7 EV", "0 EV", "-1 EV".
fn ev(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r == 0.0 {
        "0 EV".into()
    } else if r.fract() == 0.0 {
        format!("{r:+.0} EV")
    } else {
        format!("{r:+.1} EV")
    }
}

pub fn parse_exif_datetime(s: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y:%m:%d %H:%M:%S")
        .ok()
        .map(|n| n.and_utc().timestamp())
}

/// `+02:00` → 120, `-05:30` → -330.
pub fn parse_utc_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => return None,
    };
    let (h, m) = rest.split_once(':')?;
    Some(sign * (h.parse::<i32>().ok()? * 60 + m.parse::<i32>().ok()?))
}

/// Where a photo's date came from, best first. The order is the type's `Ord`, so a
/// re-read may only replace a date with one from a better source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSource {
    /// EXIF `DateTimeOriginal` (0x9003): the shutter.
    Original,
    /// EXIF `DateTimeDigitized` (0x9004, exiftool's *CreateDate*): what editors preserve.
    Digitized,
    /// XMP `photoshop:DateCreated` and friends.
    Xmp,
    /// `GPSDateStamp` + `GPSTimeStamp`, a UTC instant.
    Gps,
    /// EXIF `DateTime` (0x0132, exiftool's *ModifyDate*): the capture time on an untouched
    /// original, the export time on anything Lightroom has written.
    Modified,
    /// The file's own modified time, used when the file carries no date at all.
    FileTime,
}

impl CaptureSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Digitized => "digitized",
            Self::Xmp => "xmp",
            Self::Gps => "gps",
            Self::Modified => "modified",
            Self::FileTime => "file_time",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Original, Self::Digitized, Self::Xmp, Self::Gps, Self::Modified, Self::FileTime]
            .into_iter()
            .find(|c| c.as_str() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureTime {
    /// Local wall time as a unix timestamp, the spec's `captured_at`.
    pub local: i64,
    pub tz_offset_minutes: Option<i32>,
    pub source: CaptureSource,
}

/// Every date string a file can offer, exactly as written. Collected by `fill_from_exif`
/// and `xmp::capture_date`, so the rules below are tested without fixture files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawDates {
    pub original: Option<String>,
    pub digitized: Option<String>,
    pub modified: Option<String>,
    pub offset_original: Option<String>,
    pub offset_digitized: Option<String>,
    pub offset: Option<String>,
    /// `2026:09:10`
    pub gps_date: Option<String>,
    /// `14:37:16`
    pub gps_time: Option<String>,
    /// ISO-8601, from the XMP packet.
    pub xmp: Option<String>,
}

/// `2026-09-09T12:31:29.20`, `…+02:00`, `…Z` → local wall seconds and the offset.
pub fn parse_xmp_datetime(s: &str) -> Option<(i64, Option<i32>)> {
    let s = s.trim();
    let (stamp, offset) = match s.chars().last() {
        Some('Z') => (&s[..s.len() - 1], Some(0)),
        _ if s.len() > 6 && matches!(s.as_bytes()[s.len() - 6], b'+' | b'-') => {
            (&s[..s.len() - 6], parse_utc_offset(&s[s.len() - 6..]))
        }
        _ => (s, None),
    };
    // Fractional seconds carry no information we keep.
    let stamp = stamp.split_once('.').map_or(stamp, |(head, _)| head);
    let naive = chrono::NaiveDateTime::parse_from_str(stamp.trim(), "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(stamp.trim(), "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Some((naive.and_utc().timestamp(), offset))
}

/// The date a photo was taken, from the best source the file offers: the shutter, then the
/// editor's create date, then XMP, then GPS, and only then the file's own edit time —
/// which on a Lightroom export is the moment of export, not of the photograph.
pub fn choose_capture(d: &RawDates) -> Option<CaptureTime> {
    let offset = |s: &Option<String>| s.as_deref().and_then(parse_utc_offset);
    let general = offset(&d.offset);

    if let Some(local) = d.original.as_deref().and_then(parse_exif_datetime) {
        return Some(CaptureTime { local, tz_offset_minutes: offset(&d.offset_original).or(general), source: CaptureSource::Original });
    }
    if let Some(local) = d.digitized.as_deref().and_then(parse_exif_datetime) {
        return Some(CaptureTime { local, tz_offset_minutes: offset(&d.offset_digitized).or(general), source: CaptureSource::Digitized });
    }
    if let Some((local, own)) = d.xmp.as_deref().and_then(parse_xmp_datetime) {
        return Some(CaptureTime { local, tz_offset_minutes: own.or(general), source: CaptureSource::Xmp });
    }
    if let (Some(date), Some(time)) = (d.gps_date.as_deref(), d.gps_time.as_deref()) {
        if let Some(utc) = parse_exif_datetime(&format!("{date} {time}")) {
            // GPS is UTC; with an offset it becomes the wall time the photographer saw.
            return Some(CaptureTime {
                local: utc + general.unwrap_or(0) as i64 * 60,
                tz_offset_minutes: general,
                source: CaptureSource::Gps,
            });
        }
    }
    let local = d.modified.as_deref().and_then(parse_exif_datetime)?;
    Some(CaptureTime { local, tz_offset_minutes: general, source: CaptureSource::Modified })
}

/// The capture date from the first bytes of a file (its header), without a path: the
/// device browser dates what is on a card before importing it, and reading whole files off
/// a card to sort them would take minutes. EXIF in a JPEG or a TIFF-based RAW sits near the
/// start; the XMP packet is scanned in the same bytes. `None` when neither is there.
pub fn capture_in_head(head: &[u8]) -> Option<CaptureTime> {
    let mut dates = RawDates::default();
    if let Ok(exif) = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(head)) {
        let mut m = Metadata { orientation: 1, ..Default::default() };
        fill_from_exif(&mut m, &exif);
        dates = m.exif_dates;
    }
    let best = choose_capture(&dates);
    if best.is_none() || best.is_some_and(|c| c.source >= CaptureSource::Xmp) {
        if let Some(found) = crate::xmp::capture_date_in(head) {
            dates.xmp = Some(found);
            return choose_capture(&dates);
        }
    }
    best
}

/// The JPEG thumbnail a camera keeps in EXIF (IFD1), with the photo's orientation tag, from
/// the first bytes of a JPEG or a TIFF-based RAW. A card's browser shows these instead of
/// reading a 25 MB RAW whole for a 120 px tile. `None` when there is none in `head`.
pub fn exif_thumbnail(head: &[u8]) -> Option<(Vec<u8>, u16)> {
    let exif = exif::Reader::new().read_from_container(&mut std::io::Cursor::new(head)).ok()?;
    let at = exif.get_field(Tag::JPEGInterchangeFormat, In::THUMBNAIL).and_then(|f| f.value.get_uint(0))? as usize;
    let len = exif.get_field(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL).and_then(|f| f.value.get_uint(0))? as usize;
    let jpeg = exif.buf().get(at..at.checked_add(len)?)?;
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let orientation = uint(&exif, Tag::Orientation).filter(|o| (1..=8).contains(o)).unwrap_or(1) as u16;
    Some((jpeg.to_vec(), orientation))
}

fn fill_from_exif(m: &mut Metadata, exif: &exif::Exif) {
    m.orientation = uint(exif, Tag::Orientation).filter(|o| (1..=8).contains(o)).unwrap_or(1) as u16;
    m.width = uint(exif, Tag::PixelXDimension).or_else(|| uint(exif, Tag::ImageWidth));
    m.height = uint(exif, Tag::PixelYDimension).or_else(|| uint(exif, Tag::ImageLength));
    m.make = ascii(exif, Tag::Make);
    m.model = ascii(exif, Tag::Model);
    m.body_serial = ascii(exif, Tag::BodySerialNumber);
    m.lens = ascii(exif, Tag::LensModel);
    m.lens_make = ascii(exif, Tag::LensMake);
    m.digitized_at = ascii(exif, Tag::DateTimeDigitized).as_deref().and_then(parse_exif_datetime);
    // 0xFFFF is "uncalibrated": the ICC profile says what it is.
    m.color_space = match uint(exif, Tag::ColorSpace) {
        Some(1) => Some("sRGB".into()),
        Some(2) => Some("Adobe RGB".into()),
        _ => None,
    };
    m.exposure = exif.get_field(Tag::ExposureTime, In::PRIMARY).map(|f| f.display_value().to_string());
    m.aperture = rational(exif, Tag::FNumber);
    m.iso = uint(exif, Tag::PhotographicSensitivity);
    m.focal_length_mm = rational(exif, Tag::FocalLength);
    m.software = ascii(exif, Tag::Software);
    m.white_balance = uint(exif, Tag::WhiteBalance).map(|v| if v == 1 { "Manual" } else { "Auto" }.to_string());
    m.flash = uint(exif, Tag::Flash).map(|v| if v & 1 == 1 { "Fired" } else { "Did not fire" }.to_string());
    m.metering = uint(exif, Tag::MeteringMode).and_then(|v| {
        Some(
            match v {
                1 => "Average",
                2 => "Center-weighted average",
                3 => "Spot",
                4 => "Multi-spot",
                5 => "Pattern",
                6 => "Partial",
                _ => return None,
            }
            .to_string(),
        )
    });
    m.exposure_program = uint(exif, Tag::ExposureProgram).and_then(|v| {
        Some(
            match v {
                1 => "Manual",
                2 => "Program",
                3 => "Aperture priority",
                4 => "Shutter priority",
                5 => "Creative",
                6 => "Action",
                7 => "Portrait",
                8 => "Landscape",
                _ => return None,
            }
            .to_string(),
        )
    });
    m.exposure_bias = match &exif.get_field(Tag::ExposureBiasValue, In::PRIMARY).map(|f| &f.value) {
        Some(Value::SRational(v)) if !v.is_empty() && v[0].denom != 0 => Some(ev(v[0].to_f64())),
        _ => None,
    };
    m.gps_lat = dms(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef);
    m.gps_lon = dms(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef);
    m.gps_altitude = rational(exif, Tag::GPSAltitude);
    let dates = RawDates {
        original: ascii(exif, Tag::DateTimeOriginal),
        digitized: ascii(exif, Tag::DateTimeDigitized),
        modified: ascii(exif, Tag::DateTime),
        offset_original: ascii(exif, Tag::OffsetTimeOriginal),
        offset_digitized: ascii(exif, Tag::OffsetTimeDigitized),
        offset: ascii(exif, Tag::OffsetTime),
        gps_date: ascii(exif, Tag::GPSDateStamp),
        gps_time: exif.get_field(Tag::GPSTimeStamp, In::PRIMARY).and_then(gps_time),
        xmp: None,
    };
    apply_capture(m, &dates);
    m.exif_dates = dates;
    m.gps_direction = rational(exif, Tag::GPSImgDirection).filter(|d| d.is_finite());
    let subsec_tag = match m.capture_source {
        Some(CaptureSource::Digitized) => Tag::SubSecTimeDigitized,
        Some(CaptureSource::Modified) => Tag::SubSecTime,
        _ => Tag::SubSecTimeOriginal,
    };
    m.capture_subsec_ms = ascii(exif, subsec_tag).as_deref().and_then(subsec_ms);
}

/// EXIF `SubSecTime*` is the decimal part of the seconds as digits: "42" is 0.42 s.
fn subsec_ms(s: &str) -> Option<u16> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ms = 0u16;
    for i in 0..3 {
        ms = ms * 10 + s.as_bytes().get(i).map_or(0, |b| (b - b'0') as u16);
    }
    Some(ms)
}

/// `GPSTimeStamp` is three rationals: hours, minutes, seconds UTC.
fn gps_time(field: &exif::Field) -> Option<String> {
    match &field.value {
        Value::Rational(v) if v.len() >= 3 => {
            Some(format!("{:02}:{:02}:{:02}", v[0].to_f64() as u32, v[1].to_f64() as u32, v[2].to_f64() as u32))
        }
        _ => None,
    }
}

fn apply_capture(m: &mut Metadata, dates: &RawDates) {
    if let Some(c) = choose_capture(dates) {
        m.captured_at = Some(c.local);
        m.tz_offset_minutes = c.tz_offset_minutes;
        m.capture_source = Some(c.source);
    }
}

fn fill_from_rawler(m: &mut Metadata, path: &Path) {
    let Some(md) = crate::raw_lib::current().and_then(|lib| lib.metadata(path)) else { return };
    if m.make.is_none() && !md.make.is_empty() {
        m.make = Some(md.make.clone());
    }
    if m.model.is_none() && !md.model.is_empty() {
        m.model = Some(md.model.clone());
    }
    if m.body_serial.is_none() {
        m.body_serial = md.serial.clone();
    }
    if m.captured_at.is_none() {
        if let Some(dt) = md.date_time_original.as_deref() {
            m.captured_at = parse_exif_datetime(dt);
            if m.captured_at.is_some() {
                m.capture_source = Some(CaptureSource::Original);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dates() -> RawDates {
        RawDates::default()
    }

    #[test]
    fn the_shutter_time_wins_over_everything_else() {
        let d = RawDates {
            original: Some("2026:09:09 12:31:29".into()),
            digitized: Some("2026:09:09 12:31:29".into()),
            modified: Some("2026:09:18 14:42:42".into()),
            offset_original: Some("+02:00".into()),
            ..dates()
        };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.source, CaptureSource::Original);
        assert_eq!(c.local, parse_exif_datetime("2026:09:09 12:31:29").unwrap());
        assert_eq!(c.tz_offset_minutes, Some(120));
    }

    /// The Lightroom export in the bug report: no 0x9003, a true 0x9004, an edit-time 0x0132.
    #[test]
    fn create_date_beats_the_edit_time() {
        let d = RawDates {
            digitized: Some("2026:09:10 16:37:17".into()),
            modified: Some("2026:09:18 14:42:48".into()),
            offset: Some("+02:00".into()),
            ..dates()
        };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.source, CaptureSource::Digitized);
        assert_eq!(c.local, parse_exif_datetime("2026:09:10 16:37:17").unwrap());
        assert_eq!(c.tz_offset_minutes, Some(120));
    }

    #[test]
    fn xmp_is_read_when_exif_has_only_an_edit_time() {
        let d = RawDates {
            modified: Some("2026:09:18 14:42:48".into()),
            xmp: Some("2026-09-10T16:37:17+02:00".into()),
            ..dates()
        };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.source, CaptureSource::Xmp);
        assert_eq!(c.local, parse_exif_datetime("2026:09:10 16:37:17").unwrap());
        assert_eq!(c.tz_offset_minutes, Some(120));
    }

    #[test]
    fn gps_supplies_a_utc_instant_and_the_offset_makes_it_local() {
        let d = RawDates {
            modified: Some("2026:09:18 14:42:48".into()),
            gps_date: Some("2026:09:10".into()),
            gps_time: Some("14:37:16".into()),
            offset: Some("+02:00".into()),
            ..dates()
        };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.source, CaptureSource::Gps);
        assert_eq!(c.local, parse_exif_datetime("2026:09:10 16:37:16").unwrap());

        // Without an offset the UTC instant stands in for local time, and says so.
        let d = RawDates { offset: None, ..d };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.local, parse_exif_datetime("2026:09:10 14:37:16").unwrap());
        assert_eq!(c.tz_offset_minutes, None);
    }

    #[test]
    fn an_untouched_original_still_uses_its_modify_date() {
        let d = RawDates { modified: Some("2011:03:04 09:15:00".into()), ..dates() };
        let c = choose_capture(&d).unwrap();
        assert_eq!(c.source, CaptureSource::Modified);
        assert_eq!(c.local, parse_exif_datetime("2011:03:04 09:15:00").unwrap());
    }

    #[test]
    fn nothing_in_the_file_means_no_answer() {
        assert_eq!(choose_capture(&dates()), None);
    }

    #[test]
    fn xmp_timestamps_parse_with_and_without_offset_and_fractions() {
        assert_eq!(parse_xmp_datetime("2026-09-09T12:31:29.20"), Some((parse_exif_datetime("2026:09:09 12:31:29").unwrap(), None)));
        assert_eq!(parse_xmp_datetime("2026-09-10T16:37:17+02:00"), Some((parse_exif_datetime("2026:09:10 16:37:17").unwrap(), Some(120))));
        assert_eq!(parse_xmp_datetime("2026-09-10T16:37:17Z"), Some((parse_exif_datetime("2026:09:10 16:37:17").unwrap(), Some(0))));
        assert_eq!(parse_xmp_datetime("nonsense"), None);
    }

    /// A worse source must never displace a better one during the backfill.
    #[test]
    fn sources_rank_best_first() {
        assert!(CaptureSource::Original < CaptureSource::Digitized);
        assert!(CaptureSource::Digitized < CaptureSource::Xmp);
        assert!(CaptureSource::Xmp < CaptureSource::Gps);
        assert!(CaptureSource::Gps < CaptureSource::Modified);
        assert!(CaptureSource::Modified < CaptureSource::FileTime);
        assert_eq!(CaptureSource::parse("digitized"), Some(CaptureSource::Digitized));
        assert_eq!(CaptureSource::Digitized.as_str(), "digitized");
    }

    /// The whole path, through kamadak's parser: a blob with only `DateTimeDigitized`
    /// and an edit-time `DateTime` must still date the photo to the capture.
    #[test]
    fn an_exif_blob_with_only_a_create_date_is_read_correctly() {
        use exif::experimental::Writer;
        use exif::{Field, In, Tag, Value};

        let mut blob = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let digitized = Field { tag: Tag::DateTimeDigitized, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2026:09:10 16:37:17".to_vec()]) };
        let modified = Field { tag: Tag::DateTime, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2026:09:18 14:42:48".to_vec()]) };
        let offset = Field { tag: Tag::OffsetTime, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"+02:00".to_vec()]) };
        writer.push_field(&digitized);
        writer.push_field(&modified);
        writer.push_field(&offset);
        writer.write(&mut blob, false).unwrap();

        let exif = exif::Reader::new().read_raw(blob.into_inner()).unwrap();
        let mut m = Metadata { orientation: 1, ..Default::default() };
        fill_from_exif(&mut m, &exif);
        assert_eq!(m.captured_at, parse_exif_datetime("2026:09:10 16:37:17"));
        assert_eq!(m.capture_source, Some(CaptureSource::Digitized));
        assert_eq!(m.tz_offset_minutes, Some(120));
    }

    /// Round 9A: the date digitized, the lens maker and the colour space, for search.
    #[test]
    fn reads_the_date_digitized_the_lens_maker_and_the_colour_space() {
        use exif::experimental::Writer;
        use exif::{Field, In, Tag, Value};

        let mut blob = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let fields = [
            Field { tag: Tag::DateTimeOriginal, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2026:09:10 16:37:17".to_vec()]) },
            Field { tag: Tag::DateTimeDigitized, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2026:09:11 08:00:00".to_vec()]) },
            Field { tag: Tag::LensMake, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"Sigma".to_vec()]) },
            Field { tag: Tag::ColorSpace, ifd_num: In::PRIMARY, value: Value::Short(vec![1]) },
        ];
        for f in &fields {
            writer.push_field(f);
        }
        writer.write(&mut blob, false).unwrap();
        let exif = exif::Reader::new().read_raw(blob.into_inner()).unwrap();
        let mut m = Metadata { orientation: 1, ..Default::default() };
        fill_from_exif(&mut m, &exif);
        assert_eq!(m.digitized_at, parse_exif_datetime("2026:09:11 08:00:00"));
        assert_eq!(m.captured_at, parse_exif_datetime("2026:09:10 16:37:17"), "the capture date is still the original's");
        assert_eq!(m.lens_make.as_deref(), Some("Sigma"));
        assert_eq!(m.color_space.as_deref(), Some("sRGB"));
        assert_eq!(METADATA_VERSION, 5, "older photos are read again for these");
    }

    /// Round 19 C3: the fraction of a second the shutter fell in, and where the camera pointed.
    #[test]
    fn reads_the_sub_second_of_the_capture_and_the_gps_direction() {
        use exif::experimental::Writer;
        use exif::{Field, In, Rational, Tag, Value};

        let build = |original: &[u8], digitized: &[u8]| {
            let mut blob = std::io::Cursor::new(Vec::new());
            let mut writer = Writer::new();
            let fields = [
                Field { tag: Tag::DateTimeOriginal, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2026:09:10 16:37:17".to_vec()]) },
                Field { tag: Tag::SubSecTimeOriginal, ifd_num: In::PRIMARY, value: Value::Ascii(vec![original.to_vec()]) },
                Field { tag: Tag::SubSecTimeDigitized, ifd_num: In::PRIMARY, value: Value::Ascii(vec![digitized.to_vec()]) },
                Field { tag: Tag::GPSImgDirection, ifd_num: In::PRIMARY, value: Value::Rational(vec![Rational { num: 2705, denom: 10 }]) },
            ];
            for f in &fields {
                writer.push_field(f);
            }
            writer.write(&mut blob, false).unwrap();
            let exif = exif::Reader::new().read_raw(blob.into_inner()).unwrap();
            let mut m = Metadata { orientation: 1, ..Default::default() };
            fill_from_exif(&mut m, &exif);
            m
        };
        let m = build(b"42", b"99");
        assert_eq!(m.capture_subsec_ms, Some(420), "two digits are hundredths");
        assert_eq!(m.gps_direction, Some(270.5));
        assert_eq!(build(b"5", b"").capture_subsec_ms, Some(500));
        assert_eq!(build(b"123456", b"").capture_subsec_ms, Some(123), "finer than a millisecond is cut");
        assert_eq!(build(b"  ", b"").capture_subsec_ms, None, "blank is no reading");
        assert_eq!(subsec_ms("0"), Some(0));
        assert_eq!(subsec_ms("x1"), None);
    }

    /// Round 7: what a tag can name beyond the camera and the exposure.
    #[test]
    fn reads_the_software_and_how_the_photo_was_exposed() {
        use exif::experimental::Writer;
        use exif::{Field, In, Rational, SRational, Tag, Value};

        let mut blob = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let fields = [
            Field { tag: Tag::Software, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"Adobe Photoshop Lightroom Classic 13.0 (Windows)".to_vec()]) },
            Field { tag: Tag::WhiteBalance, ifd_num: In::PRIMARY, value: Value::Short(vec![0]) },
            Field { tag: Tag::Flash, ifd_num: In::PRIMARY, value: Value::Short(vec![0x19]) },
            Field { tag: Tag::MeteringMode, ifd_num: In::PRIMARY, value: Value::Short(vec![5]) },
            Field { tag: Tag::ExposureProgram, ifd_num: In::PRIMARY, value: Value::Short(vec![3]) },
            Field { tag: Tag::ExposureBiasValue, ifd_num: In::PRIMARY, value: Value::SRational(vec![SRational { num: -2, denom: 3 }]) },
            Field { tag: Tag::FNumber, ifd_num: In::PRIMARY, value: Value::Rational(vec![Rational { num: 18, denom: 10 }]) },
        ];
        for f in &fields {
            writer.push_field(f);
        }
        writer.write(&mut blob, false).unwrap();
        let exif = exif::Reader::new().read_raw(blob.into_inner()).unwrap();
        let mut m = Metadata { orientation: 1, ..Default::default() };
        fill_from_exif(&mut m, &exif);
        assert_eq!(m.software.as_deref(), Some("Adobe Photoshop Lightroom Classic 13.0 (Windows)"));
        assert_eq!(m.white_balance.as_deref(), Some("Auto"));
        assert_eq!(m.flash.as_deref(), Some("Fired"));
        assert_eq!(m.metering.as_deref(), Some("Pattern"));
        assert_eq!(m.exposure_program.as_deref(), Some("Aperture priority"));
        assert_eq!(m.exposure_bias.as_deref(), Some("-0.7 EV"));
    }

    #[test]
    fn the_xmp_creator_tool_names_the_app_that_exported_a_file() {
        let packet = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:Description xmp:CreatorTool="Capture One 23 Windows" /></x:xmpmeta>"#;
        assert_eq!(crate::xmp::creator_tool_in(packet).as_deref(), Some("Capture One 23 Windows"));
    }

    #[test]
    fn probe_fills_video_facts_without_overriding_exif() {
        let mut m = Metadata { orientation: 1, ..Default::default() };
        let p = Probe {
            width: Some(1920),
            height: Some(1080),
            rotation: 90,
            duration_ms: Some(19800),
            creation_time: Some(1_000),
            creation_local: Some(8_200),
            tz_offset_minutes: Some(120),
            make: Some("Apple".into()),
            model: Some("iPhone".into()),
            ..Default::default()
        };
        apply_probe(&mut m, p);
        assert_eq!((m.width, m.height), (Some(1080), Some(1920)));
        assert_eq!(m.duration_ms, Some(19800));
        assert_eq!(m.captured_at, Some(8_200));
        assert_eq!(m.tz_offset_minutes, Some(120));
        assert_eq!(m.model.as_deref(), Some("iPhone"));
        assert!(m.probe.is_some());
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("probe"));
        let back: Metadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn parses_exif_time_and_offsets() {
        assert_eq!(parse_exif_datetime("2026:09:17 18:30:00"), Some(1_789_669_800));
        assert_eq!(parse_utc_offset("+02:00"), Some(120));
        assert_eq!(parse_utc_offset("-05:30"), Some(-330));
        assert_eq!(parse_utc_offset("bogus"), None);
    }

    /// The device browser reads only a file's head: a JPEG's APP1 is found there.
    #[test]
    fn a_capture_date_is_read_from_the_head_of_a_jpeg() {
        use exif::experimental::Writer;
        use exif::{Field, In, Tag, Value};

        let mut blob = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let original = Field { tag: Tag::DateTimeOriginal, ifd_num: In::PRIMARY, value: Value::Ascii(vec![b"2025:03:14 09:26:53".to_vec()]) };
        writer.push_field(&original);
        writer.write(&mut blob, false).unwrap();
        let tiff = blob.into_inner();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend_from_slice(&((tiff.len() + 8) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        // Where the picture would go: the head ends here, before any of it.
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02]);

        let c = capture_in_head(&jpeg).unwrap();
        assert_eq!(c.source, CaptureSource::Original);
        assert_eq!(c.local, parse_exif_datetime("2025:03:14 09:26:53").unwrap());
        assert!(capture_in_head(b"not an image at all").is_none());
    }

    #[test]
    fn the_exif_thumbnail_is_read_from_the_head() {
        use exif::experimental::Writer;
        use exif::{Field, In, Tag, Value};

        let small = image::RgbImage::from_pixel(160, 120, image::Rgb([10, 200, 30]));
        let mut thumb = Vec::new();
        image::DynamicImage::ImageRgb8(small).write_to(&mut std::io::Cursor::new(&mut thumb), image::ImageFormat::Jpeg).unwrap();
        let mut blob = std::io::Cursor::new(Vec::new());
        let mut writer = Writer::new();
        let orientation = Field { tag: Tag::Orientation, ifd_num: In::PRIMARY, value: Value::Short(vec![6]) };
        writer.push_field(&orientation);
        writer.set_jpeg(&thumb, In::THUMBNAIL);
        writer.write(&mut blob, false).unwrap();

        // A TIFF-based RAW starts like this blob does.
        let (jpeg, o) = exif_thumbnail(blob.get_ref()).unwrap();
        assert_eq!(o, 6);
        assert_eq!(jpeg, thumb);
        let img = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((img.width(), img.height()), (160, 120));
        assert!(exif_thumbnail(b"no exif here").is_none());
    }
}
