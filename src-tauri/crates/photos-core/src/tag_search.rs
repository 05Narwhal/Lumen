//! Search by tags (Round 7): every grid page's search field offers *tags* for anything Lumen
//! knows (people, pets, places, albums, scene tags, dates, kinds, ratings and every EXIF
//! field, *Exported from* included) and the chosen tags filter the page in place.
//!
//! Tags combine with AND, except values of one field that exclude each other (Photos or
//! Videos, two cameras, two places, two dates), which are one OR group ([`AnyOf`]). People,
//! pets, scene tags and albums stay AND: one photo can show both.
//!
//! [`Facets`] are what can be suggested, with how many photos each has; the app reads them
//! once per library version. [`suggest`] then answers each keystroke from memory.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::query::{AnyOf, AssetKind, AssetQuery, DateField, ExifField, LumenMark, Measure, PlaceRef, Shape, TextMatch};
use crate::query_parse::{fold, name_words, read_date, read_date_expr, DateSpan};
use crate::settings::DateOrder;
use crate::Result;

/// One chosen tag: what it narrows to, and what its chip says.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchTag {
    pub label: String,
    pub value: TagValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum TagValue {
    Person {
        #[ts(type = "number")]
        id: i64,
    },
    Pet {
        #[ts(type = "number")]
        id: i64,
    },
    Place { place: PlaceRef },
    Album {
        #[ts(type = "number")]
        id: i64,
    },
    /// Photos in any album of a collection, nested collections included (Round 19 §7).
    Collection {
        #[ts(type = "number")]
        id: i64,
    },
    /// A scene tag (`embedding::SCENE_TAGS`).
    Scene { tag: String },
    /// Inclusive, in `captured_at` seconds; `None` is an open end (Round 11: "before 2024").
    Dates {
        #[ts(type = "number | null")]
        from: Option<i64>,
        #[ts(type = "number | null")]
        to: Option<i64>,
        /// Which date (Round 8A); absent is when it was taken.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        field: Option<DateField>,
    },
    /// Months of any year (1–12): "March", "Summer".
    Months { months: Vec<u8> },
    /// 0 photos (Live Photos included), 1 videos.
    Media {
        #[ts(type = "number")]
        media: i64,
    },
    Kind { kind: AssetKind },
    Favorites,
    /// At least this many stars.
    Rating {
        #[ts(type = "number")]
        min: i64,
    },
    /// 1 picked, −1 rejected.
    Flag { flag: i8 },
    Location { has: bool },
    /// Text was read in it (Round 13; offered only while *Text in photos* is on).
    HasText,
    /// Read, and nothing found (Round 18 C2).
    NoText,
    /// `Text: menu` / `Text: "pri mostu"` (Round 18 C2): words in the text read in the photo,
    /// not its title, caption or keywords. `exact` keeps the words together.
    PhotoText { text: String, exact: bool },
    /// An EXIF field's value as `metadata_json` stores it.
    Exif { field: ExifField, value: String },
    /// Without the dot.
    Extension { ext: String },
    /// Words searched as the search field always has: captions, keywords, by meaning.
    Text { text: String },
    /// `[min, max)` of a measure (Round 8A): *Under 10 s*, *Over 50 MB*, *12 MP and up*.
    Range {
        measure: Measure,
        #[ts(type = "number | null")]
        min: Option<i64>,
        #[ts(type = "number | null")]
        max: Option<i64>,
    },
    /// Portrait, landscape or square (Round 8A).
    Shape { shape: Shape },
    /// Edited, in Develop, or exported by Lumen (Round 8A).
    Lumen { mark: LumenMark },
    /// A copy made for an external editor (Round 22 lane C): any app, or the app with this id.
    EditedIn {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        app: Option<String>,
    },
}

/// A tag the field offers for what was typed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TagSuggestion {
    pub tag: SearchTag,
    /// What the tag is, as the list shows it: "Person", "Camera", "Exported from".
    pub field: String,
    /// How many photos have it, when known.
    #[ts(type = "number | null")]
    pub count: Option<i64>,
}

/// `[min, max)`, either end open.
type Span = (Option<i64>, Option<i64>);

/// The query `tags` ask for: OR inside a field whose values exclude each other, AND between.
pub fn to_query(tags: &[SearchTag]) -> AssetQuery {
    fn add<T: PartialEq>(v: &mut Vec<T>, x: T) {
        if !v.contains(&x) {
            v.push(x);
        }
    }
    let mut q = AssetQuery::default();
    let (mut media, mut kinds, mut places, mut dates, mut exts) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut exif: Vec<(ExifField, Vec<String>)> = Vec::new();
    let mut other_dates: Vec<(DateField, Vec<Span>)> = Vec::new();
    let mut ranges: Vec<(Measure, Vec<Span>)> = Vec::new();
    let (mut shapes, mut lumen) = (Vec::new(), Vec::new());
    let mut edited_in: Vec<Option<String>> = Vec::new();
    let mut text: Vec<&str> = Vec::new();
    for tag in tags {
        match &tag.value {
            TagValue::Person { id } => add(&mut q.person_ids, *id),
            TagValue::Pet { id } => add(&mut q.pet_ids, *id),
            TagValue::Album { id } => add(&mut q.album_ids, *id),
            TagValue::Collection { id } => add(&mut q.collection_ids, *id),
            TagValue::Scene { tag } => add(&mut q.tags, tag.clone()),
            TagValue::Place { place } => add(&mut places, place.clone()),
            TagValue::Dates { from, to, field: None | Some(DateField::Taken) } => add(&mut dates, (*from, *to)),
            TagValue::Dates { from, to, field: Some(field) } => match other_dates.iter_mut().find(|(f, _)| f == field) {
                Some((_, spans)) => add(spans, (*from, *to)),
                None => other_dates.push((*field, vec![(*from, *to)])),
            },
            TagValue::Range { measure, min, max } => match ranges.iter_mut().find(|(m, _)| m == measure) {
                Some((_, spans)) => add(spans, (*min, *max)),
                None => ranges.push((*measure, vec![(*min, *max)])),
            },
            TagValue::Shape { shape } => add(&mut shapes, *shape),
            TagValue::Lumen { mark } => add(&mut lumen, *mark),
            TagValue::EditedIn { app } => edited_in.push(app.clone()),
            TagValue::Months { months } => months.iter().for_each(|m| add(&mut q.months, *m)),
            TagValue::Media { media: m } => add(&mut media, *m),
            TagValue::Kind { kind } => add(&mut kinds, *kind),
            TagValue::Favorites => q.favorites = true,
            // Two ratings: either, so the lower.
            TagValue::Rating { min } => q.min_rating = Some(q.min_rating.map_or(*min, |r| r.min(*min))),
            TagValue::Flag { flag } => q.flag = Some(*flag),
            TagValue::Location { has } => q.has_gps = Some(*has),
            TagValue::HasText => q.has_text = true,
            TagValue::NoText => q.no_text = true,
            TagValue::PhotoText { text, exact } => add(&mut q.in_text, TextMatch { text: text.trim().to_string(), exact: *exact }),
            TagValue::Exif { field, value } => match exif.iter_mut().find(|(f, _)| f == field) {
                Some((_, values)) => add(values, value.clone()),
                None => exif.push((*field, vec![value.clone()])),
            },
            TagValue::Extension { ext } => add(&mut exts, ext.to_lowercase()),
            TagValue::Text { text: t } => text.push(t.trim()),
        }
    }
    let groups = [
        (!media.is_empty()).then_some(AnyOf::MediaTypes(media)),
        (!kinds.is_empty()).then_some(AnyOf::Kinds(kinds)),
        (!places.is_empty()).then_some(AnyOf::Places(places)),
        (!dates.is_empty()).then_some(AnyOf::Dates(dates)),
        (!exts.is_empty()).then_some(AnyOf::Extensions(exts)),
    ];
    q.any_of = groups.into_iter().flatten().collect();
    q.any_of.extend(exif.into_iter().map(|(field, values)| AnyOf::Exif { field, values }));
    q.any_of.extend(other_dates.into_iter().map(|(field, spans)| AnyOf::DatesOf { field, spans }));
    q.any_of.extend(ranges.into_iter().map(|(measure, spans)| AnyOf::Ranges { measure, spans }));
    if !shapes.is_empty() {
        q.any_of.push(AnyOf::Shapes(shapes));
    }
    // Made in Lumen: each mark must hold (an edited photo that was also exported).
    q.any_of.extend(lumen.into_iter().map(|m| AnyOf::Lumen(vec![m])));
    // A photo has one `edited_in`, so two of these meet only when they say the same.
    q.any_of.extend(edited_in.into_iter().map(|a| AnyOf::EditedIn(vec![a])));
    let text = text.into_iter().filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
    q.text = (!text.is_empty()).then_some(text);
    q
}

/// `base` narrowed by `tags` (a [`to_query`]): both must hold.
pub fn merge(base: &AssetQuery, tags: &AssetQuery) -> AssetQuery {
    fn union<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
        let mut out = a.to_vec();
        out.extend(b.iter().filter(|x| !a.contains(x)).cloned());
        out
    }
    let mut q = base.clone();
    q.person_ids = union(&base.person_ids, &tags.person_ids);
    q.pet_ids = union(&base.pet_ids, &tags.pet_ids);
    q.album_ids = union(&base.album_ids, &tags.album_ids);
    q.collection_ids = union(&base.collection_ids, &tags.collection_ids);
    q.tags = union(&base.tags, &tags.tags);
    q.any_of = union(&base.any_of, &tags.any_of);
    // Months of the page and months of a tag: both must hold, so only the shared ones.
    q.months = match (base.months.is_empty(), tags.months.is_empty()) {
        (true, _) => tags.months.clone(),
        (_, true) => base.months.clone(),
        _ => base.months.iter().filter(|m| tags.months.contains(m)).copied().collect(),
    };
    if q.months.is_empty() && !(base.months.is_empty() && tags.months.is_empty()) {
        // Nothing shared: a span that holds nothing.
        q.any_of.push(AnyOf::Dates(vec![(Some(1), Some(0))]));
    }
    q.favorites = base.favorites || tags.favorites;
    q.min_rating = base.min_rating.max(tags.min_rating);
    q.flag = tags.flag.or(base.flag);
    q.has_gps = tags.has_gps.or(base.has_gps);
    q.has_text = base.has_text || tags.has_text;
    q.no_text = base.no_text || tags.no_text;
    q.in_text = union(&base.in_text, &tags.in_text);
    q.text = match (base.text.as_deref().map(str::trim).filter(|t| !t.is_empty()), tags.text.as_deref()) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b).map(str::to_string),
    };
    q
}

/// What the list says a field is.
fn exif_field_name(f: ExifField) -> &'static str {
    match f {
        ExifField::Make => "Camera make",
        ExifField::Model => "Camera",
        ExifField::Lens => "Lens",
        ExifField::Iso => "ISO",
        ExifField::Aperture => "Aperture",
        ExifField::Exposure => "Shutter speed",
        ExifField::FocalLength => "Focal length",
        ExifField::Software => "Exported from",
        ExifField::WhiteBalance => "White balance",
        ExifField::Flash => "Flash",
        ExifField::Metering => "Metering",
        ExifField::ExposureProgram => "Exposure mode",
        ExifField::ExposureBias => "Exposure bias",
        ExifField::VideoCodec => "Video codec",
        ExifField::BodySerial => "Camera body",
        ExifField::AudioCodec => "Audio codec",
        ExifField::ColorSpace => "Colour space",
        ExifField::HdrFormat => "HDR",
        ExifField::BitDepth => "Bit depth",
        ExifField::LensMake => "Lens maker",
    }
}

/// Other words that name a field, so typing one lists its values.
fn exif_field_aliases(f: ExifField) -> &'static str {
    match f {
        ExifField::Make => "make brand manufacturer",
        ExifField::Model => "camera model",
        ExifField::Lens => "lens",
        ExifField::Iso => "iso",
        ExifField::Aperture => "aperture f stop",
        ExifField::Exposure => "shutter speed exposure",
        ExifField::FocalLength => "focal length mm",
        ExifField::Software => "exported from software app",
        ExifField::WhiteBalance => "white balance wb",
        ExifField::Flash => "flash",
        ExifField::Metering => "metering",
        ExifField::ExposureProgram => "exposure mode program",
        ExifField::ExposureBias => "exposure bias compensation ev",
        ExifField::VideoCodec => "codec video format",
        ExifField::BodySerial => "camera body serial number",
        ExifField::AudioCodec => "audio codec sound",
        ExifField::ColorSpace => "colour color space profile gamut",
        ExifField::HdrFormat => "hdr dolby vision hlg pq hdr10",
        ExifField::BitDepth => "bit depth bits 10-bit 8-bit",
        ExifField::LensMake => "lens maker make brand",
    }
}

/// "36.0" as "36", "1.8" as "1.8".
fn number(v: &str) -> String {
    v.parse::<f64>().map(|n| format!("{n}")).unwrap_or_else(|_| v.to_string())
}

/// A value as the chip reads it. `apple` says the photos with this software are an iPhone's,
/// whose software is the iOS version alone ("18.5").
fn exif_label(f: ExifField, v: &str, apple: bool) -> String {
    match f {
        ExifField::Iso => format!("ISO {v}"),
        ExifField::Aperture => format!("f/{}", number(v)),
        ExifField::Exposure => format!("{v} s"),
        ExifField::FocalLength => format!("{} mm", number(v)),
        ExifField::Software if apple && v.starts_with(|c: char| c.is_ascii_digit()) => format!("iOS {v}"),
        ExifField::VideoCodec | ExifField::AudioCodec => codec_label(v),
        ExifField::BodySerial => format!("Serial {v}"),
        ExifField::BitDepth => format!("{v}-bit"),
        _ => v.to_string(),
    }
}

/// FFmpeg's codec names as people know them.
fn codec_label(v: &str) -> String {
    match v.to_lowercase().as_str() {
        "h264" => "H.264".into(),
        "hevc" | "h265" => "HEVC (H.265)".into(),
        "av1" => "AV1".into(),
        "vp8" => "VP8".into(),
        "vp9" => "VP9".into(),
        "prores" => "ProRes".into(),
        "mpeg2video" => "MPEG-2".into(),
        "mpeg4" => "MPEG-4".into(),
        "mjpeg" => "Motion JPEG".into(),
        "aac" => "AAC".into(),
        "mp3" => "MP3".into(),
        "opus" => "Opus".into(),
        "flac" => "FLAC".into(),
        "alac" => "Apple Lossless".into(),
        "ac3" => "Dolby Digital".into(),
        "eac3" => "Dolby Digital Plus".into(),
        // pcm_s16le, pcm_s24be, pcm_f32le: one chip per depth.
        pcm if pcm.starts_with("pcm_") => {
            let bits: String = pcm.chars().filter(|c| c.is_ascii_digit()).collect();
            match (bits.is_empty(), pcm.starts_with("pcm_f")) {
                (true, _) => "PCM".into(),
                (false, true) => format!("PCM ({bits}-bit float)"),
                (false, false) => format!("PCM ({bits}-bit)"),
            }
        }
        other => other.to_uppercase(),
    }
}

/// "Ver.1.03", "Firmware 2.10", "V1.2": a camera's firmware, not an app.
fn is_firmware(v: &str) -> bool {
    let l = v.trim().to_lowercase();
    let rest = l.strip_prefix("firmware").or_else(|| l.strip_prefix("ver")).or_else(|| l.strip_prefix('v')).unwrap_or(&l);
    rest.len() < l.len() && rest.trim_start_matches(['.', ' ', ':']).starts_with(|c: char| c.is_ascii_digit())
}

/// One thing that can be suggested.
#[derive(Debug, Clone)]
struct Entry {
    tag: SearchTag,
    field: String,
    count: Option<i64>,
    /// The label folded, for a typed phrase with punctuation ("f/1.8", "1/100").
    folded: String,
    /// The label's words, folded.
    words: Vec<String>,
    /// Words naming the field ("camera", "exported"), and a fixed tag's other names.
    aliases: Vec<String>,
    /// Offered even when no photo has it yet (Favorites, ratings): the tag is the way to look.
    fixed: bool,
}

/// Where facets are read from (Round 8A): one connection (tests), or the pool, a short hold
/// per query so no read keeps a connection long enough to be logged slow.
enum Source<'a> {
    Conn(&'a Connection),
    Db(&'a crate::db::Db),
}

impl Source<'_> {
    #[track_caller]
    fn run<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        match self {
            Source::Conn(c) => f(c),
            Source::Db(db) => db.with_conn(f),
        }
    }
}

/// Words that find the copies made for an external editor (Round 22 lane C).
const EDITED_IN_WORDS: &str = "edited external app editor copy open with";

/// Words that find what Lumen made.
const LUMEN_WORDS: &str = "lumen made edited edit edits developed develop exported export";

/// The spans every library is offered, counted like the fixed tags and listed when any
/// photo has them: `(label, measure, min, max, words)`, `[min, max)`.
#[allow(clippy::type_complexity)]
const BUCKETS: &[(&str, Measure, Option<i64>, Option<i64>, &str)] = &[
    ("Under 10 s", Measure::Duration, None, Some(10_000), DURATION_WORDS),
    ("10–30 s", Measure::Duration, Some(10_000), Some(30_000), DURATION_WORDS),
    ("30 s–1 min", Measure::Duration, Some(30_000), Some(60_000), DURATION_WORDS),
    ("1–5 min", Measure::Duration, Some(60_000), Some(300_000), DURATION_WORDS),
    ("Over 5 min", Measure::Duration, Some(300_000), None, DURATION_WORDS),
    ("Under 1 MB", Measure::FileSize, None, Some(1_000_000), SIZE_WORDS),
    ("Over 10 MB", Measure::FileSize, Some(10_000_000), None, SIZE_WORDS),
    ("Over 50 MB", Measure::FileSize, Some(50_000_000), None, SIZE_WORDS),
    ("Over 500 MB", Measure::FileSize, Some(500_000_000), None, SIZE_WORDS),
    ("Over 1 GB", Measure::FileSize, Some(1_000_000_000), None, SIZE_WORDS),
    ("Under 2 MP", Measure::Pixels, None, Some(2_000_000), PIXEL_WORDS),
    ("12 MP and up", Measure::Pixels, Some(12_000_000), None, PIXEL_WORDS),
    ("24 MP and up", Measure::Pixels, Some(24_000_000), None, PIXEL_WORDS),
    ("48 MP and up", Measure::Pixels, Some(48_000_000), None, PIXEL_WORDS),
    ("4K video", Measure::VideoSize, Some(3_840), None, VIDEO_SIZE_WORDS),
    ("1080p video", Measure::VideoSize, Some(1_920), Some(3_840), VIDEO_SIZE_WORDS),
    ("720p video", Measure::VideoSize, Some(1_280), Some(1_920), VIDEO_SIZE_WORDS),
    ("SD video", Measure::VideoSize, None, Some(1_280), VIDEO_SIZE_WORDS),
    ("Below sea level", Measure::Altitude, None, Some(0), ALTITUDE_WORDS),
    ("Up to 500 m", Measure::Altitude, Some(0), Some(500), ALTITUDE_WORDS),
    ("500–1,500 m", Measure::Altitude, Some(500), Some(1_500), ALTITUDE_WORDS),
    ("1,500–3,000 m", Measure::Altitude, Some(1_500), Some(3_000), ALTITUDE_WORDS),
    ("Above 3,000 m", Measure::Altitude, Some(3_000), None, ALTITUDE_WORDS),
];
const DURATION_WORDS: &str = "length duration long short video videos seconds minutes";
const SIZE_WORDS: &str = "file size big large small mb gb";
const PIXEL_WORDS: &str = "resolution megapixels mp pixels";
const VIDEO_SIZE_WORDS: &str = "resolution video 4k uhd hd full";
const ALTITUDE_WORDS: &str = "altitude elevation height mountain metres meters";

/// What the list says a measure is.
fn measure_name(m: Measure) -> &'static str {
    match m {
        Measure::Duration => "Length",
        Measure::FileSize => "File size",
        Measure::Pixels => "Resolution",
        Measure::VideoSize => "Video resolution",
        Measure::FrameRate => "Frame rate",
        Measure::Altitude => "Altitude",
    }
}

/// Rows the EXIF pass reads per hold.
const EXIF_CHUNK: i64 = 20_000;

/// Everything the search field can suggest, with photo counts.
#[derive(Debug, Clone, Default)]
pub struct Facets {
    entries: Vec<Entry>,
}

impl Facets {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The `n` places with the most photos, most first (Home's *Places*). A country and its
    /// towns are separate entries, as in the search box.
    pub fn top_places(&self, n: usize) -> Vec<PlaceRef> {
        let mut places: Vec<(&PlaceRef, i64)> = self
            .entries
            .iter()
            .filter_map(|e| match &e.tag.value {
                TagValue::Place { place } => Some((place, e.count.unwrap_or(0))),
                _ => None,
            })
            .collect();
        places.sort_by_key(|p| std::cmp::Reverse(p.1));
        places.into_iter().take(n).map(|(p, _)| p.clone()).collect()
    }

    /// Reads a library's facets: only what the Library shows counts, so nothing in the trash,
    /// the Hidden folder or an album that hides its photos is ever suggested.
    pub fn load(conn: &Connection) -> Result<Facets> {
        Self::read(Source::Conn(conn))
    }

    /// [`Facets::load`] from the pool, one short hold per query (the app's way).
    pub fn load_from(db: &crate::db::Db) -> Result<Facets> {
        Self::read(Source::Db(db))
    }

    fn read(src: Source<'_>) -> Result<Facets> {
        let shown = format!("a.state = 0 AND a.in_vault = 0 AND {} AND NOT {}", crate::concealed::SHOWN, crate::assets::MOVIE_HALF);
        let mut entries = Vec::new();
        let mut push = |label: String, value: TagValue, field: &str, count: Option<i64>, aliases: &str| {
            entries.push(Entry {
                folded: fold(&label),
                words: name_words(&label),
                aliases: name_words(aliases),
                fixed: matches!(
                    value,
                    TagValue::Media { .. } | TagValue::Kind { .. } | TagValue::Favorites | TagValue::Rating { .. } | TagValue::Flag { .. } | TagValue::Location { .. } | TagValue::HasText | TagValue::NoText | TagValue::Lumen { .. } | TagValue::EditedIn { app: None }
                ),
                tag: SearchTag { label, value },
                field: field.to_string(),
                count,
            });
        };
        let counted = |sql: &str| -> Result<HashMap<i64, i64>> {
            src.run(|c| {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })
        };
        let named = |sql: &str| -> Result<Vec<(i64, String)>> {
            src.run(|c| {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })
        };
        // (text, count) rows of one grouping query.
        let grouped = |sql: &str| -> Result<Vec<(String, i64)>> {
            src.run(|c| {
                let mut stmt = c.prepare(sql)?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })
        };

        // People and pets with a name.
        let faces = counted(&format!(
            "SELECT f.person_id, COUNT(DISTINCT f.asset_id) FROM faces f JOIN assets a ON a.id = f.asset_id
             WHERE f.person_id IS NOT NULL AND f.rejected IS NULL AND {shown} GROUP BY f.person_id",
        ))?;
        for (id, name) in named("SELECT id, name FROM people WHERE status = 'listed' AND name IS NOT NULL AND TRIM(name) != ''")? {
            push(name, TagValue::Person { id }, "Person", Some(faces.get(&id).copied().unwrap_or(0)), "person people");
        }
        let has_pets: bool = src.run(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'pets')", [], |r| r.get(0))?))?;
        if has_pets {
            let seen = counted(&format!(
                "SELECT d.pet_id, COUNT(DISTINCT d.asset_id) FROM pet_detections d JOIN assets a ON a.id = d.asset_id
                 WHERE d.pet_id IS NOT NULL AND d.rejected IS NULL AND {shown} GROUP BY d.pet_id",
            ))?;
            for (id, name) in named("SELECT id, name FROM pets WHERE status = 'listed' AND name IS NOT NULL AND TRIM(name) != ''")? {
                push(name, TagValue::Pet { id }, "Pet", Some(seen.get(&id).copied().unwrap_or(0)), "pet pets animal");
            }
        }

        // Places: towns and countries from `place_name`, villages and districts from `locality`.
        let mut places: Vec<(PlaceRef, i64)> = Vec::new();
        fn add_place(places: &mut Vec<(PlaceRef, i64)>, p: PlaceRef, n: i64) {
            match places.iter_mut().find(|(q, _)| *q == p) {
                Some((_, c)) => *c += n,
                None => places.push((p, n)),
            }
        }
        for (name, n) in grouped(&format!("SELECT a.place_name, COUNT(*) FROM assets a WHERE a.place_name IS NOT NULL AND a.place_name != '' AND {shown} GROUP BY a.place_name"))? {
            match name.rsplit_once(", ") {
                Some((city, country)) => {
                    add_place(&mut places, PlaceRef::City { name: city.into(), country: Some(country.into()) }, n);
                    add_place(&mut places, PlaceRef::Country { name: country.into() }, n);
                }
                None => add_place(&mut places, PlaceRef::Country { name }, n),
            }
        }
        for (label, n) in grouped(&format!("SELECT a.locality, COUNT(*) FROM assets a WHERE a.locality IS NOT NULL AND a.locality != '' AND {shown} GROUP BY a.locality"))? {
            let (name, country) = match label.rsplit_once(", ") {
                Some((n, c)) => (n.to_string(), Some(c.to_string())),
                None => (label.clone(), None),
            };
            // A town of the same name already stands for it.
            let town = PlaceRef::City { name: name.clone(), country: country.clone() };
            if !places.iter().any(|(p, _)| *p == town) {
                add_place(&mut places, PlaceRef::Locality { name, country }, n);
            }
        }
        for (place, n) in places {
            push(place.label(), TagValue::Place { place }, "Place", Some(n), "place location");
        }

        // Manual albums.
        let items = counted(&format!("SELECT ai.album_id, COUNT(*) FROM album_items ai JOIN assets a ON a.id = ai.asset_id WHERE {shown} GROUP BY ai.album_id"))?;
        for (id, name) in named(&format!("SELECT id, name FROM albums WHERE kind = {}", crate::albums::KIND_MANUAL))? {
            push(name, TagValue::Album { id }, "Album", Some(items.get(&id).copied().unwrap_or(0)), "album");
        }
        // Collections count the photos of their albums and of the collections inside them.
        let in_collections = counted(&format!(
            "WITH RECURSIVE sub(root, id) AS (SELECT id, id FROM album_collections UNION SELECT sub.root, c.id FROM album_collections c JOIN sub ON c.parent_id = sub.id)
             SELECT sub.root, COUNT(DISTINCT ai.asset_id) FROM sub JOIN albums al ON al.collection_id = sub.id JOIN album_items ai ON ai.album_id = al.id
             JOIN assets a ON a.id = ai.asset_id WHERE {shown} GROUP BY sub.root",
        ))?;
        for (id, name) in named("SELECT id, name FROM album_collections")? {
            push(name, TagValue::Collection { id }, "Collection", Some(in_collections.get(&id).copied().unwrap_or(0)), "collection collections folder");
        }

        // Scene tags: the distinct tag lists are few, so they are counted here, not per tag in SQL.
        let lists: Vec<(String, i64)> = grouped(&format!("SELECT a.scene_tags, COUNT(*) FROM assets a WHERE a.scene_tags IS NOT NULL AND a.scene_tags != '' AND {shown} GROUP BY a.scene_tags"))?
            .into_iter()
            .map(|(l, n)| (format!(" {l} "), n))
            .collect();
        for tag in crate::embedding::SCENE_TAGS {
            let needle = format!(" {tag} ");
            let n: i64 = lists.iter().filter(|(l, _)| l.contains(&needle)).map(|(_, n)| n).sum();
            if n > 0 {
                push(capitalise(tag), TagValue::Scene { tag: tag.to_string() }, "Scene", Some(n), "scene");
            }
        }

        // Fixed tags, counted in one pass.
        let fixed: Vec<(&str, TagValue, &str, String)> = vec![
            ("Photos", TagValue::Media { media: 0 }, "photos photo pictures images", "a.media_type != 1".into()),
            ("Videos", TagValue::Media { media: 1 }, "videos video movies clips", "a.media_type = 1".into()),
            ("Live Photos", TagValue::Kind { kind: AssetKind::Live }, "live", AssetKind::Live.predicate()),
            ("RAW", TagValue::Kind { kind: AssetKind::Raw }, "raw raws", AssetKind::Raw.predicate()),
            ("HDR", TagValue::Kind { kind: AssetKind::Hdr }, "hdr", AssetKind::Hdr.predicate()),
            ("Screenshots", TagValue::Kind { kind: AssetKind::Screenshot }, "screenshot screenshots", AssetKind::Screenshot.predicate()),
            ("Panoramas", TagValue::Kind { kind: AssetKind::Panorama }, "panorama panoramas pano", AssetKind::Panorama.predicate()),
            ("Selfies", TagValue::Kind { kind: AssetKind::Selfie }, "selfie selfies front", AssetKind::Selfie.predicate()),
            ("No sound", TagValue::Kind { kind: AssetKind::Silent }, "no sound silent mute without audio", AssetKind::Silent.predicate()),
            ("Favorites", TagValue::Favorites, "favorites favourites favorite favourite hearts loved", "a.is_favorite = 1".into()),
            ("Picked", TagValue::Flag { flag: 1 }, "picked pick flag flagged", "a.flag = 1".into()),
            ("Rejected", TagValue::Flag { flag: -1 }, "rejected reject flag", "a.flag = -1".into()),
            ("Has location", TagValue::Location { has: true }, "location gps geotagged map", "a.gps_lat IS NOT NULL".into()),
            ("No location", TagValue::Location { has: false }, "location gps without missing", "a.gps_lat IS NULL".into()),
            ("Has text", TagValue::HasText, "text words written sign signs document documents ocr", crate::query::HAS_TEXT.into()),
            ("No text", TagValue::NoText, "text words none without nothing read", format!("EXISTS (SELECT 1 FROM ocr_scan o WHERE o.asset_id = a.id) AND NOT {}", crate::query::HAS_TEXT)),
            ("5 stars", TagValue::Rating { min: 5 }, "rating rated stars star", "a.rating >= 5".into()),
            ("4 stars and up", TagValue::Rating { min: 4 }, "rating rated stars star", "a.rating >= 4".into()),
            ("3 stars and up", TagValue::Rating { min: 3 }, "rating rated stars star", "a.rating >= 3".into()),
            ("2 stars and up", TagValue::Rating { min: 2 }, "rating rated stars star", "a.rating >= 2".into()),
            ("1 star and up", TagValue::Rating { min: 1 }, "rating rated stars star", "a.rating >= 1".into()),
            ("Edited in Lumen", TagValue::Lumen { mark: LumenMark::Edited }, LUMEN_WORDS, LumenMark::Edited.predicate()),
            ("In Develop", TagValue::Lumen { mark: LumenMark::Developed }, LUMEN_WORDS, LumenMark::Developed.predicate()),
            ("Exported from Lumen", TagValue::Lumen { mark: LumenMark::Exported }, LUMEN_WORDS, LumenMark::Exported.predicate()),
            ("Edited in an app", TagValue::EditedIn { app: None }, EDITED_IN_WORDS, "a.edited_in IS NOT NULL".into()),
            ("Portrait", TagValue::Shape { shape: Shape::Portrait }, "portrait tall vertical orientation shape", Shape::Portrait.predicate()),
            ("Landscape", TagValue::Shape { shape: Shape::Landscape }, "landscape wide horizontal orientation shape", Shape::Landscape.predicate()),
            ("Square", TagValue::Shape { shape: Shape::Square }, "square orientation shape", Shape::Square.predicate()),
        ];
        let mut fixed = fixed;
        for (label, measure, min, max, aliases) in BUCKETS {
            fixed.push((*label, TagValue::Range { measure: *measure, min: *min, max: *max }, *aliases, measure.span_sql((*min, *max))));
        }
        let sums = fixed.iter().map(|(_, _, _, pred)| format!("COALESCE(SUM(CASE WHEN {pred} THEN 1 ELSE 0 END), 0)")).collect::<Vec<_>>().join(", ");
        let counts: Vec<i64> = src.run(|c| Ok(c.query_row(&format!("SELECT {sums} FROM assets a WHERE {shown}"), [], |r| (0..fixed.len()).map(|i| r.get(i)).collect())?))?;
        for ((label, value, aliases, _), n) in fixed.into_iter().zip(counts) {
            let field = match value {
                TagValue::Media { .. } | TagValue::Kind { .. } => "Kind",
                TagValue::Rating { .. } => "Rating",
                TagValue::Flag { .. } => "Flag",
                TagValue::Location { .. } => "Location",
                TagValue::HasText | TagValue::NoText => "Text",
                TagValue::Lumen { .. } => "Made in Lumen",
                TagValue::EditedIn { .. } => "Edited in",
                TagValue::Shape { .. } => "Shape",
                TagValue::Range { measure, .. } => measure_name(measure),
                _ => "",
            };
            push(label.to_string(), value, field, Some(n), aliases);
        }

        // The apps a copy was made for (Round 22 lane C), by the name the app table gives.
        for (id, n) in grouped(&format!("SELECT a.edited_in, COUNT(*) FROM assets a WHERE a.edited_in IS NOT NULL AND {shown} GROUP BY a.edited_in"))? {
            let name = crate::external_apps::app_name(&id).map_or_else(|| "another app".to_string(), str::to_string);
            push(format!("Edited in {name}"), TagValue::EditedIn { app: Some(id) }, "Edited in", Some(n), EDITED_IN_WORDS);
        }

        // File types.
        for (ext, n) in grouped(&format!("SELECT a.extension, COUNT(*) FROM assets a WHERE {shown} AND a.extension IS NOT NULL AND a.extension != '' GROUP BY a.extension"))? {
            push(ext.to_uppercase(), TagValue::Extension { ext }, "File type", Some(n), "file type format extension");
        }

        // Every EXIF field, read in passes of `EXIF_CHUNK` rows (keyset, by id) and grouped here.
        let cols = ExifField::ALL.iter().map(|f| format!("CAST({} AS TEXT)", f.raw_sql())).collect::<Vec<_>>().join(", ");
        // A video's frame rate rides along, grouped here to whole frames per second.
        let fps_at = ExifField::ALL.len() + 1;
        let sql = format!(
            "SELECT a.id, {cols}, CASE WHEN a.media_type = 1 THEN json_extract(a.metadata_json, '$.probe.fps') END FROM assets a WHERE {shown} AND a.metadata_json IS NOT NULL AND a.id > ?1 ORDER BY a.id LIMIT ?2"
        );
        let mut rates: HashMap<i64, i64> = HashMap::new();
        let mut values: Vec<HashMap<String, i64>> = vec![HashMap::new(); ExifField::ALL.len()];
        // Per software, how many of its photos an Apple camera took.
        let mut apple: HashMap<String, (i64, i64)> = HashMap::new();
        let make_at = ExifField::ALL.iter().position(|f| *f == ExifField::Make).unwrap_or(0);
        let software_at = ExifField::ALL.iter().position(|f| *f == ExifField::Software).unwrap_or(0);
        let mut after = 0i64;
        loop {
            type Row = (i64, Vec<Option<String>>, Option<f64>);
            let rows: Vec<Row> = src.run(|c| {
                let mut stmt = c.prepare_cached(&sql)?;
                let rows = stmt.query_map(rusqlite::params![after, EXIF_CHUNK], |r| {
                    Ok((r.get(0)?, (1..=ExifField::ALL.len()).map(|i| r.get(i)).collect::<rusqlite::Result<Vec<Option<String>>>>()?, r.get(fps_at)?))
                })?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })?;
            let Some(last) = rows.last().map(|(id, _, _)| *id) else { break };
            for (_, row, fps) in &rows {
                if let Some(fps) = fps.filter(|f| *f >= 1.0 && *f < 1000.0) {
                    *rates.entry(fps.round() as i64).or_default() += 1;
                }
                let make = row[make_at].as_deref().map(str::trim);
                for (i, slot) in values.iter_mut().enumerate() {
                    let Some(v) = row[i].as_deref().map(str::trim).filter(|v| !v.is_empty()) else { continue };
                    if i == software_at {
                        let e = apple.entry(v.to_string()).or_default();
                        e.1 += 1;
                        if make == Some("Apple") {
                            e.0 += 1;
                        }
                    }
                    *slot.entry(v.to_string()).or_default() += 1;
                }
            }
            if (rows.len() as i64) < EXIF_CHUNK {
                break;
            }
            after = last;
        }
        for (field, found) in ExifField::ALL.iter().zip(values) {
            for (value, n) in found {
                let is_apple = apple.get(&value).is_some_and(|(a, all)| *field == ExifField::Software && a * 2 > *all);
                let label = exif_label(*field, &value, is_apple);
                // A camera writes its firmware where an app writes its name ("Ver.1.03"):
                // that photo was not exported from anything.
                let (name, aliases) = if *field == ExifField::Software && is_firmware(&value) {
                    ("Camera firmware", "camera firmware software version")
                } else {
                    (exif_field_name(*field), exif_field_aliases(*field))
                };
                push(label, TagValue::Exif { field: *field, value }, name, Some(n), aliases);
            }
        }
        for (fps, n) in rates {
            let value = TagValue::Range { measure: Measure::FrameRate, min: Some(fps * 1000 - 500), max: Some(fps * 1000 + 500) };
            push(format!("{fps} fps"), value, "Frame rate", Some(n), "frame rate fps frames per second video");
        }
        // Every date by year (Round 10): "created" alone offers *File created: 2024*. Grouped
        // here, not in SQL, as the dates are computed.
        for field in [DateField::Taken, DateField::FileCreated, DateField::FileModified, DateField::Imported, DateField::Digitized] {
            let expr = field.sql();
            let times: Vec<i64> = src.run(|c| {
                let mut stmt = c.prepare(&format!("SELECT {expr} FROM assets a WHERE {shown} AND {expr} IS NOT NULL"))?;
                let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
                Ok(rows.collect::<std::result::Result<_, _>>()?)
            })?;
            let mut years: BTreeMap<i32, i64> = BTreeMap::new();
            for t in times {
                if let Some(at) = chrono::DateTime::from_timestamp(t, 0) {
                    *years.entry(chrono::Datelike::year(&at)).or_default() += 1;
                }
            }
            let start = |y: i32| NaiveDate::from_ymd_opt(y, 1, 1).and_then(|d| d.and_hms_opt(0, 0, 0)).map(|d| d.and_utc().timestamp());
            for (y, n) in years.into_iter().rev() {
                let (Some(from), Some(next)) = (start(y), start(y + 1)) else { continue };
                let value = TagValue::Dates { from: Some(from), to: Some(next - 1), field: (field != DateField::Taken).then_some(field) };
                push(format!("{}: {y}", date_field_name(field)), value, "Date", Some(n), date_field_aliases(field));
            }
        }
        Ok(Facets { entries })
    }
}

/// What a date tag of `field` is called.
fn date_field_name(field: DateField) -> &'static str {
    match field {
        DateField::Taken => "Taken",
        DateField::Imported => "Imported",
        DateField::FileModified => "File modified",
        DateField::FileCreated => "File created",
        DateField::Digitized => "Digitized",
    }
}

/// The date a word names: "created", "modified", "taken"…
fn date_field_word(word: &str) -> Option<DateField> {
    Some(match word {
        "taken" | "shot" | "captured" | "capture" => DateField::Taken,
        "imported" | "import" | "added" => DateField::Imported,
        "modified" | "changed" | "modification" => DateField::FileModified,
        "created" | "creation" => DateField::FileCreated,
        "digitized" | "digitised" => DateField::Digitized,
        _ => return None,
    })
}

/// The words that name a date field, for its year tags ("dateCreated" folds to "datecreated").
fn date_field_aliases(field: DateField) -> &'static str {
    match field {
        DateField::Taken => "date taken shot captured capture datetaken year",
        DateField::Imported => "date imported import added dateimported dateadded",
        DateField::FileModified => "date file modified changed modification last datemodified lastmodified",
        DateField::FileCreated => "date file created creation datecreated",
        DateField::Digitized => "date digitized digitised datedigitized exif",
    }
}

/// "modified 2024" → (FileModified, "2024"): the date a phrase starts by naming, and the rest.
/// Also "date created 2024", "dateCreated 2024", "creation date 2024", "last modified 2024",
/// and with a colon or equals sign after the name: "taken: 4.6.2026", "Taken : 4 june", "taken=2024".
fn date_field_prefix(input: &str) -> Option<(DateField, &str)> {
    let lower = input.to_lowercase();
    let mut rest = lower.trim_start();
    for lead in ["file ", "date ", "last "] {
        rest = rest.strip_prefix(lead).unwrap_or(rest).trim_start();
    }
    let end = rest.find(|c: char| !c.is_alphanumeric())?;
    let (word, after) = rest.split_at(end);
    // The name ends at a space, a colon or an equals sign: "taken4" names nothing.
    if !after.starts_with(|c: char| c.is_whitespace() || c == ':' || c == '=') {
        return None;
    }
    let after = after.trim_start();
    let mut after = after.strip_prefix([':', '=']).unwrap_or(after);
    let word = word.strip_prefix("date").filter(|w| !w.is_empty()).unwrap_or(word);
    let word = word.strip_suffix("date").filter(|w| !w.is_empty()).unwrap_or(word);
    let field = date_field_word(word)?;
    let trimmed = after.trim_start();
    after = trimmed.strip_prefix("date ").unwrap_or(trimmed);
    let after = after.trim_start();
    let after = after.strip_prefix("in ").or_else(|| after.strip_prefix("on ")).unwrap_or(after).trim();
    // Back to the input's own text, so dates read as typed.
    let at = input.len() - after.len();
    (!after.is_empty()).then(|| (field, input[at..].trim()))
}

/// "text: menu", "Text = “pri mostu”" → (the words, whether they are a phrase): the text read in
/// the photo (Round 18 C2). A colon or equals sign is required, so "text menu" stays a search of
/// the words. An open quote is a phrase still being typed.
fn photo_text_prefix(input: &str) -> Option<(String, bool)> {
    let rest = input.trim_start();
    let end = rest.find(|c: char| !c.is_alphabetic()).unwrap_or(rest.len());
    if !rest[..end].eq_ignore_ascii_case("text") {
        return None;
    }
    let after = rest[end..].trim_start().strip_prefix([':', '='])?.trim();
    let quote = ['"', '“', '”', '„'];
    let (body, exact) = match after.strip_prefix(quote) {
        Some(inner) => (inner.trim_end_matches(quote), true),
        None => (after, false),
    };
    let words = body.split_whitespace().collect::<Vec<_>>().join(" ");
    words.chars().any(char::is_alphanumeric).then_some((words, exact))
}

/// A typed comparison (Round 8A): "longer than 2 min", "> 90 s", "under 30s", "over 50 MB",
/// "at least 12 mp", "under 30 fps". The unit says the measure; a number alone is no
/// comparison. Returns the chip's label and the span `[min, max)`.
pub fn read_comparison(input: &str) -> Option<(String, Measure, Option<i64>, Option<i64>)> {
    // ">=" reads as ">", "<=" as "<": every span is inclusive below and exclusive above.
    let text = input.trim().to_lowercase().replace(">=", " > ").replace("<=", " < ").replace('>', " > ").replace('<', " < ");
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const MORE: [&str; 11] = ["longer than", "more than", "greater than", "bigger than", "larger than", "at least", "over", "above", "from", "min", ">"];
    const LESS: [&str; 9] = ["shorter than", "less than", "smaller than", "at most", "up to", "under", "below", "max", "<"];
    let (more, rest) = MORE
        .iter()
        .find_map(|w| text.strip_prefix(w).filter(|r| r.starts_with(' ') || w.len() == 1).map(|r| (true, r)))
        .or_else(|| LESS.iter().find_map(|w| text.strip_prefix(w).filter(|r| r.starts_with(' ') || w.len() == 1).map(|r| (false, r))))?;
    let rest = rest.trim();
    // The number, then the unit, with or without a space between ("30s", "50 MB").
    let split = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(rest.len());
    let n: f64 = rest[..split].replace(',', ".").parse().ok()?;
    let unit = rest[split..].trim();
    let (measure, scale, shown) = match unit {
        "s" | "sec" | "secs" | "second" | "seconds" => (Measure::Duration, 1_000.0, "s"),
        "m" | "min" | "mins" | "minute" | "minutes" => (Measure::Duration, 60_000.0, "min"),
        "h" | "hr" | "hrs" | "hour" | "hours" => (Measure::Duration, 3_600_000.0, "h"),
        "kb" => (Measure::FileSize, 1e3, "KB"),
        "mb" => (Measure::FileSize, 1e6, "MB"),
        "gb" => (Measure::FileSize, 1e9, "GB"),
        "mp" | "megapixel" | "megapixels" => (Measure::Pixels, 1e6, "MP"),
        "fps" => (Measure::FrameRate, 1e3, "fps"),
        _ => return None,
    };
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    let bound = (n * scale).round() as i64;
    let amount = format!("{} {shown}", number(&n.to_string()));
    let label = match (measure, more) {
        (Measure::Duration, true) => format!("Longer than {amount}"),
        (Measure::Duration, false) => format!("Shorter than {amount}"),
        (_, true) => format!("Over {amount}"),
        (_, false) => format!("Under {amount}"),
    };
    Some(if more { (label, measure, Some(bound), None) } else { (label, measure, None, Some(bound)) })
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// How many suggestions the list shows, and how many of one field.
const LIMIT: usize = 12;
const PER_FIELD: usize = 5;
/// How many years of one date field (Round 15): each date field is capped on its own, so Taken's
/// many years can never push *File created* and the others out of the list.
const PER_DATE_FIELD: usize = 3;
/// How many starters an empty box offers.
const STARTERS: usize = 8;

/// The slot a suggestion counts against: its field, and for a date also which date it is.
fn cap_key(e: &Entry) -> (String, usize) {
    match &e.tag.value {
        TagValue::Dates { field, .. } => (format!("{}/{}", e.field, date_field_name(field.unwrap_or(DateField::Taken))), PER_DATE_FIELD),
        _ => (e.field.clone(), PER_FIELD),
    }
}

/// What an empty, focused box offers, so people find what can be searched (Round 15): the newest
/// year of each date field that has photos, then *Longer than 1 min* when there are videos and
/// one *Made in Lumen* tag when Lumen made any. Field `"Try"`.
fn starters(facets: &Facets) -> Vec<TagSuggestion> {
    let live = || facets.entries.iter().filter(|e| e.count.unwrap_or(0) > 0);
    let mut seen: Vec<Option<DateField>> = Vec::new();
    let mut out: Vec<TagSuggestion> = Vec::new();
    let mut offer = |e: &Entry| out.push(TagSuggestion { tag: e.tag.clone(), field: "Try".into(), count: e.count });
    for e in live() {
        if let TagValue::Dates { field, .. } = e.tag.value {
            if e.field == "Date" && !seen.contains(&field) {
                seen.push(field);
                offer(e);
            }
        }
    }
    if let Some(e) = live().find(|e| e.tag.value == TagValue::HasText) {
        offer(e);
    }
    if live().any(|e| matches!(e.tag.value, TagValue::Range { measure: Measure::Duration, .. })) {
        if let Some((label, measure, min, max)) = read_comparison("> 1 min") {
            out.push(TagSuggestion { tag: SearchTag { label, value: TagValue::Range { measure, min, max } }, field: "Try".into(), count: None });
        }
    }
    if let Some(e) = live().find(|e| matches!(e.tag.value, TagValue::Lumen { .. })) {
        out.push(TagSuggestion { tag: e.tag.clone(), field: "Try".into(), count: e.count });
    }
    out.truncate(STARTERS);
    out
}

/// How well a typed word matches an entry: a whole word of the label, the start of one, a
/// whole word naming the field, the start of one. `None` when it matches nothing.
fn word_score(word: &str, e: &Entry) -> Option<u8> {
    if e.words.iter().any(|w| w == word) {
        Some(0)
    } else if e.words.iter().any(|w| w.starts_with(word)) {
        Some(1)
    } else if e.aliases.iter().any(|w| w == word) {
        Some(2)
    } else if e.aliases.iter().any(|w| w.starts_with(word)) && word.chars().count() >= 2 {
        Some(3)
    } else {
        None
    }
}

/// The tags `input` may mean, best first, grouped by field, the text itself last; an empty
/// `input` gets the [`starters`]. `today`
/// reads dates ("2023", "last summer"), and `order` which way round `4.6.2026` is.
pub fn suggest(facets: &Facets, input: &str, today: NaiveDate, order: DateOrder) -> Vec<TagSuggestion> {
    candidates(facets, input, today, order).into_iter().map(|c| c.suggestion).collect()
}

/// One of [`suggest`]'s suggestions, and whether it is surely what the words mean (Round 20 §2):
/// a typed sentence turns a sure one into its tag without asking.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub suggestion: TagSuggestion,
    pub sure: bool,
}

/// [`suggest`]'s list, each marked [`Candidate::sure`] when it is a tag a reader made of the
/// whole input (a date, a comparison, `text:`), a facet whose label is the typed words, or the
/// only facet (or only fixed one) every typed word is a whole word of; failing any of those, the
/// only one (or only fixed one) every typed word is a whole word or other name of
/// (`screenshot` → *Screenshots*). Never the text itself.
pub fn candidates(facets: &Facets, input: &str, today: NaiveDate, order: DateOrder) -> Vec<Candidate> {
    let made = |tag: SearchTag, field: &str| Candidate { suggestion: TagSuggestion { tag, field: field.into(), count: None }, sure: true };
    let input = input.trim();
    if input.is_empty() {
        return starters(facets).into_iter().map(|suggestion| Candidate { suggestion, sure: false }).collect();
    }
    // "text:" alone names the field: it lists its tags, as "text" does.
    let input = if input.trim_end_matches([':', '=', ' ']).eq_ignore_ascii_case("text") { "text" } else { input };
    let words = name_words(input);
    let mut out: Vec<Candidate> = Vec::new();
    if let Some((text, exact)) = photo_text_prefix(input) {
        let label = if exact { format!("Text: “{text}”") } else { format!("Text: {text}") };
        return vec![made(SearchTag { label, value: TagValue::PhotoText { text, exact } }, "Text")];
    }
    if let Some((label, span)) = read_date(input, today, 0, order) {
        let value = match span {
            DateSpan::Range(from, to) => TagValue::Dates { from: Some(from), to: Some(to), field: None },
            DateSpan::Months(months) => TagValue::Months { months },
        };
        out.push(made(SearchTag { label, value }, "Date"));
    } else if let Some((field, rest)) = date_field_prefix(input) {
        // "modified 2024", "imported last week", "taken: before 4.6.2026": a span of that date.
        if let Some((label, from, to)) = read_date_expr(rest, today, 0, order) {
            let name = date_field_name(field);
            let field = (field != DateField::Taken).then_some(field);
            out.push(made(SearchTag { label: format!("{name}: {label}"), value: TagValue::Dates { from, to, field } }, "Date"));
        }
    } else if let Some((label, from, to)) = read_date_expr(input, today, 0, order) {
        // "before 2024", "1.6.2026 – 4.6.2026": when it was taken.
        out.push(made(SearchTag { label, value: TagValue::Dates { from, to, field: None } }, "Date"));
    }
    if let Some((label, measure, min, max)) = read_comparison(input) {
        out.push(made(SearchTag { label, value: TagValue::Range { measure, min, max } }, measure_name(measure)));
    }
    if !words.is_empty() {
        // Typed punctuation ("f/1", "1/100") is a phrase: the label must hold it at a word's start.
        let phrase = input.contains(|c: char| !c.is_alphanumeric() && !c.is_whitespace()).then(|| fold(input));
        let mut scored: Vec<(u32, bool, i64, &Entry)> = facets
            .entries
            .iter()
            .filter(|e| e.fixed || e.count.unwrap_or(0) > 0)
            .filter(|e| {
                phrase.as_deref().is_none_or(|p| e.folded.match_indices(p).any(|(i, _)| i == 0 || !e.folded[..i].ends_with(char::is_alphanumeric)))
            })
            .filter_map(|e| {
                let scores: Vec<u8> = words.iter().map(|w| word_score(w, e)).collect::<Option<_>>()?;
                // At least one word must be about the value or name the field outright.
                let total: u32 = scores.iter().map(|s| *s as u32).sum();
                let whole = fold(&e.tag.label) == words.join(" ");
                Some((total, !whole, e.count.unwrap_or(0), e))
            })
            .collect();
        scored.sort_by(|a, b| (a.1, a.0, std::cmp::Reverse(a.2)).cmp(&(b.1, b.0, std::cmp::Reverse(b.2))).then_with(|| a.3.tag.label.cmp(&b.3.tag.label)));
        // The one entry the words surely mean, when there is one.
        let only = |set: Vec<&Entry>| -> Option<*const Entry> {
            let fixed: Vec<&Entry> = set.iter().copied().filter(|e| e.fixed).collect();
            match (set.as_slice(), fixed.as_slice()) {
                ([one], _) | (_, [one]) => Some(*one as *const Entry),
                _ => None,
            }
        };
        let zero: Vec<&Entry> = scored.iter().filter(|s| s.0 == 0).map(|s| s.3).collect();
        let sure = if zero.is_empty() {
            only(scored.iter().map(|s| s.3).filter(|e| words.iter().all(|w| e.words.contains(w) || e.aliases.contains(w))).collect())
        } else {
            only(zero)
        };
        let mut per_field: HashMap<String, usize> = HashMap::new();
        for (_, not_whole, _, e) in scored {
            if out.len() >= LIMIT {
                break;
            }
            // The tag the typed date already made ("created 2024") is offered once.
            if out.iter().any(|s| s.suggestion.tag.value == e.tag.value) {
                continue;
            }
            let (key, cap) = cap_key(e);
            let n = per_field.entry(key).or_default();
            if *n >= cap {
                continue;
            }
            *n += 1;
            let suggestion = TagSuggestion { tag: e.tag.clone(), field: e.field.clone(), count: e.count };
            out.push(Candidate { suggestion, sure: !not_whole || sure == Some(e as *const Entry) });
        }
        // Grouped by field, in the order each field first appears.
        let mut order: Vec<String> = Vec::new();
        for s in &out {
            if !order.contains(&s.suggestion.field) {
                order.push(s.suggestion.field.clone());
            }
        }
        out.sort_by_key(|s| order.iter().position(|f| *f == s.suggestion.field));
    }
    let text = SearchTag { label: input.to_string(), value: TagValue::Text { text: input.to_string() } };
    out.push(Candidate { suggestion: TagSuggestion { tag: text, field: "Text".into(), count: None }, sure: false });
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::db::Db;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
    }

    /// Anna in 1 and 2; Lisbon 1–2, Porto 3; iPhone 12 on 1–3, a Nikon on 4 exported from
    /// Lightroom, and on 7 with its firmware; 5 is a video; 6 is in the trash.
    pub(crate) fn library(c: &Connection) -> Result<()> {
        let rows = [
            (1, 0, "heic", Some("Lisbon, Portugal"), r#"{"make":"Apple","model":"iPhone 12","iso":32,"aperture":1.6,"software":"18.5","flash":"Did not fire"}"#, "beach sunset"),
            (2, 0, "heic", Some("Lisbon, Portugal"), r#"{"make":"Apple","model":"iPhone 12","iso":100,"software":"18.5"}"#, "beach"),
            (3, 0, "jpg", Some("Porto, Portugal"), r#"{"make":"Apple","model":"iPhone 12","iso":100}"#, "dog"),
            (4, 0, "nef", None, r#"{"make":"NIKON CORPORATION","model":"NIKON D7100","iso":1000,"software":"Adobe Photoshop Lightroom Classic 13.2 (Windows)","flash":"Fired"}"#, ""),
            (7, 0, "jpg", None, r#"{"make":"NIKON CORPORATION","model":"NIKON D7100","software":"Ver.1.03"}"#, ""),
            (5, 1, "mov", None, r#"{"model":"iPhone 12"}"#, ""),
        ];
        for (id, media, ext, place, meta, tags) in rows {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_name, metadata_json, scene_tags, rating, is_favorite)
                 VALUES (?1, ?2, 'p', 'f', ?3, ?4, 1, 1, 10, ?5, ?6, ?7, MIN(?1, 5), ?1 % 2)",
                rusqlite::params![id, format!("h{id}"), ext, media, place, meta, tags],
            )?;
        }
        c.execute_batch(
            r#"INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, state, metadata_json)
                 VALUES (6, 'h6', 'p', 'f', 'png', 0, 1, 1, 10, 1, '{"model":"Trashed Cam"}');
               INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, in_vault, place_name, metadata_json)
                 VALUES (8, 'h8', 'p', 'f', 'jpg', 0, 1, 1, 10, 1, 'Secret Cove, Portugal', '{"model":"Vault Cam"}');
               INSERT INTO people (id, name, created_at) VALUES (1, 'Anna Berg', 0);
               INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) VALUES (1, 0, 0, .1, .1, .9, x'00', 'm', 1), (2, 0, 0, .1, .1, .9, x'00', 'm', 1);
               INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (9, 'Trip to Porto', 0, 0, 0);
               INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (9, 3, 1, 0);"#,
        )?;
        Ok(())
    }

    fn labels(s: &[TagSuggestion]) -> Vec<String> {
        s.iter().map(|s| format!("{}: {}", s.field, s.tag.label)).collect()
    }

    fn ids(c: &Connection, tags: &[SearchTag]) -> Vec<i64> {
        let scope = crate::scope::LayoutScope { query: Some(to_query(tags)), ..Default::default() };
        let mut ids = crate::layout::ids_scoped(c, &scope, 100).unwrap();
        ids.sort_unstable();
        ids
    }

    fn pick(s: &[TagSuggestion], label: &str) -> SearchTag {
        s.iter().find(|s| s.tag.label == label).unwrap_or_else(|| panic!("{label} in {:?}", labels(s))).tag.clone()
    }

    #[test]
    fn a_collection_is_suggested_counted_and_searched_with_its_nested_albums() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            c.execute_batch(
                "INSERT INTO album_collections (id, name, parent_id, created_at, updated_at) VALUES (1, 'Europe', NULL, 0, 0), (2, 'Portugal trips', 1, 0, 0);
                 UPDATE albums SET collection_id = 2 WHERE id = 9;",
            )?;
            let f = Facets::load(c)?;
            let s = suggest(&f, "europ", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Collection: Europe");
            assert_eq!(s[0].count, Some(1), "the photo of the album one collection down");
            let tag = pick(&s, "Europe");
            assert_eq!(tag.value, TagValue::Collection { id: 1 });
            assert_eq!(ids(c, &[tag]), vec![3]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn suggests_what_the_library_knows_and_the_text_last() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            let f = Facets::load(c)?;
            let s = suggest(&f, "ann", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Person: Anna Berg");
            assert_eq!(s[0].count, Some(2));
            assert_eq!(s.last().unwrap().tag.value, TagValue::Text { text: "ann".into() }, "the text is always offered");
            let s = suggest(&f, "lis", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Place: Lisbon, Portugal");
            let s = suggest(&f, "portugal", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Place: Portugal", "the whole name first");
            assert_eq!(s[0].count, Some(3));
            // Naming a field lists its values.
            let s = suggest(&f, "exported", today(), DateOrder::Dmy);
            let l = labels(&s);
            assert!(l.contains(&"Exported from: iOS 18.5".to_string()), "{l:?}");
            assert!(l.contains(&"Exported from: Adobe Photoshop Lightroom Classic 13.2 (Windows)".to_string()), "{l:?}");
            assert!(!l.iter().any(|l| l.contains("Ver.1.03")), "a camera's firmware is not an export: {l:?}");
            assert_eq!(labels(&suggest(&f, "firmware", today(), DateOrder::Dmy))[0], "Camera firmware: Ver.1.03");
            let s = suggest(&f, "lightroom", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Exported from: Adobe Photoshop Lightroom Classic 13.2 (Windows)");
            let s = suggest(&f, "iso 100", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "ISO: ISO 100");
            assert_eq!(s[0].count, Some(2));
            assert!(labels(&suggest(&f, "video", today(), DateOrder::Dmy)).contains(&"Kind: Videos".to_string()));
            assert!(labels(&suggest(&f, "stars", today(), DateOrder::Dmy)).contains(&"Rating: 3 stars and up".to_string()));
            assert!(labels(&suggest(&f, "trip", today(), DateOrder::Dmy)).contains(&"Album: Trip to Porto".to_string()));
            assert!(labels(&suggest(&f, "sun", today(), DateOrder::Dmy)).contains(&"Scene: Sunset".to_string()));
            assert!(labels(&suggest(&f, "nef", today(), DateOrder::Dmy)).contains(&"File type: NEF".to_string()));
            // Punctuation is a phrase: "f/1" is an aperture, not a word starting with f.
            assert_eq!(labels(&suggest(&f, "f/1", today(), DateOrder::Dmy)), vec!["Aperture: f/1.6", "Text: f/1"]);
            // Nothing is picked yet, but the tag is still the way to look.
            assert!(labels(&suggest(&f, "picked", today(), DateOrder::Dmy)).contains(&"Flag: Picked".to_string()));
            // The trash counts for nothing.
            assert!(!labels(&suggest(&f, "trashed", today(), DateOrder::Dmy)).iter().any(|l| l.contains("Trashed Cam")));
            // Nor does the Hidden folder: its places and cameras are never suggested.
            assert!(!labels(&suggest(&f, "secret", today(), DateOrder::Dmy)).iter().any(|l| l.contains("Secret Cove")));
            assert!(!labels(&suggest(&f, "vault", today(), DateOrder::Dmy)).iter().any(|l| l.contains("Vault Cam")));
            // Dates read as dates.
            let s = suggest(&f, "march 2024", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Date: March 2024");
            // Round 15: a blank box offers starters only, never the blank text.
            assert!(suggest(&f, "  ", today(), DateOrder::Dmy).iter().all(|s| s.field == "Try"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn has_text_tag_is_offered_and_applies() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            c.execute("INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (2, 0, 'Gostilna', 0.9, x'00', 'Latn')", [])?;
            let f = Facets::load(c)?;
            let s = suggest(&f, "text", today(), DateOrder::Dmy);
            let has = s.iter().find(|s| s.tag.value == TagValue::HasText).expect("Has text offered");
            assert_eq!((has.field.as_str(), has.tag.label.as_str(), has.count), ("Text", "Has text", Some(1)));
            assert!(labels(&suggest(&f, "sign", today(), DateOrder::Dmy)).contains(&"Text: Has text".to_string()));
            assert_eq!(ids(c, std::slice::from_ref(&has.tag)), vec![2]);
            assert!(serde_json::to_string(&has.tag)?.contains(r#""type":"hasText""#));
            Ok(())
        })
        .unwrap();
    }

    /// Round 18 C2: `Text: menu`, `Text: "exact phrase"`, *No text*, and the *Try* starter.
    #[test]
    fn text_token_reads_words_and_phrases_and_applies() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            // 1 and 2 have text, 3 was read and has none, the rest were never read. Photo 1's
            // keywords say "dog" and photo 3's too: only the text read counts for `Text:`.
            for (id, lines) in [(1, vec!["Gostilna Pri Mostu", "Dnevni menu"]), (2, vec!["Menu", "Juha"])] {
                c.execute("INSERT INTO ocr_scan (asset_id, model) VALUES (?1, 'm')", [id])?;
                for (ord, t) in lines.iter().enumerate() {
                    c.execute("INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (?1, ?2, ?3, 0.9, x'00', 'Latn')", rusqlite::params![id, ord as i64, t])?;
                }
                crate::fts::index_ids(c, &[id])?;
            }
            c.execute("INSERT INTO ocr_scan (asset_id, model) VALUES (3, 'm')", [])?;
            let f = Facets::load(c)?;
            let one = |input: &str| suggest(&f, input, today(), DateOrder::Dmy);

            let s = one("Text: menu");
            assert_eq!(labels(&s), vec!["Text: Text: menu"], "only the token, not a search of everything");
            assert_eq!(s[0].tag.value, TagValue::PhotoText { text: "menu".into(), exact: false });
            assert_eq!(ids(c, &[s[0].tag.clone()]), vec![1, 2]);
            assert_eq!(one("text=menu")[0].tag.value, TagValue::PhotoText { text: "menu".into(), exact: false });
            assert_eq!(one("  TEXT :  menu juha ")[0].tag.value, TagValue::PhotoText { text: "menu juha".into(), exact: false });
            let phrase = one("text: \"pri mostu\"");
            assert_eq!(phrase[0].tag.value, TagValue::PhotoText { text: "pri mostu".into(), exact: true });
            assert_eq!(phrase[0].tag.label, "Text: “pri mostu”");
            assert_eq!(ids(c, &[phrase[0].tag.clone()]), vec![1]);
            assert_eq!(one("text: \"pri mo")[0].tag.value, TagValue::PhotoText { text: "pri mo".into(), exact: true }, "an open quote is a phrase being typed");
            // Words that are not a token stay what they were.
            assert!(!labels(&one("text menu")).iter().any(|l| l.starts_with("Text: Text")));
            assert!(!labels(&one("context: x")).iter().any(|l| l.contains("Text: x")));
            // A name with nothing after it offers the tags that name it.
            let l = labels(&one("text:"));
            assert!(l.contains(&"Text: Has text".to_string()) && l.contains(&"Text: No text".to_string()), "{l:?}");

            // Combines with the other tags (AND), here a date and a second Text.
            let menu = one("text: menu")[0].tag.clone();
            let juha = one("text: juha")[0].tag.clone();
            assert_eq!(ids(c, &[menu.clone(), juha]), vec![2]);
            let porto = pick(&suggest(&f, "porto", today(), DateOrder::Dmy), "Porto, Portugal");
            assert_eq!(ids(c, &[menu, porto]), Vec::<i64>::new());

            // No text: read, and nothing found.
            let no = pick(&one("no text"), "No text");
            assert_eq!(ids(c, std::slice::from_ref(&no)), vec![3]);
            assert!(serde_json::to_string(&no)?.contains(r#""type":"noText""#));
            let has = pick(&one("has text"), "Has text");
            assert_eq!(ids(c, &[has]), vec![1, 2]);

            // The empty box offers Has text beside the dates.
            let starters = one("");
            assert!(starters.iter().any(|s| s.tag.value == TagValue::HasText && s.field == "Try"), "{:?}", labels(&starters));
            assert!(serde_json::to_string(&phrase[0].tag)?.contains(r#""type":"photoText""#));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_and_between_fields_and_or_within_one() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            library(c)?;
            let f = Facets::load(c)?;
            let tag = |input: &str, label: &str| pick(&suggest(&f, input, today(), DateOrder::Dmy), label);
            let photos = tag("photos", "Photos");
            let videos = tag("videos", "Videos");
            assert_eq!(ids(c, std::slice::from_ref(&photos)), vec![1, 2, 3, 4, 7]);
            assert_eq!(ids(c, &[photos.clone(), videos]), vec![1, 2, 3, 4, 5, 7], "Photos or Videos");
            let lisbon = tag("lisbon", "Lisbon, Portugal");
            let porto = tag("porto", "Porto, Portugal");
            assert_eq!(ids(c, &[lisbon.clone(), porto]), vec![1, 2, 3], "either place");
            let iso100 = tag("iso 100", "ISO 100");
            assert_eq!(ids(c, &[lisbon.clone(), iso100.clone()]), vec![2], "a place and an ISO");
            let iso32 = tag("iso 32", "ISO 32");
            assert_eq!(ids(c, &[iso32, iso100]), vec![1, 2, 3], "two ISOs");
            let anna = tag("anna", "Anna Berg");
            assert_eq!(ids(c, &[anna.clone(), tag("beach", "Beach")]), vec![1, 2]);
            assert_eq!(ids(c, &[anna, tag("sunset", "Sunset")]), vec![1]);
            assert_eq!(ids(c, &[tag("lightroom", "Adobe Photoshop Lightroom Classic 13.2 (Windows)")]), vec![4]);
            assert_eq!(ids(c, &[tag("iOS", "iOS 18.5")]), vec![1, 2]);
            assert_eq!(ids(c, &[tag("3 stars", "3 stars and up")]), vec![3, 4, 5, 7]);
            assert_eq!(ids(c, &[tag("fav", "Favorites")]), vec![1, 3, 5, 7]);
            assert_eq!(ids(c, &[tag("trip", "Trip to Porto")]), vec![3]);
            assert_eq!(ids(c, &[tag("flash", "Fired")]), vec![4]);
            // Tags carried in a page's query narrow that page: an album's scope, then the tags.
            let lisbon_q = AssetQuery { search_tags: vec![lisbon.clone()], favorites: true, ..Default::default() };
            let scope = crate::scope::LayoutScope { query: Some(lisbon_q), ..Default::default() };
            assert_eq!(crate::layout::ids_scoped(c, &scope, 100)?, vec![1], "Lisbon and a favourite");
            let months = |m: Vec<u8>| SearchTag { label: "m".into(), value: TagValue::Months { months: m } };
            let base = AssetQuery { months: vec![3, 4], ..Default::default() };
            assert_eq!(merge(&base, &to_query(&[months(vec![4, 5])])).months, vec![4], "both month sets must hold");
            assert!(!merge(&base, &to_query(&[months(vec![7])])).any_of.is_empty(), "no shared month matches nothing");
            // The text tag searches as the field always has (here, file names: none match).
            let text = SearchTag { label: "zzz".into(), value: TagValue::Text { text: "zzz".into() } };
            assert_eq!(to_query(&[text]).text.as_deref(), Some("zzz"));
            Ok(())
        })
        .unwrap();
    }

    /// Round 8A's library: 1 a 12 MP portrait photo edited and in Develop, 2 a Lumen export,
    /// 3 a square photo up a mountain, 4 a 4K HEVC video of 8 s at 30 fps, 5 a 1080p H.264
    /// video of 95 s at 60 fps, 6 a 70 MB video of 7 min.
    fn made_and_measured(c: &Connection) -> Result<()> {
        let day = |d: u32| NaiveDate::from_ymd_opt(2024, 3, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        #[allow(clippy::type_complexity)]
        let rows: [(i64, i64, i64, i64, i64, i64, Option<i64>, Option<f64>, &str); 6] = [
            (1, 0, 3024, 4032, 1, 4_000_000, None, None, r#"{"make":"Apple","body_serial":"F2LX"}"#),
            (2, 0, 2000, 1500, 1, 900_000, None, None, r#"{"software":"Lumen"}"#),
            (3, 0, 1000, 1000, 1, 2_000_000, None, Some(3_200.0), r#"{}"#),
            (4, 1, 3840, 2160, 1, 30_000_000, Some(8_000), None, r#"{"probe":{"video_codec":"hevc","fps":29.97}}"#),
            (5, 1, 1080, 1920, 1, 40_000_000, Some(95_000), None, r#"{"probe":{"video_codec":"h264","fps":60}}"#),
            (6, 1, 1280, 720, 1, 70_000_000, Some(420_000), Some(-12.0), r#"{"probe":{"video_codec":"h264","fps":30}}"#),
        ];
        for (id, media, w, h, orientation, bytes, duration, altitude, meta) in rows {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, width, height, orientation, byte_size, duration_ms,
                   gps_altitude, metadata_json, captured_at, imported_at, file_modified_at, file_created_at)
                 VALUES (?1, ?2, 'p', 'f', 'jpg', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                rusqlite::params![id, format!("h{id}"), media, w, h, orientation, bytes, duration, altitude, meta, day(1), day(10 + id as u32), day(id as u32), (id == 3).then(|| day(20))],
            )?;
        }
        c.execute_batch(
            "UPDATE assets SET orientation = 6 WHERE id = 2; -- 2000×1500 turned a quarter: portrait as shown
             INSERT INTO edits (asset_id, recipe_json, updated_at) VALUES (1, '{}', 0);
             INSERT INTO develop_drafts (asset_id, recipe_json, version, updated_at) VALUES (1, '{}', 1, 0);
             INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, derived_from)
               VALUES (7, 'h7', 'p', 'f', 'jpg', 0, 1, 1, 10, 3);",
        )?;
        Ok(())
    }

    #[test]
    fn a_copy_made_for_an_editor_is_a_tag() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            c.execute_batch("UPDATE assets SET edited_in = 'photoshop' WHERE id = 3; UPDATE assets SET edited_in = 'my-custom-app' WHERE id = 2;")?;
            let f = Facets::load(c)?;
            let l = labels(&suggest(&f, "edited in", today(), DateOrder::Dmy));
            for want in ["Edited in: Edited in an app", "Edited in: Edited in Adobe Photoshop", "Edited in: Edited in another app"] {
                assert!(l.contains(&want.to_string()), "{want} in {l:?}");
            }
            let tag = |input: &str, label: &str| pick(&suggest(&f, input, today(), DateOrder::Dmy), label);
            assert_eq!(ids(c, &[tag("edited in an", "Edited in an app")]), vec![2, 3]);
            assert_eq!(ids(c, &[tag("photoshop", "Edited in Adobe Photoshop")]), vec![3]);
            assert_eq!(ids(c, &[tag("edited in an", "Edited in an app"), tag("photoshop", "Edited in Adobe Photoshop")]), vec![3], "any app and Photoshop: Photoshop's");
            let json = serde_json::to_string(&TagValue::EditedIn { app: None }).unwrap();
            assert_eq!(json, r#"{"type":"editedIn"}"#);
            assert_eq!(serde_json::from_str::<TagValue>(r#"{"type":"editedIn","app":"gimp"}"#).unwrap(), TagValue::EditedIn { app: Some("gimp".into()) });
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn what_lumen_made_is_a_tag() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            let f = Facets::load(c)?;
            let l = labels(&suggest(&f, "lumen", today(), DateOrder::Dmy));
            for want in ["Made in Lumen: Edited in Lumen", "Made in Lumen: In Develop", "Made in Lumen: Exported from Lumen"] {
                assert!(l.contains(&want.to_string()), "{want} in {l:?}");
            }
            for word in ["developed", "edited", "exported"] {
                assert!(labels(&suggest(&f, word, today(), DateOrder::Dmy)).iter().any(|l| l.starts_with("Made in Lumen")), "{word}");
            }
            let tag = |input: &str, label: &str| pick(&suggest(&f, input, today(), DateOrder::Dmy), label);
            assert_eq!(ids(c, &[tag("lumen", "Edited in Lumen")]), vec![1]);
            assert_eq!(ids(c, &[tag("lumen", "In Develop")]), vec![1]);
            assert_eq!(ids(c, &[tag("lumen", "Exported from Lumen")]), vec![2, 7], "Software=Lumen, or made from another photo");
            assert_eq!(ids(c, &[tag("lumen", "Edited in Lumen"), tag("lumen", "Exported from Lumen")]), Vec::<i64>::new(), "both must hold");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn length_size_resolution_shape_and_height_are_tags() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            let f = Facets::load(c)?;
            let tag = |input: &str, label: &str| pick(&suggest(&f, input, today(), DateOrder::Dmy), label);
            let count = |input: &str, label: &str| suggest(&f, input, today(), DateOrder::Dmy).into_iter().find(|s| s.tag.label == label).and_then(|s| s.count);
            // Video length: buckets, OR within.
            assert_eq!(ids(c, &[tag("length", "Under 10 s")]), vec![4]);
            assert_eq!(count("length", "1–5 min"), Some(1));
            assert_eq!(ids(c, &[tag("length", "Under 10 s"), tag("length", "Over 5 min")]), vec![4, 6], "either length");
            assert!(!labels(&suggest(&f, "length", today(), DateOrder::Dmy)).iter().any(|l| l.contains("10–30 s")), "no video that long, no tag");
            // Typed comparisons.
            for (input, label, want) in [
                ("longer than 2 min", "Longer than 2 min", vec![6]),
                ("> 90 s", "Longer than 90 s", vec![5, 6]),
                ("under 30s", "Shorter than 30 s", vec![4]),
                ("over 50 MB", "Over 50 MB", vec![6]),
                ("at least 12 mp", "Over 12 MP", vec![1]),
                ("under 45 fps", "Under 45 fps", vec![4, 6]),
            ] {
                let s = suggest(&f, input, today(), DateOrder::Dmy);
                assert_eq!(ids(c, &[pick(&s, label)]), want, "{input}");
            }
            assert!(read_comparison("2 min").is_none(), "a number alone is no comparison");
            assert!(read_comparison("over 3 apples").is_none());
            // Size, megapixels, video resolution, frame rate, codec.
            assert_eq!(ids(c, &[tag("size", "Over 50 MB")]), vec![6]);
            assert_eq!(ids(c, &[tag("mp", "12 MP and up")]), vec![1]);
            assert_eq!(ids(c, &[tag("4k", "4K video")]), vec![4]);
            assert_eq!(ids(c, &[tag("1080p", "1080p video")]), vec![5], "a portrait video's long edge");
            assert_eq!(ids(c, &[tag("fps", "30 fps")]), vec![4, 6], "29.97 is 30");
            assert_eq!(ids(c, &[tag("hevc", "HEVC (H.265)")]), vec![4]);
            assert_eq!(ids(c, &[tag("codec", "H.264")]), vec![5, 6]);
            assert_eq!(ids(c, &[tag("serial", "Serial F2LX")]), vec![1]);
            // Shape as shown: 2 is turned a quarter.
            assert_eq!(ids(c, &[tag("portrait", "Portrait")]), vec![1, 2, 5]);
            assert_eq!(ids(c, &[tag("square", "Square")]), vec![3]);
            assert_eq!(ids(c, &[tag("landscape", "Landscape")]), vec![4, 6]);
            // Altitude.
            assert_eq!(ids(c, &[tag("altitude", "Above 3,000 m")]), vec![3]);
            assert_eq!(ids(c, &[tag("altitude", "Below sea level")]), vec![6]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn every_date_can_be_searched() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            let f = Facets::load(c)?;
            let s = suggest(&f, "modified march 2024", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Date: File modified: March 2024");
            let modified = pick(&s, "File modified: March 2024");
            assert_eq!(ids(c, std::slice::from_ref(&modified)), vec![1, 2, 3, 4, 5, 6], "modified on March 1–6");
            let imported = pick(&suggest(&f, "imported in march 2024", today(), DateOrder::Dmy), "Imported: March 2024");
            assert_eq!(ids(c, &[imported]).len(), 6);
            let created = pick(&suggest(&f, "file created march 2024", today(), DateOrder::Dmy), "File created: March 2024");
            assert_eq!(ids(c, &[created]), vec![3], "only 3 knows when its file was made");
            let taken = pick(&suggest(&f, "taken 2023", today(), DateOrder::Dmy), "Taken: 2023");
            assert!(matches!(taken.value, TagValue::Dates { field: None, .. }), "taken is the timeline's date");
            assert!(ids(c, &[taken]).is_empty());
            // A plain date is still when it was taken, and its JSON has no field.
            let plain = pick(&suggest(&f, "2024", today(), DateOrder::Dmy), "2024");
            assert!(!serde_json::to_string(&plain).unwrap().contains("field"));
            // Two dates of one kind are either; of two kinds, both.
            let q = to_query(&[modified.clone(), pick(&suggest(&f, "modified 2023", today(), DateOrder::Dmy), "File modified: 2023")]);
            assert_eq!(q.any_of.len(), 1);
            let q = to_query(&[modified, plain]);
            assert_eq!(q.any_of.len(), 2);
            Ok(())
        })
        .unwrap();
    }

    /// `made_and_measured`, plus two photos taken (and file-modified) in each of six more years:
    /// Taken and File modified then have more years, with more photos each, than the per-field
    /// cap, as in a real library where File created has few.
    fn many_modified_years(c: &Connection) -> Result<()> {
        made_and_measured(c)?;
        for (i, year) in (2015..2021).flat_map(|y| [y, y]).enumerate() {
            let at = NaiveDate::from_ymd_opt(year, 6, 1).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, file_modified_at)
                 VALUES (?1, ?2, 'p', 'f', 'jpg', 0, 1, 1, 10, ?3, ?3)",
                rusqlite::params![100 + i as i64, format!("m{i}"), at],
            )?;
        }
        Ok(())
    }

    #[test]
    fn a_bare_year_offers_every_date_field_that_has_it() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            let f = Facets::load(c)?;
            let s = labels(&suggest(&f, "2024", today(), DateOrder::Dmy));
            assert!(s.contains(&"Date: 2024".to_string()), "the taken year, read from the text: {s:?}");
            for want in ["Date: File created: 2024", "Date: File modified: 2024"] {
                assert!(s.contains(&want.to_string()), "{want}: {s:?}");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_first_letters_of_a_field_offer_it() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            many_modified_years(c)?;
            let f = Facets::load(c)?;
            for (typed, want) in [
                ("fi", "Date: File created: "),
                ("fi", "Date: File modified: "),
                ("file", "Date: File created: "),
                ("cre", "Date: File created: "),
                ("mod", "Date: File modified: "),
                ("imp", "Date: Imported: "),
                // Every date field answers to "date": Taken's many years must not take every slot.
                ("date", "Date: File created: "),
                ("date", "Date: Imported: "),
                ("dat", "Date: File created: "),
            ] {
                let s = labels(&suggest(&f, typed, today(), DateOrder::Dmy));
                assert!(s.iter().any(|l| l.starts_with(want)), "{typed} → {want}…: {s:?}");
            }
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_empty_box_offers_starters() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            many_modified_years(c)?;
            let f = Facets::load(c)?;
            let s = suggest(&f, "", today(), DateOrder::Dmy);
            let l = labels(&s);
            assert!(l.contains(&"Try: File created: 2024".to_string()), "{l:?}");
            assert!(l.contains(&"Try: File modified: 2024".to_string()), "the newest year of each field: {l:?}");
            assert!(!l.iter().any(|x| x == "Try: File modified: 2020"), "one year per field: {l:?}");
            assert!(s.iter().any(|x| matches!(x.tag.value, TagValue::Lumen { .. })), "{l:?}");
            assert!(l.contains(&"Try: Longer than 1 min".to_string()), "{l:?}");
            assert!(s.iter().all(|x| x.field == "Try"));
            assert!(!s.iter().any(|x| matches!(x.tag.value, TagValue::Text { .. })), "no typed text when nothing is typed");
            assert!(s.len() <= 8);
            assert!(suggest(&Facets::default(), "  ", today(), DateOrder::Dmy).is_empty(), "an empty library offers nothing");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_date_named_alone_offers_its_years_and_days_are_read() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            made_and_measured(c)?;
            let f = Facets::load(c)?;
            for typed in ["created", "date created", "dateCreated", "DateCreated", "file created", "creation date"] {
                let s = suggest(&f, typed, today(), DateOrder::Dmy);
                assert!(labels(&s).contains(&"Date: File created: 2024".to_string()), "{typed}: {:?}", labels(&s));
            }
            for typed in ["modified", "date modified", "dateModified", "last modified"] {
                assert!(labels(&suggest(&f, typed, today(), DateOrder::Dmy)).contains(&"Date: File modified: 2024".to_string()), "{typed}");
            }
            assert!(labels(&suggest(&f, "date taken", today(), DateOrder::Dmy)).contains(&"Date: Taken: 2024".to_string()));
            assert!(labels(&suggest(&f, "imported", today(), DateOrder::Dmy)).contains(&"Date: Imported: 2024".to_string()));
            // The year's tag is the one the typed phrase makes: offered once, and it finds 3.
            let s = suggest(&f, "created 2024", today(), DateOrder::Dmy);
            assert_eq!(labels(&s).iter().filter(|l| l.as_str() == "Date: File created: 2024").count(), 1, "{:?}", labels(&s));
            assert_eq!(ids(c, &[pick(&s, "File created: 2024")]), vec![3]);
            // A day, of any field, in any common form.
            let s = suggest(&f, "dateCreated 20.3.2024", today(), DateOrder::Dmy);
            assert_eq!(ids(c, &[pick(&s, "File created: 20 March 2024")]), vec![3]);
            let s = suggest(&f, "modified 2024-03-02", today(), DateOrder::Dmy);
            assert_eq!(ids(c, &[pick(&s, "File modified: 2 March 2024")]), vec![2]);
            let s = suggest(&f, "1.3.2024", today(), DateOrder::Dmy);
            assert_eq!(labels(&s)[0], "Date: 1 March 2024");
            assert_eq!(ids(c, &[pick(&s, "1 March 2024")]), vec![1, 2, 3, 4, 5, 6], "all taken on 1 March");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn any_typed_date_after_any_date_field_is_one_chip() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Around midnight on 4 June 2026, on the camera's wall clock.
            let at = |d: u32, h: u32, m: u32, s: u32| NaiveDate::from_ymd_opt(2026, 6, d).unwrap().and_hms_opt(h, m, s).unwrap().and_utc().timestamp();
            for (id, t) in [(1, at(3, 23, 59, 59)), (2, at(4, 0, 0, 0)), (3, at(4, 23, 59, 59)), (4, at(5, 0, 0, 0))] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, metadata_json, captured_at, imported_at, file_modified_at)
                     VALUES (?1, ?2, 'p', 'f', 'jpg', 0, 1, 1, '{}', ?3, ?3, ?4)",
                    rusqlite::params![id, format!("h{id}"), t, at(1, 12, 0, 0) + id * 86_400 * 30],
                )?;
            }
            let f = Facets::load(c)?;
            let first = |typed: &str| suggest(&f, typed, today(), DateOrder::Dmy).into_iter().next().unwrap();
            for typed in ["taken 4.6.2026", "date taken: 4.6.2026", "taken:4.6.2026", "Taken : 4.6.2026", "taken = 4.6.2026", "date taken 4 june"] {
                let s = suggest(&f, typed, today(), DateOrder::Dmy);
                assert_eq!(labels(&s)[0], "Date: Taken: 4 June 2026", "{typed}");
                assert_eq!(s.iter().filter(|x| x.field == "Date").count(), 1, "{typed}: one chip, {:?}", labels(&s));
                assert_eq!(ids(c, &[s[0].tag.clone()]), vec![2, 3], "{typed}");
            }
            // Each field by name, in the order the user reads dates.
            assert_eq!(suggest(&f, "taken 6/4/2026", today(), DateOrder::Mdy)[0].tag.label, "Taken: 4 June 2026");
            assert_eq!(suggest(&f, "taken 4/6/2026", today(), DateOrder::Mdy)[0].tag.label, "Taken: 6 April 2026");
            for (typed, label) in [
                ("modified before 2024", "File modified: before 2024"),
                ("created: 1.6.2026 – 4.6.2026", "File created: 1 – 4 June 2026"),
                ("imported after 4.6.2026", "Imported: after 4 June 2026"),
                ("digitized since March 2024", "Digitized: since March 2024"),
                ("date modified between June 2026 and August 2026", "File modified: June – August 2026"),
            ] {
                assert_eq!(first(typed).tag.label, label, "{typed}");
            }
            // An open end stays open: nothing taken after 4 June but the last.
            let after = first("taken after 4.6.2026");
            assert!(matches!(after.tag.value, TagValue::Dates { from: Some(_), to: None, field: None }), "{:?}", after.tag.value);
            assert_eq!(ids(c, &[after.tag]), vec![4]);
            let before = first("taken before 4.6.2026");
            assert_eq!(ids(c, &[before.tag]), vec![1]);
            // File times by their own clock: modified 1 July, 31 July, 30 August, 29 September.
            assert_eq!(ids(c, &[first("modified after 31.7.2026").tag]), vec![3, 4]);
            // A field's name alone still lists its years.
            assert!(labels(&suggest(&f, "taken", today(), DateOrder::Dmy)).contains(&"Date: Taken: 2026".to_string()));
            Ok(())
        })
        .unwrap();
    }

    /// Round 9A's library: 1 a Display P3 photo through a Sigma lens, digitized on 5 March;
    /// 2 an sRGB photo; 3 an iPhone HDR video (Dolby Vision on HLG, 10-bit, AAC); 4 an 8-bit
    /// H.264 video with PCM sound; 5 a silent 10-bit HDR10 video in Rec. 2020; 6 a video never
    /// probed.
    fn stored_but_not_offered(c: &Connection) -> Result<()> {
        let day = |d: u32| NaiveDate::from_ymd_opt(2024, 3, d).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let rows: [(i64, i64, String); 6] = [
            (1, 0, format!(r#"{{"lens_make":"Sigma","color_space":"Display P3","digitized_at":{}}}"#, day(5))),
            (2, 0, r#"{"color_space":"sRGB"}"#.into()),
            (3, 1, r#"{"probe":{"video_codec":"hevc","audio_codec":"aac","pix_fmt":"yuv420p10le","color_transfer":"arib-std-b67","color_primaries":"bt2020","dolby_vision":true}}"#.into()),
            (4, 1, r#"{"probe":{"video_codec":"h264","audio_codec":"pcm_s16le","pix_fmt":"yuv420p","color_transfer":"bt709","color_primaries":"bt709"}}"#.into()),
            (5, 1, r#"{"probe":{"video_codec":"hevc","pix_fmt":"yuv420p10le","color_transfer":"smpte2084","color_primaries":"bt2020"}}"#.into()),
            (6, 1, r#"{}"#.into()),
        ];
        for (id, media, meta) in rows {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, metadata_json, captured_at, imported_at)
                 VALUES (?1, ?2, 'p', 'f', 'jpg', ?3, 1, 1, ?4, ?5, ?5)",
                rusqlite::params![id, format!("h{id}"), media, meta, day(20)],
            )?;
        }
        Ok(())
    }

    #[test]
    fn what_lumen_stores_is_offered_as_tags() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            stored_but_not_offered(c)?;
            let f = Facets::load(c)?;
            let tag = |input: &str, label: &str| pick(&suggest(&f, input, today(), DateOrder::Dmy), label);
            let field = |input: &str, label: &str| suggest(&f, input, today(), DateOrder::Dmy).into_iter().find(|s| s.tag.label == label).map(|s| s.field);
            // The date digitized, as a date of its own.
            let digitized = tag("digitized march 2024", "Digitized: March 2024");
            assert_eq!(ids(c, &[digitized]), vec![1]);
            assert!(ids(c, &[tag("digitized 2023", "Digitized: 2023")]).is_empty());
            // Sound.
            assert_eq!(ids(c, &[tag("audio", "AAC")]), vec![3]);
            assert_eq!(ids(c, &[tag("audio codec", "PCM (16-bit)")]), vec![4], "each PCM its own chip");
            assert_eq!(codec_label("pcm_s24le"), "PCM (24-bit)");
            assert_eq!(codec_label("pcm_f32le"), "PCM (32-bit float)");
            assert_eq!(field("audio", "AAC").as_deref(), Some("Audio codec"));
            assert_eq!(ids(c, &[tag("no sound", "No sound")]), vec![5], "a video never probed is not called silent");
            // Colour space: a photo's profile, a video's primaries.
            assert_eq!(ids(c, &[tag("display p3", "Display P3")]), vec![1]);
            assert_eq!(ids(c, &[tag("color space", "sRGB")]), vec![2]);
            assert_eq!(ids(c, &[tag("colour", "Rec. 2020")]), vec![3, 5]);
            assert_eq!(ids(c, &[tag("colour", "Rec. 709")]), vec![4]);
            // HDR kind by transfer; Dolby Vision first.
            assert_eq!(ids(c, &[tag("hdr", "Dolby Vision")]), vec![3]);
            assert_eq!(ids(c, &[tag("hdr", "PQ (HDR10)")]), vec![5]);
            assert!(!labels(&suggest(&f, "hdr", today(), DateOrder::Dmy)).iter().any(|l| l.contains("bt709")), "SDR is no HDR kind");
            // Bit depth.
            assert_eq!(ids(c, &[tag("10-bit", "10-bit")]), vec![3, 5]);
            assert_eq!(ids(c, &[tag("bit depth", "8-bit")]), vec![4]);
            // Lens maker.
            assert_eq!(ids(c, &[tag("sigma", "Sigma")]), vec![1]);
            assert_eq!(field("sigma", "Sigma").as_deref(), Some("Lens maker"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_round_trip_through_json_for_the_url() {
        let tags = vec![
            SearchTag { label: "Lisbon, Portugal".into(), value: TagValue::Place { place: PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) } } },
            SearchTag { label: "Favorites".into(), value: TagValue::Favorites },
            SearchTag { label: "ISO 100".into(), value: TagValue::Exif { field: ExifField::Iso, value: "100".into() } },
        ];
        let json = serde_json::to_string(&tags).unwrap();
        assert!(json.contains(r#""type":"favorites""#), "{json}");
        assert_eq!(serde_json::from_str::<Vec<SearchTag>>(&json).unwrap(), tags);
    }

    /// Facets of a copy of a real catalogue, and what a few inputs suggest:
    /// `LUMEN_DB=<copy of library.db> cargo test --release -p photos-core facets_on_a_copy -- --ignored --nocapture`
    #[test]
    #[ignore = "needs LUMEN_DB (a copy)"]
    fn facets_on_a_copy() {
        let Ok(path) = std::env::var("LUMEN_DB") else { return };
        let db = Db::open(std::path::Path::new(&path)).unwrap();
        let started = std::time::Instant::now();
        let f = Facets::load_from(&db).unwrap();
        println!("facets: {} in {:?}, a short hold per query", f.len(), started.elapsed());
        db.with_conn(|_| {
            for input in [
                "exported", "iphone", "iso 100", "f/1", "lisbon", "flash", "beach", "2024", "heic", "stars",
                // Round 8A
                "lumen", "length", "longer than 2 min", "size", "mp", "4k", "fps", "codec", "portrait", "altitude", "serial", "modified 2024", "imported last week",
                // Round 9A
                "digitized 2024", "audio", "no sound", "colour space", "display p3", "hdr", "bit depth", "lens maker",
            ] {
                println!("{input}: {:?}", labels(&suggest(&f, input, today(), DateOrder::Dmy)));
            }
            Ok(())
        })
        .unwrap();
    }

    /// Suggestions answer a keystroke within the budget (≤ 30 ms at 250k).
    #[test]
    #[ignore]
    fn suggestions_at_250k() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 250_000, 7)?;
            let started = std::time::Instant::now();
            let f = Facets::load(c)?;
            println!("facets of 250k: {} in {:?}", f.len(), started.elapsed());
            for input in ["a", "ip", "iso", "exported", "lisbon portugal", "2023"] {
                let started = std::time::Instant::now();
                let s = suggest(&f, input, today(), DateOrder::Dmy);
                let took = started.elapsed();
                println!("{input}: {} in {took:?}", s.len());
                assert!(took < std::time::Duration::from_millis(30), "{input}: {took:?}");
            }
            Ok(())
        })
        .unwrap();
    }
}
