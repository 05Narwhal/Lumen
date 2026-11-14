//! What the user is looking for (architecture spec §11). One value describes free text
//! plus every structured filter, and compiles to fragments that [`crate::scope::LayoutScope`]
//! splices into the single layout query — so search, smart albums and filter chips all
//! render through the grid that already exists.

use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A photo with a line of text read in it (Round 13).
pub const HAS_TEXT: &str = "EXISTS (SELECT 1 FROM asset_text t WHERE t.asset_id = a.id)";

/// A saved or live search. Every field is optional; an empty query matches everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AssetQuery {
    /// Free text over filename, device name, place name and keywords.
    pub text: Option<String>,
    /// Inclusive bounds on the timeline timestamp (`captured_at`, else `imported_at`).
    #[ts(type = "number | null")]
    pub date_from: Option<i64>,
    #[ts(type = "number | null")]
    pub date_to: Option<i64>,
    #[ts(type = "number[]")]
    pub device_ids: Vec<i64>,
    /// 0 photos (including Live Photos), 1 videos.
    #[ts(type = "number | null")]
    pub media_type: Option<i64>,
    pub favorites: bool,
    #[ts(type = "number | null")]
    pub min_rating: Option<i64>,
    pub has_gps: Option<bool>,
    /// Only photos in which text was read (Round 13): `asset_text` has a line for them.
    pub has_text: bool,
    /// Only photos that were read and had no text (Round 18 C2): scanned, with no line.
    pub no_text: bool,
    /// Words that must be in the text read in the photo, and nowhere else (Round 18 C2): every
    /// rule must hold.
    pub in_text: Vec<TextMatch>,
    /// Camera model as EXIF reported it, matched case-insensitively as a substring.
    pub camera: Option<String>,
    pub lens: Option<String>,
    /// A special kind of item: Live Photos, RAW, HDR, screenshots, panoramas, selfies.
    pub kind: Option<AssetKind>,
    /// Only files with this extension (case-insensitive, without the dot), e.g. `nef`.
    pub extension: Option<String>,
    /// Show a RAW even when the library hides RAWs shot beside a JPEG (Develop's picker).
    pub include_raw_pairs: bool,
    /// Any of these colour labels (1 red … 5 purple); empty is no filter.
    #[ts(type = "number[]")]
    pub color_labels: Vec<u8>,
    /// −1 rejected, 0 unflagged, 1 picked.
    #[ts(type = "number | null")]
    pub flag: Option<i8>,
    /// Photos showing **every** one of these people (Round 4K).
    #[ts(type = "number[]")]
    pub person_ids: Vec<i64>,
    /// Photos showing every one of these pets (Phase C fills the table; empty until then).
    #[ts(type = "number[]")]
    pub pet_ids: Vec<i64>,
    /// Taken at every one of these places (a city, or a country).
    pub places: Vec<PlaceRef>,
    /// Showing every one of these scene tags (`embedding::SCENE_TAGS`).
    pub tags: Vec<String>,
    /// Taken in any of these months (1–12), in any year: "march", "summer".
    pub months: Vec<u8>,
    /// In every one of these albums.
    #[ts(type = "number[]")]
    pub album_ids: Vec<i64>,
    /// In an album of every one of these collections, albums of nested collections included
    /// (Round 19 §7).
    #[ts(type = "number[]")]
    pub collection_ids: Vec<i64>,
    /// Round 7's tags: each group is met by any one of its values; groups and every other
    /// field AND together. Absent from queries saved before it.
    pub any_of: Vec<AnyOf>,
    /// The tags chosen in a grid page's search field (Round 7), expanded into the fields
    /// above before the query runs ([`AssetQuery::expanded`]).
    pub search_tags: Vec<crate::tag_search::SearchTag>,
    /// The text read as `or` / `and` / `not` of terms (Round 19 §6). Absent means no operator
    /// was typed and the query runs exactly as before; a query without one is saved without it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional, as = "Option<crate::query_expr::QueryExpr>")]
    pub expr: Option<Box<crate::query_expr::QueryExpr>>,
    /// The search field's tags joined by `and` / `or` / `not` (Round 20 §1), turned into
    /// `search_tags` or `expr` before the query runs ([`AssetQuery::expanded`]). Absent from
    /// queries saved before it, which keep `search_tags`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tag_expr: Option<crate::tag_expr::TagSeq>,
    /// Photos whose embedding matched `text` by meaning (Phase 7b), filled in by the app just
    /// before the query runs. Never sent over the wire and never part of what the user saved.
    #[serde(skip)]
    #[ts(skip)]
    pub semantic: Option<Vec<i64>>,
    /// The order of *Best match* (Round 4K), best first, filled in by the app like `semantic`.
    /// It orders a scope with `best_match`; it never narrows anything.
    #[serde(skip)]
    #[ts(skip)]
    pub rank: Option<Vec<i64>>,
    /// Whether free text also searches the text read in photos (Round 13), set by the app from
    /// *Read text in photos* just before the query runs. Off, the `photo_text` column is left
    /// out of the match but its rows stay, so turning the switch on needs no rescan.
    #[serde(skip)]
    #[ts(skip)]
    pub photo_text: bool,
}

/// A `Text:` rule (Round 18 C2): `menu` finds photos whose own text has the word, `pri mostu`
/// both words in any order; `exact` keeps them together as typed (`Text: "pri mostu"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TextMatch {
    pub text: String,
    pub exact: bool,
}

impl TextMatch {
    /// The FTS5 expression, limited to the `photo_text` column; empty when no word is left.
    pub fn expression(&self) -> String {
        let words: Vec<String> = self
            .text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| crate::fts::fold_dj(&w.to_lowercase()).into_owned())
            .collect();
        if words.is_empty() {
            String::new()
        } else if self.exact {
            format!("{{photo_text}} : \"{}\"", words.join(" "))
        } else {
            format!("{{photo_text}} : ({})", words.iter().map(|w| format!("\"{w}\"*")).collect::<Vec<_>>().join(" "))
        }
    }
}

/// Values of one field that exclude each other, so any one matches (Round 7): Photos or
/// Videos, two cameras, two places, two dates.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum AnyOf {
    /// 0 photos (Live Photos included), 1 videos.
    MediaTypes(#[ts(type = "number[]")] Vec<i64>),
    Kinds(Vec<AssetKind>),
    Places(Vec<PlaceRef>),
    /// `[from, to]`, inclusive, in `captured_at` seconds, either end open (Round 11).
    Dates(#[ts(type = "[number | null, number | null][]")] Vec<(Option<i64>, Option<i64>)>),
    /// Without the dot, any case.
    Extensions(Vec<String>),
    Devices(#[ts(type = "number[]")] Vec<i64>),
    /// An EXIF field's values as `metadata_json` stores them, compared case-folded.
    Exif { field: ExifField, values: Vec<String> },
    /// Another date than when it was taken (Round 8A): `[from, to]`, inclusive, wall-clock
    /// seconds as `captured_at` reads them.
    DatesOf {
        field: DateField,
        #[ts(type = "[number | null, number | null][]")]
        spans: Vec<(Option<i64>, Option<i64>)>,
    },
    /// Spans of one measure (Round 8A): video length, file size, megapixels…, each
    /// `[min, max)` with either end open.
    Ranges {
        measure: Measure,
        #[ts(type = "[number | null, number | null][]")]
        spans: Vec<(Option<i64>, Option<i64>)>,
    },
    /// Portrait, landscape or square, as the photo is shown (Round 8A).
    Shapes(Vec<Shape>),
    /// Made in Lumen (Round 8A). One mark per group: a photo can be both edited and exported.
    Lumen(Vec<LumenMark>),
    /// A copy made for an external editor (Round 22 lane C): `None` any app, `Some(id)` that one.
    EditedIn(Vec<Option<String>>),
}

/// Which date a date tag reads (Round 8A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DateField {
    /// `captured_at`, else `imported_at`: the timeline's date.
    #[default]
    Taken,
    Imported,
    FileModified,
    FileCreated,
    /// EXIF `DateTimeDigitized` (Round 9A), wall-clock like `captured_at`.
    Digitized,
}

impl DateField {
    /// The date in wall-clock seconds, as `captured_at` is. The file and import times are UTC,
    /// so they are moved by this computer's offset, as months are. Constants only.
    pub fn sql(self) -> String {
        let local = |col: &str| format!("({col} + {})", crate::layout::local_offset_secs());
        match self {
            DateField::Taken => "COALESCE(a.captured_at, a.imported_at)".into(),
            DateField::Imported => local("a.imported_at"),
            DateField::FileModified => local("a.file_modified_at"),
            DateField::FileCreated => local("a.file_created_at"),
            DateField::Digitized => "json_extract(a.metadata_json, '$.digitized_at')".into(),
        }
    }
}

/// A number a range tag reads (Round 8A), each a fixed expression with its own unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Measure {
    /// A video's length, in milliseconds.
    Duration,
    /// The original's size, in bytes.
    FileSize,
    /// A photo's pixels (width × height).
    Pixels,
    /// A video's long edge, in pixels: 3840 is 4K.
    VideoSize,
    /// A video's frame rate, in thousandths of a frame per second.
    FrameRate,
    /// Metres above sea level, where the photo was taken.
    Altitude,
}

impl Measure {
    /// `(guard, value)`: which items have the measure at all, and its SQL. Constants only.
    pub fn sql(self) -> (&'static str, &'static str) {
        match self {
            Measure::Duration => ("a.media_type = 1", "a.duration_ms"),
            Measure::FileSize => ("1", "a.byte_size"),
            Measure::Pixels => ("a.media_type != 1", "(a.width * a.height)"),
            Measure::VideoSize => ("a.media_type = 1", "MAX(a.width, a.height)"),
            Measure::FrameRate => ("a.media_type = 1", "CAST(ROUND(json_extract(a.metadata_json, '$.probe.fps') * 1000) AS INTEGER)"),
            Measure::Altitude => ("1", "a.gps_altitude"),
        }
    }

    /// `[min, max)` as a WHERE clause with the bounds inline: they are numbers, never text.
    pub fn span_sql(self, (min, max): (Option<i64>, Option<i64>)) -> String {
        let (guard, value) = self.sql();
        let mut parts = vec![guard.to_string(), format!("{value} IS NOT NULL")];
        if let Some(lo) = min {
            parts.push(format!("{value} >= {lo}"));
        }
        if let Some(hi) = max {
            parts.push(format!("{value} < {hi}"));
        }
        format!("({})", parts.join(" AND "))
    }
}

/// How a photo is shown: taller, wider, or within 2 % of square.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Shape {
    Portrait,
    Landscape,
    Square,
}

impl Shape {
    /// Width and height as shown: orientations 5–8 turn the picture a quarter. Constants only.
    pub fn predicate(self) -> String {
        let (w, h) = ("(CASE WHEN a.orientation BETWEEN 5 AND 8 THEN a.height ELSE a.width END)", "(CASE WHEN a.orientation BETWEEN 5 AND 8 THEN a.width ELSE a.height END)");
        let shape = match self {
            Shape::Portrait => format!("{h} > {w} * 1.02"),
            Shape::Landscape => format!("{w} > {h} * 1.02"),
            Shape::Square => format!("ABS({w} - {h}) <= 0.02 * MAX({w}, {h})"),
        };
        format!("(a.width > 0 AND a.height > 0 AND {shape})")
    }
}

/// What Lumen made of a photo (Round 8A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LumenMark {
    /// Has an edit in the library.
    Edited,
    /// Has Develop work not yet exported (`develop_drafts`).
    Developed,
    /// Made by a Lumen export and added to the library: it knows its source, or its
    /// Software tag says Lumen.
    Exported,
}

impl LumenMark {
    /// Constants only.
    pub fn predicate(self) -> String {
        match self {
            LumenMark::Edited => "a.id IN (SELECT e.asset_id FROM edits e)".into(),
            LumenMark::Developed => "a.id IN (SELECT dd.asset_id FROM develop_drafts dd)".into(),
            LumenMark::Exported => "(a.derived_from IS NOT NULL OR LOWER(COALESCE(json_extract(a.metadata_json, '$.software'), '')) = 'lumen')".into(),
        }
    }
}

/// The EXIF fields a tag can name, each a fixed key of `assets.metadata_json` (Round 7), so
/// no path ever comes from the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ExifField {
    Make,
    Model,
    Lens,
    Iso,
    Aperture,
    Exposure,
    FocalLength,
    Software,
    WhiteBalance,
    Flash,
    Metering,
    ExposureProgram,
    ExposureBias,
    /// A video's codec, as FFmpeg names it (`h264`, `hevc`), Round 8A.
    VideoCodec,
    /// The camera body's serial number, Round 8A.
    BodySerial,
    /// A video's sound, as FFmpeg names it (`aac`, `pcm_s16le`), Round 9A.
    AudioCodec,
    /// "sRGB", "Display P3": a photo's profile, else a video's primaries (Round 9A).
    ColorSpace,
    /// A video's HDR: Dolby Vision, HLG or PQ (Round 9A).
    HdrFormat,
    /// A video's bits per colour: 8, 10 or 12 (Round 9A).
    BitDepth,
    /// EXIF `LensMake` (Round 9A).
    LensMake,
}

impl ExifField {
    pub const ALL: [ExifField; 20] = [
        ExifField::Make,
        ExifField::Model,
        ExifField::Lens,
        ExifField::Iso,
        ExifField::Aperture,
        ExifField::Exposure,
        ExifField::FocalLength,
        ExifField::Software,
        ExifField::WhiteBalance,
        ExifField::Flash,
        ExifField::Metering,
        ExifField::ExposureProgram,
        ExifField::ExposureBias,
        ExifField::VideoCodec,
        ExifField::BodySerial,
        ExifField::AudioCodec,
        ExifField::ColorSpace,
        ExifField::HdrFormat,
        ExifField::BitDepth,
        ExifField::LensMake,
    ];

    /// The `metadata_json` key (`photos_media::Metadata`'s field name).
    pub fn key(self) -> &'static str {
        match self {
            ExifField::Make => "make",
            ExifField::Model => "model",
            ExifField::Lens => "lens",
            ExifField::Iso => "iso",
            ExifField::Aperture => "aperture",
            ExifField::Exposure => "exposure",
            ExifField::FocalLength => "focal_length_mm",
            ExifField::Software => "software",
            ExifField::WhiteBalance => "white_balance",
            ExifField::Flash => "flash",
            ExifField::Metering => "metering",
            ExifField::ExposureProgram => "exposure_program",
            ExifField::ExposureBias => "exposure_bias",
            ExifField::VideoCodec => "probe.video_codec",
            ExifField::BodySerial => "body_serial",
            ExifField::AudioCodec => "probe.audio_codec",
            ExifField::ColorSpace => "color_space",
            ExifField::HdrFormat => "probe.color_transfer",
            ExifField::BitDepth => "probe.pix_fmt",
            ExifField::LensMake => "lens_make",
        }
    }

    /// The value as SQL, as suggestions list it: the stored field, or for a few (Round 9A)
    /// what it means. Built from constants only.
    pub fn raw_sql(self) -> String {
        let at = |key: &str| format!("json_extract(a.metadata_json, '$.{key}')");
        match self {
            ExifField::ColorSpace => format!(
                "COALESCE({}, CASE {} WHEN 'bt709' THEN 'Rec. 709' WHEN 'bt2020' THEN 'Rec. 2020' WHEN 'smpte432' THEN 'Display P3' END)",
                at("color_space"),
                at("probe.color_primaries")
            ),
            ExifField::HdrFormat => format!(
                "CASE WHEN {} THEN 'Dolby Vision' WHEN {t} = 'arib-std-b67' THEN 'HLG' WHEN {t} = 'smpte2084' THEN 'PQ (HDR10)' END",
                at("probe.dolby_vision"),
                t = at("probe.color_transfer")
            ),
            ExifField::BitDepth => format!(
                "CASE WHEN {p} IS NULL THEN NULL WHEN {p} LIKE '%12le' OR {p} LIKE '%12be' THEN '12' WHEN {p} LIKE '%10le' OR {p} LIKE '%10be' OR {p} LIKE 'p010%' THEN '10' ELSE '8' END",
                p = at("probe.pix_fmt")
            ),
            _ => at(self.key()),
        }
    }

    /// The value as SQL text, case-folded: what a tag's value is compared with, and what
    /// suggestions list. Built from constants only.
    pub fn sql_value(self) -> String {
        format!("LOWER(CAST({} AS TEXT))", self.raw_sql())
    }
}

/// The WHERE clause for one place, binding its parameters.
fn place_clause(place: &PlaceRef, params: &mut Vec<Value>) -> String {
    match place {
        PlaceRef::City { country: Some(_), .. } => {
            params.push(Value::Text(place.label()));
            "a.place_name = ?".into()
        }
        PlaceRef::City { name, country: None } => {
            params.push(Value::Text(name.clone()));
            params.push(Value::Text(format!("{}, %", like_escape(name))));
            "(a.place_name = ? OR a.place_name LIKE ? ESCAPE '\\')".into()
        }
        PlaceRef::Locality { .. } => {
            params.push(Value::Text(place.label()));
            "a.locality = ?".into()
        }
        PlaceRef::Country { name } => {
            params.push(Value::Text(name.clone()));
            params.push(Value::Text(format!("%, {}", like_escape(name))));
            "(a.place_name = ? OR a.place_name LIKE ? ESCAPE '\\')".into()
        }
    }
}

impl AnyOf {
    /// One `( … OR … )` clause, or `None` for a group that narrows nothing.
    fn clause(&self, params: &mut Vec<Value>) -> Option<String> {
        let holes = |n: usize| std::iter::repeat_n("?", n).collect::<Vec<_>>().join(",");
        let parts: Vec<String> = match self {
            AnyOf::MediaTypes(v) => match (v.iter().any(|m| *m != 1), v.contains(&1)) {
                (true, true) | (false, false) => return None,
                (true, false) => vec!["a.media_type != 1".into()],
                (false, true) => vec!["a.media_type = 1".into()],
            },
            AnyOf::Kinds(v) => v.iter().map(|k| k.predicate()).collect(),
            AnyOf::Places(v) => v.iter().map(|p| place_clause(p, params)).collect(),
            AnyOf::Dates(v) => v.iter().map(|span| date_span_sql("COALESCE(a.captured_at, a.imported_at)", *span, params)).collect(),
            AnyOf::Extensions(v) if !v.is_empty() => {
                params.extend(v.iter().map(|e| Value::Text(e.trim().trim_start_matches('.').to_lowercase())));
                vec![format!("a.extension IN ({})", holes(v.len()))]
            }
            AnyOf::Devices(v) if !v.is_empty() => {
                params.extend(v.iter().map(|d| Value::Integer(*d)));
                vec![format!("a.device_id IN ({})", holes(v.len()))]
            }
            AnyOf::Exif { field, values } if !values.is_empty() => {
                params.extend(values.iter().map(|v| Value::Text(v.trim().to_lowercase())));
                vec![format!("{} IN ({})", field.sql_value(), holes(values.len()))]
            }
            AnyOf::DatesOf { field, spans } => {
                let date = field.sql();
                spans.iter().map(|span| date_span_sql(&date, *span, params)).collect()
            }
            AnyOf::Ranges { measure, spans } => spans.iter().map(|s| measure.span_sql(*s)).collect(),
            AnyOf::Shapes(v) => v.iter().map(|s| s.predicate()).collect(),
            AnyOf::Lumen(v) => v.iter().map(|m| m.predicate()).collect(),
            AnyOf::EditedIn(v) => v
                .iter()
                .map(|app| match app {
                    Some(id) => {
                        params.push(Value::Text(id.clone()));
                        "a.edited_in = ?".to_string()
                    }
                    None => "a.edited_in IS NOT NULL".to_string(),
                })
                .collect(),
            _ => vec![],
        };
        (!parts.is_empty()).then(|| format!("({})", parts.join(" OR ")))
    }
}

/// `date` inside `[from, to]`, inclusive, an absent end open.
fn date_span_sql(date: &str, (from, to): (Option<i64>, Option<i64>), params: &mut Vec<Value>) -> String {
    params.extend(from.into_iter().chain(to).map(Value::Integer));
    match (from, to) {
        (Some(_), Some(_)) => format!("{date} BETWEEN ? AND ?"),
        (Some(_), None) => format!("{date} >= ?"),
        (None, Some(_)) => format!("{date} <= ?"),
        (None, None) => "1".into(),
    }
}

/// A place as the catalogue names it (`assets.place_name` is "City, Country" or "Country").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum PlaceRef {
    /// A city; `country` tells Paris, France from Paris, United States.
    City { name: String, country: Option<String> },
    Country { name: String },
    /// The village or district a photo was taken in (`assets.locality`, Round 4K): every
    /// populated place of 500 people or more, so all of a library's places can be searched.
    Locality { name: String, country: Option<String> },
}

impl PlaceRef {
    /// "Lisbon, Portugal" or "Portugal": what the place's photos store as `place_name`.
    pub fn label(&self) -> String {
        match self {
            PlaceRef::City { name, country: Some(c) } | PlaceRef::Locality { name, country: Some(c) } => format!("{name}, {c}"),
            PlaceRef::City { name, country: None } | PlaceRef::Locality { name, country: None } => name.clone(),
            PlaceRef::Country { name } => name.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum AssetKind {
    Live,
    Raw,
    Hdr,
    Screenshot,
    Panorama,
    Selfie,
    /// A video with no sound (Round 9A). Only a probed video: an unprobed one may have some.
    Silent,
}

impl AssetKind {
    /// The WHERE clause for alias `a`. Built from constants only, so nothing is bound.
    pub fn predicate(self) -> String {
        match self {
            AssetKind::Live => format!("a.media_type = {}", crate::assets::MEDIA_TYPE_LIVE),
            AssetKind::Raw => format!("a.extension IN {}", crate::formats::raw_sql_list()),
            AssetKind::Hdr => "a.hdr > 0".into(),
            // Desktop screenshots are named so; a phone's are PNGs no camera took. An
            // iPhone PNG *export* of a photo keeps its camera, and so its device.
            AssetKind::Screenshot => "(LOWER(a.filename) LIKE 'screenshot%' OR LOWER(a.filename) LIKE 'screen shot%' \
                 OR (a.extension = 'png' AND a.device_id IS NULL))"
                .into(),
            // At least 2.5:1 either way, which no camera's native frame is.
            AssetKind::Panorama => "(a.media_type != 1 AND a.width > 0 AND a.height > 0 \
                 AND MAX(a.width, a.height) >= 2.5 * MIN(a.width, a.height))"
                .into(),
            // Phones name the lens, and the front one is the selfie camera.
            AssetKind::Selfie => "LOWER(COALESCE(json_extract(a.metadata_json, '$.lens'), '')) LIKE '%front%'".into(),
            AssetKind::Silent => "(a.media_type = 1 AND json_extract(a.metadata_json, '$.probe') IS NOT NULL AND json_extract(a.metadata_json, '$.probe.audio_codec') IS NULL)".into(),
        }
    }
}

impl AssetQuery {
    /// This query with its search tags turned into the fields they narrow, and none left. A tag
    /// sequence that says what a flat list says joins `search_tags`; any other becomes one more
    /// term of `expr` (Round 20 §1).
    pub fn expanded(&self) -> AssetQuery {
        let mut q = self.clone();
        if let Some(seq) = q.tag_expr.take() {
            let s = seq.repaired();
            match s.as_plain() {
                Some(tags) => q.search_tags.extend(tags),
                None => {
                    if let Some(e) = s.to_expr(&mut crate::tag_expr::core_leaf) {
                        q.expr = Some(Box::new(match q.expr.take() {
                            Some(typed) => crate::query_expr::QueryExpr::And(vec![*typed, e]),
                            None => e,
                        }));
                    }
                }
            }
        }
        if q.search_tags.is_empty() {
            return q;
        }
        let tags = std::mem::take(&mut q.search_tags);
        crate::tag_search::merge(&q, &crate::tag_search::to_query(&tags))
    }

    /// True when the query narrows nothing, so callers can skip it entirely. A tag sequence
    /// with no tags narrows nothing.
    pub fn is_empty(&self) -> bool {
        let tag_expr = self.tag_expr.as_ref().map(|s| s.repaired()).filter(|s| !s.is_empty());
        Self { semantic: None, rank: None, photo_text: false, tag_expr, ..self.clone() } == Self::default()
    }

    /// An FTS5 `MATCH` expression: every word becomes a quoted prefix term, so "img 004"
    /// finds `IMG_0042.HEIC`. Punctuation is dropped rather than escaped — FTS5's syntax
    /// characters inside a quoted string are literal, but an unbalanced quote is an error.
    pub fn match_expression(text: &str) -> String {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{}\"*", crate::fts::fold_dj(&w.to_lowercase())))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `(extra joins, WHERE clauses, parameters)`. Parameters are in the order the
    /// fragments use them, after whatever the scope bound first.
    pub fn sql_parts(&self) -> (String, Vec<String>, Vec<Value>) {
        self.sql_parts_with(true)
    }

    /// The query as one WHERE clause of its own, with no join (Round 19 §6): a term inside an
    /// `or` / `not`. `1` when it narrows nothing.
    pub(crate) fn clause(&self, params: &mut Vec<Value>) -> String {
        let (_, wh, qp) = self.sql_parts_with(false);
        params.extend(qp);
        if wh.is_empty() {
            "1".into()
        } else {
            format!("({})", wh.join(" AND "))
        }
    }

    /// `fts_join`: free text joins `assets_fts`, as it always has; off, it is a subquery, so the
    /// clause can stand inside `or` and `not`.
    fn sql_parts_with(&self, fts_join: bool) -> (String, Vec<String>, Vec<Value>) {
        if !self.search_tags.is_empty() || self.tag_expr.is_some() {
            return self.expanded().sql_parts_with(fts_join);
        }
        let mut joins = String::new();
        let mut wh: Vec<String> = Vec::new();
        let mut params: Vec<Value> = Vec::new();

        if let Some(text) = self.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            let mut expr = Self::match_expression(text);
            if !self.photo_text && !expr.is_empty() {
                expr = format!("{{filename device_name place_name keywords}} : ({expr})");
            }
            match (&self.semantic, expr.is_empty()) {
                // Words or meaning: a photo matches by its text or by what it shows.
                (Some(ids), _) if !ids.is_empty() => {
                    let json = serde_json::to_string(ids).unwrap_or_else(|_| "[]".into());
                    if expr.is_empty() {
                        wh.push("a.id IN (SELECT value FROM json_each(?))".into());
                    } else {
                        wh.push("(a.id IN (SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?) OR a.id IN (SELECT value FROM json_each(?)))".into());
                        params.push(Value::Text(expr));
                    }
                    params.push(Value::Text(json));
                }
                (_, false) if fts_join => {
                    joins.push_str(" JOIN assets_fts f ON f.rowid = a.id");
                    wh.push("assets_fts MATCH ?".into());
                    params.push(Value::Text(expr));
                }
                (_, false) => {
                    wh.push("a.id IN (SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?)".into());
                    params.push(Value::Text(expr));
                }
                _ => {}
            }
        }
        if let Some(from) = self.date_from {
            wh.push("COALESCE(a.captured_at, a.imported_at) >= ?".into());
            params.push(Value::Integer(from));
        }
        if let Some(to) = self.date_to {
            wh.push("COALESCE(a.captured_at, a.imported_at) <= ?".into());
            params.push(Value::Integer(to));
        }
        if !self.device_ids.is_empty() {
            let holes = std::iter::repeat_n("?", self.device_ids.len()).collect::<Vec<_>>().join(",");
            wh.push(format!("a.device_id IN ({holes})"));
            params.extend(self.device_ids.iter().map(|d| Value::Integer(*d)));
        }
        if let Some(mt) = self.media_type {
            wh.push(if mt == 1 { "a.media_type = 1".into() } else { "a.media_type != 1".into() });
        }
        if self.favorites {
            wh.push("a.is_favorite = 1".into());
        }
        if let Some(kind) = self.kind {
            wh.push(kind.predicate());
        }
        if let Some(ext) = self.extension.as_deref().map(|e| e.trim().trim_start_matches('.')).filter(|e| !e.is_empty()) {
            wh.push("a.extension = ?".into());
            params.push(Value::Text(ext.to_lowercase()));
        }
        if !self.color_labels.is_empty() {
            // Small integers from a typed field: inlined, so no parameter order to keep.
            let list: Vec<String> = self.color_labels.iter().map(|l| l.min(&5).to_string()).collect();
            wh.push(format!("a.color_label IN ({})", list.join(",")));
        }
        if let Some(f) = self.flag {
            wh.push(format!("a.flag = {}", f.clamp(-1, 1)));
        }
        if let Some(r) = self.min_rating.filter(|r| *r > 0) {
            wh.push("a.rating >= ?".into());
            params.push(Value::Integer(r));
        }
        match self.has_gps {
            Some(true) => wh.push("a.gps_lat IS NOT NULL".into()),
            Some(false) => wh.push("a.gps_lat IS NULL".into()),
            None => {}
        }
        if self.has_text {
            wh.push(HAS_TEXT.into());
        }
        if self.no_text {
            wh.push(format!("EXISTS (SELECT 1 FROM ocr_scan o WHERE o.asset_id = a.id) AND NOT {HAS_TEXT}"));
        }
        for rule in &self.in_text {
            let expr = rule.expression();
            if !expr.is_empty() {
                wh.push("a.id IN (SELECT rowid FROM assets_fts WHERE assets_fts MATCH ?)".into());
                params.push(Value::Text(expr));
            }
        }
        // Every person: one subquery each, so "Anna Ben" is photos of both. `IN` rather than
        // `EXISTS`, so SQLite reads `faces_person` once instead of probing faces per photo.
        for id in &self.person_ids {
            wh.push("a.id IN (SELECT fp.asset_id FROM faces fp WHERE fp.person_id = ? AND fp.rejected IS NULL)".into());
            params.push(Value::Integer(*id));
        }
        // Every pet, as every person (Round 4C).
        for id in &self.pet_ids {
            wh.push("a.id IN (SELECT pd.asset_id FROM pet_detections pd WHERE pd.pet_id = ? AND pd.rejected IS NULL)".into());
            params.push(Value::Integer(*id));
        }
        for place in &self.places {
            let clause = place_clause(place, &mut params);
            wh.push(clause);
        }
        // Scene tags are stored space-separated, and a tag may hold spaces ("night sky"), so a
        // tag matches as a whole run of words between spaces.
        for tag in self.tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
            wh.push("(' ' || COALESCE(a.scene_tags, '') || ' ') LIKE ? ESCAPE '\\'".into());
            params.push(Value::Text(format!("% {} %", like_escape(&tag.to_lowercase()))));
        }
        let months: Vec<String> = self.months.iter().filter(|m| (1..=12).contains(*m)).map(|m| m.to_string()).collect();
        if !months.is_empty() {
            // `captured_at` is already wall-clock time; only `imported_at` (UTC) is shifted.
            wh.push(format!(
                "CAST(strftime('%m', COALESCE(a.captured_at, a.imported_at + {}), 'unixepoch') AS INTEGER) IN ({})",
                crate::layout::local_offset_secs(),
                months.join(",")
            ));
        }
        for id in &self.album_ids {
            wh.push("a.id IN (SELECT ai2.asset_id FROM album_items ai2 WHERE ai2.album_id = ?)".into());
            params.push(Value::Integer(*id));
        }
        for id in &self.collection_ids {
            wh.push(
                "a.id IN (SELECT ai3.asset_id FROM album_items ai3 JOIN albums al3 ON al3.id = ai3.album_id WHERE al3.collection_id IN \
                 (WITH RECURSIVE sub(id) AS (SELECT ? UNION SELECT c.id FROM album_collections c JOIN sub ON c.parent_id = sub.id) SELECT id FROM sub))"
                    .into(),
            );
            params.push(Value::Integer(*id));
        }
        // Camera and lens live in the metadata blob; JSON1 ships with rusqlite's bundled SQLite.
        for (field, path) in [(&self.camera, "$.model"), (&self.lens, "$.lens")] {
            if let Some(v) = field.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                wh.push(format!("LOWER(COALESCE(json_extract(a.metadata_json, '{path}'), '')) LIKE ?"));
                params.push(Value::Text(format!("%{}%", v.to_lowercase())));
            }
        }
        for group in &self.any_of {
            if let Some(clause) = group.clause(&mut params) {
                wh.push(clause);
            }
        }
        // The terms of an `or` / `and` / `not` search (Round 19 §6), after every other field.
        if let Some(expr) = &self.expr {
            wh.push(expr.sql(&mut params));
        }
        (joins, wh, params)
    }
}

/// `s` with LIKE's wildcards and the escape character escaped, for `ESCAPE '\'`.
fn like_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    /// Round 7: tags combine with AND, but values of one field that exclude each other
    /// (Photos or Videos, two cameras, two places) are an OR group.
    #[test]
    fn any_of_groups_are_or_inside_and_between() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for (i, media, model, place, ext) in [
                (1, 0, "iPhone 12", "Lisbon, Portugal", "heic"),
                (2, 1, "iPhone 12", "Porto, Portugal", "mov"),
                (3, 0, "NIKON D7100", "Lisbon, Portugal", "nef"),
                (4, 0, "Pixel 8", "Paris, France", "jpg"),
            ] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, place_name, metadata_json)
                     VALUES (?1, ?2, 'p', 'f', ?3, ?4, 1, 1, 10, ?5, json_object('model', ?6, 'iso', 100, 'aperture', 1.8, 'software', 'Adobe Lightroom'))",
                    rusqlite::params![i, format!("h{i}"), ext, media, place, model],
                )?;
            }
            let ids = |q: AssetQuery| crate::layout::ids_scoped(c, &crate::scope::LayoutScope { query: Some(q), ..Default::default() }, 100);
            let q = |any_of: Vec<AnyOf>| AssetQuery { any_of, ..Default::default() };
            let mut both = ids(q(vec![AnyOf::MediaTypes(vec![0, 1])]))?;
            both.sort();
            assert_eq!(both, vec![1, 2, 3, 4], "Photos or Videos is everything");
            let mut two = ids(q(vec![AnyOf::Exif { field: ExifField::Model, values: vec!["iphone 12".into(), "Pixel 8".into()] }]))?;
            two.sort();
            assert_eq!(two, vec![1, 2, 4], "either camera, case folded");
            let lisbon = PlaceRef::City { name: "Lisbon".into(), country: Some("Portugal".into()) };
            let porto = PlaceRef::City { name: "Porto".into(), country: Some("Portugal".into()) };
            let mut and = ids(q(vec![
                AnyOf::Exif { field: ExifField::Model, values: vec!["iPhone 12".into()] },
                AnyOf::Places(vec![lisbon.clone(), porto]),
                AnyOf::MediaTypes(vec![0]),
            ]))?;
            and.sort();
            assert_eq!(and, vec![1], "groups AND");
            assert_eq!(ids(q(vec![AnyOf::Extensions(vec!["NEF".into()])]))?, vec![3]);
            assert_eq!(ids(q(vec![AnyOf::Exif { field: ExifField::Aperture, values: vec!["1.8".into()] }]))?.len(), 4, "numbers match as stored");
            assert_eq!(ids(q(vec![AnyOf::Exif { field: ExifField::Software, values: vec!["adobe lightroom".into()] }]))?.len(), 4);
            // A field is a fixed name, never a JSON path from the wire.
            assert!(serde_json::from_str::<AnyOf>(r#"{"exif":{"field":"$.x') OR 1=1 --","values":["a"]}}"#).is_err());
            // Older saved queries have no `anyOf` and still load.
            let old: AssetQuery = serde_json::from_str(r#"{"text":null,"dateFrom":null,"dateTo":null,"deviceIds":[],"mediaType":null,"favorites":false,"minRating":null,"hasGps":null,"camera":null,"lens":null,"kind":null}"#)?;
            assert!(old.any_of.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn meaning_finds_what_words_miss_and_nothing_changes_without_it() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            for (i, name) in [(1, "beach.jpg"), (2, "IMG_0002.jpg"), (3, "IMG_0003.jpg")] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
                     VALUES (?1, ?2, 'p', ?3, 'jpg', 0, 1, 1, 10)",
                    rusqlite::params![i, format!("h{i}"), name],
                )?;
            }
            crate::fts::rebuild(c)?;
            let count = |q: AssetQuery| crate::layout::count_scoped(c, &crate::scope::LayoutScope { query: Some(q), ..Default::default() });
            let words = AssetQuery { text: Some("beach".into()), ..Default::default() };
            assert_eq!(count(words.clone())?, 1);
            assert_eq!(count(AssetQuery { semantic: Some(vec![2]), ..words.clone() })?, 2, "the file name, or what it shows");
            assert_eq!(count(AssetQuery { semantic: Some(vec![]), ..words.clone() })?, 1, "no hits: words alone");
            let nonsense = AssetQuery { text: Some("!!!".into()), semantic: Some(vec![3]), ..Default::default() };
            assert_eq!(count(nonsense)?, 1, "meaning without a matchable word");
            assert!(AssetQuery { semantic: Some(vec![1]), ..Default::default() }.is_empty(), "hits alone are not a query");
            let json = serde_json::to_string(&AssetQuery { semantic: Some(vec![1]), ..words })?;
            assert!(!json.contains("semantic"));
            Ok(())
        })
        .unwrap();
    }

    use super::*;

    /// Six photos in March and July of 2023–2025; Anna (1) is in 1, 2, 3, Ben (2) in 2, 3, 4;
    /// 2 and 5 are at the beach, 3 is a sunset at the beach; 1 and 2 were taken in Lisbon.
    fn people_library(c: &rusqlite::Connection) -> crate::Result<()> {
        let at = |y: i32, m: u32| chrono::NaiveDate::from_ymd_opt(y, m, 15).unwrap().and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp();
        let rows = [
            (1, at(2023, 3), "", Some("Lisbon, Portugal")),
            (2, at(2023, 7), "beach", Some("Lisbon, Portugal")),
            (3, at(2024, 3), "sunset beach", Some("Porto, Portugal")),
            (4, at(2024, 7), "dog", Some("Portugal")),
            (5, at(2025, 3), "beach", Some("Paris, France")),
            (6, at(2025, 7), "group of people", None),
        ];
        for (id, ts, tags, place) in rows {
            c.execute(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, captured_at, imported_at, scene_tags, place_name)
                 VALUES (?1, ?2, 'p', 'x.jpg', 'jpg', 0, 1, 1, ?3, ?3, ?4, ?5)",
                rusqlite::params![id, format!("h{id}"), ts, tags, place],
            )?;
        }
        c.execute_batch(
            "INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 0), (2, 'Ben', 0);
             INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id, rejected) VALUES
               (1, 0, 0, .1, .1, .9, x'00', 'm', 1, NULL), (2, 0, 0, .1, .1, .9, x'00', 'm', 1, NULL), (3, 0, 0, .1, .1, .9, x'00', 'm', 1, NULL),
               (2, .5, 0, .1, .1, .9, x'00', 'm', 2, NULL), (3, .5, 0, .1, .1, .9, x'00', 'm', 2, NULL), (4, .5, 0, .1, .1, .9, x'00', 'm', 2, NULL),
               (6, .5, 0, .1, .1, .9, x'00', 'm', 2, 'screen');
             INSERT INTO albums (id, name, kind, created_at, updated_at) VALUES (9, 'Trip', 0, 0, 0);
             INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (9, 2, 1, 0), (9, 5, 2, 0);",
        )?;
        Ok(())
    }

    fn ids(c: &rusqlite::Connection, q: AssetQuery) -> Vec<i64> {
        let scope = crate::scope::LayoutScope { query: Some(q), ..Default::default() };
        let mut ids = crate::layout::ids_scoped(c, &scope, 100).unwrap();
        ids.sort_unstable();
        ids
    }

    #[test]
    fn has_text_keeps_photos_with_lines() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            c.execute("INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (2, 0, 'Gostilna', 0.9, x'00', 'Latn')", [])?;
            let q = AssetQuery { has_text: true, ..Default::default() };
            assert!(!q.is_empty());
            assert_eq!(ids(c, q), vec![2]);
            let old: AssetQuery = serde_json::from_str(r#"{"favorites":false}"#)?;
            assert!(!old.has_text, "a query saved before Round 13 has no text rule");
            Ok(())
        })
        .unwrap();
    }

    /// Round 18 C2: `Text: menu` looks in the text read in photos and nowhere else.
    fn read_lines(c: &rusqlite::Connection, rows: &[(i64, &[&str])]) -> crate::Result<()> {
        for (asset, lines) in rows {
            c.execute("INSERT OR IGNORE INTO ocr_scan (asset_id, model) VALUES (?1, 'm')", [asset])?;
            for (ord, text) in lines.iter().enumerate() {
                c.execute("INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (?1, ?2, ?3, 0.9, x'00', 'Latn')", rusqlite::params![asset, ord as i64, text])?;
            }
            crate::fts::index_ids(c, &[*asset])?;
        }
        Ok(())
    }

    #[test]
    fn text_in_photo_matches_only_the_text_read() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            // Photo 1's keywords say "menu" too, but nothing was read in it.
            c.execute("UPDATE assets SET scene_tags = 'menu' WHERE id = 1", [])?;
            c.execute("INSERT INTO ocr_scan (asset_id, model) VALUES (4, 'm')", [])?;
            read_lines(c, &[(2, &["Gostilna Pri Mostu", "Dnevni menu 12 €"]), (3, &["Menu", "Juha dneva"]), (5, &["Pri Mostu"])])?;
            let by = |text: &str, exact: bool| AssetQuery { in_text: vec![TextMatch { text: text.into(), exact }], ..Default::default() };
            assert!(!by("menu", false).is_empty());
            assert_eq!(ids(c, by("menu", false)), vec![2, 3], "words in the photo's own text, not its keywords");
            assert_eq!(ids(c, by("men", false)), vec![2, 3], "a word may be cut short, as in search");
            assert_eq!(ids(c, by("pri mostu", false)), vec![2, 5]);
            assert_eq!(ids(c, by("mostu gostilna", false)), vec![2], "every word, in any order");
            assert_eq!(ids(c, by("mostu gostilna", true)), Vec::<i64>::new(), "an exact phrase keeps the order");
            assert_eq!(ids(c, by("pri mostu", true)), vec![2, 5]);
            assert_eq!(ids(c, by("dnevni menu", true)), vec![2]);
            assert_eq!(ids(c, by("gostilna pri mostu dnevni", true)), vec![2], "a phrase may run over a line break");
            // Two rules: both must hold.
            let both = AssetQuery { in_text: vec![TextMatch { text: "menu".into(), exact: false }, TextMatch { text: "juha".into(), exact: false }], ..Default::default() };
            assert_eq!(ids(c, both), vec![3]);
            // Punctuation alone matches nothing and breaks nothing.
            assert_eq!(ids(c, by("\"", false)), vec![1, 2, 3, 4, 5, 6], "nothing left to match narrows nothing");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn no_text_is_read_and_empty() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            c.execute("INSERT INTO ocr_scan (asset_id, model) VALUES (4, 'm')", [])?;
            read_lines(c, &[(2, &["Gostilna"])])?;
            // 2 has text, 4 was read and has none, the rest were never read.
            assert_eq!(ids(c, AssetQuery { no_text: true, ..Default::default() }), vec![4]);
            let old: AssetQuery = serde_json::from_str(r#"{"favorites":false}"#)?;
            assert!(old.in_text.is_empty() && !old.no_text, "a query saved before Round 18 has no such rule");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn two_people_means_both() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            assert_eq!(ids(c, AssetQuery { person_ids: vec![1], ..Default::default() }), vec![1, 2, 3]);
            assert_eq!(ids(c, AssetQuery { person_ids: vec![2], ..Default::default() }), vec![2, 3, 4], "a rejected face is nobody");
            assert_eq!(ids(c, AssetQuery { person_ids: vec![1, 2], ..Default::default() }), vec![2, 3]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn tags_places_months_and_albums_filter() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            let tag = |t: &[&str]| AssetQuery { tags: t.iter().map(|s| s.to_string()).collect(), ..Default::default() };
            assert_eq!(ids(c, tag(&["beach"])), vec![2, 3, 5]);
            assert_eq!(ids(c, tag(&["beach", "sunset"])), vec![3]);
            assert_eq!(ids(c, tag(&["group of people"])), vec![6], "a tag of several words");
            assert!(ids(c, tag(&["sea"])).is_empty(), "a tag is a whole tag");
            let place = |p: PlaceRef| AssetQuery { places: vec![p], ..Default::default() };
            let city = |n: &str, c: Option<&str>| PlaceRef::City { name: n.into(), country: c.map(Into::into) };
            assert_eq!(ids(c, place(city("Lisbon", Some("Portugal")))), vec![1, 2]);
            assert_eq!(ids(c, place(city("Paris", None))), vec![5]);
            assert_eq!(ids(c, place(PlaceRef::Country { name: "Portugal".into() })), vec![1, 2, 3, 4]);
            assert_eq!(ids(c, AssetQuery { months: vec![3], ..Default::default() }), vec![1, 3, 5], "March of three years");
            assert_eq!(ids(c, AssetQuery { months: vec![6, 7, 8], ..Default::default() }), vec![2, 4, 6]);
            assert_eq!(ids(c, AssetQuery { album_ids: vec![9], ..Default::default() }), vec![2, 5]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_collection_holds_the_photos_of_its_albums_nested_ones_included() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            c.execute_batch(
                "INSERT INTO album_collections (id, name, parent_id, created_at, updated_at) VALUES (1, 'Trips', NULL, 0, 0), (2, 'Europe', 1, 0, 0), (3, 'Empty', NULL, 0, 0);
                 UPDATE albums SET collection_id = 2 WHERE id = 9;
                 INSERT INTO albums (id, name, kind, collection_id, created_at, updated_at) VALUES (10, 'Road', 0, 1, 0, 0);
                 INSERT INTO album_items (album_id, asset_id, position, added_at) VALUES (10, 4, 1, 0), (10, 5, 2, 0);",
            )?;
            let in_collection = |ids: &[i64]| AssetQuery { collection_ids: ids.to_vec(), ..Default::default() };
            assert_eq!(ids(c, in_collection(&[2])), vec![2, 5], "Europe holds its own album");
            assert_eq!(ids(c, in_collection(&[1])), vec![2, 4, 5], "Trips holds its album and Europe's, each photo once");
            assert!(ids(c, in_collection(&[3])).is_empty());
            assert_eq!(ids(c, in_collection(&[1, 2])), vec![2, 5], "two collections mean both");
            let old: AssetQuery = serde_json::from_str(r#"{"favorites":false}"#)?;
            assert!(old.collection_ids.is_empty(), "a query saved before has no such rule");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_combined_query_is_the_intersection() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            let vocab = crate::query_parse::Vocabulary::load(c)?;
            let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
            let parsed = crate::query_parse::parse("anna beach 2023", &vocab, today, 0, crate::settings::DateOrder::Dmy, |_| 1);
            assert_eq!(ids(c, parsed.query), vec![2]);
            let parsed = crate::query_parse::parse("ben portugal summer", &vocab, today, 0, crate::settings::DateOrder::Dmy, |_| 1);
            assert_eq!(ids(c, parsed.query), vec![2, 4]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_person_filter_reads_the_person_index() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            people_library(c)?;
            let (joins, wh, params) = AssetQuery { person_ids: vec![1], ..Default::default() }.sql_parts();
            let sql = format!("EXPLAIN QUERY PLAN SELECT a.id FROM assets a{joins} WHERE {}", wh.join(" AND "));
            let mut stmt = c.prepare(&sql)?;
            let plan: Vec<String> = stmt.query_map(rusqlite::params_from_iter(params), |r| r.get::<_, String>(3))?.collect::<std::result::Result<_, _>>()?;
            assert!(plan.iter().any(|p| p.contains("faces_person")), "{plan:?}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    #[ignore]
    fn a_combined_search_at_250k() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            crate::synthetic::generate(c, 250_000, 7)?;
            c.execute_batch(
                "INSERT INTO people (id, name, created_at) VALUES (1, 'Anna', 0);
                 INSERT INTO faces (asset_id, x, y, w, h, score, vec, model, person_id) SELECT id, 0, 0, .1, .1, .9, x'00', 'm', 1 FROM assets WHERE id % 50 = 0;
                 UPDATE assets SET scene_tags = CASE WHEN id % 7 = 0 THEN 'beach sunset' ELSE 'street' END;
                 ANALYZE;",
            )?;
            let q = AssetQuery {
                person_ids: vec![1],
                tags: vec!["beach".into()],
                date_from: Some(1_600_000_000),
                date_to: Some(1_700_000_000),
                ..Default::default()
            };
            let scope = crate::scope::LayoutScope { query: Some(q), ..Default::default() };
            crate::layout::count_scoped(c, &scope)?;
            let start = std::time::Instant::now();
            let n = crate::layout::count_scoped(c, &scope)?;
            let took = start.elapsed();
            println!("person + tag + dates at 250k: {n} in {took:?}");
            assert!(took < std::time::Duration::from_millis(10), "{took:?}");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn builds_match_expressions() {
        assert_eq!(AssetQuery::match_expression("lisbon"), "\"lisbon\"*");
        assert_eq!(AssetQuery::match_expression("  IMG 0042 "), "\"img\"* \"0042\"*");
        assert_eq!(AssetQuery::match_expression("o'neill \"x\""), "\"o\"* \"neill\"* \"x\"*");
        assert_eq!(AssetQuery::match_expression("  \"  "), "", "nothing left to match");
    }

    #[test]
    fn builds_clauses_and_parameters_in_order() {
        let q = AssetQuery {
            text: Some("beach".into()),
            min_rating: Some(3),
            device_ids: vec![2, 5],
            has_gps: Some(true),
            camera: Some("ILCE-7M4".into()),
            ..Default::default()
        };
        let (joins, wh, params) = q.sql_parts();
        assert!(joins.contains("assets_fts f ON f.rowid = a.id"));
        assert!(wh.iter().any(|w| w == "assets_fts MATCH ?"));
        assert!(wh.iter().any(|w| w == "a.device_id IN (?,?)"));
        assert!(wh.iter().any(|w| w == "a.rating >= ?"));
        assert!(wh.iter().any(|w| w == "a.gps_lat IS NOT NULL"));
        assert!(wh.iter().any(|w| w.contains("json_extract(a.metadata_json, '$.model')")));
        assert_eq!(params.len(), 5, "text, two devices, rating, camera");
        // Read text in photos is off by default: photo_text is left out of the match.
        assert_eq!(params[0], Value::Text("{filename device_name place_name keywords} : (\"beach\"*)".into()));
        assert_eq!(params[1], Value::Integer(2));
        assert_eq!(params[3], Value::Integer(3));
    }

    #[test]
    fn filters_by_one_extension_case_insensitively() {
        let q = AssetQuery { extension: Some(".NEF".into()), ..Default::default() };
        let (_, wh, params) = q.sql_parts();
        assert_eq!(wh, vec!["a.extension = ?".to_string()]);
        assert_eq!(params, vec![Value::Text("nef".into())]);
        let blank = AssetQuery { extension: Some("  ".into()), ..Default::default() };
        assert!(blank.sql_parts().1.is_empty());
    }

    #[test]
    fn an_empty_query_adds_nothing() {
        let (joins, wh, params) = AssetQuery::default().sql_parts();
        assert!(joins.is_empty() && wh.is_empty() && params.is_empty());
        assert!(AssetQuery::default().is_empty());
        assert!(!AssetQuery { favorites: true, ..Default::default() }.is_empty());
        // Blank text is not a filter.
        assert!(AssetQuery { text: Some("   ".into()), ..Default::default() }.sql_parts().1.is_empty());
        // Rating 0 means "any", not "at least zero".
        assert!(AssetQuery { min_rating: Some(0), ..Default::default() }.sql_parts().1.is_empty());
    }

    #[test]
    fn round_trips_through_json() {
        let q = AssetQuery { text: Some("lisbon".into()), device_ids: vec![1], favorites: true, ..Default::default() };
        let json = serde_json::to_string(&q).unwrap();
        assert_eq!(serde_json::from_str::<AssetQuery>(&json).unwrap(), q);
        // Missing fields default, so a query saved by an older build still loads.
        let old: AssetQuery = serde_json::from_str(r#"{"text":"beach"}"#).unwrap();
        assert_eq!(old.text.as_deref(), Some("beach"));
        assert!(old.device_ids.is_empty());
    }

    fn sign_library(c: &rusqlite::Connection) -> crate::Result<()> {
        c.execute_batch(
            "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at)
               VALUES (1, 'h1', 'p', 'IMG_0001.jpg', 'jpg', 0, 1, 1, 1), (2, 'h2', 'p', 'IMG_0002.jpg', 'jpg', 0, 1, 1, 1);
             INSERT INTO asset_text (asset_id, ord, text, conf, quad, script) VALUES (1, 0, 'Gostilna MENU', 0.9, x'00', 'Latn');",
        )?;
        crate::fts::index_ids(c, &[1, 2])
    }

    /// Review focus 2: with the switch off, a word only on a sign finds nothing, and the
    /// rows stay so turning it back on needs no rescan.
    #[test]
    fn photo_text_is_left_out_when_off() {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            sign_library(c)?;
            let q = |on: bool| AssetQuery { text: Some("menu".into()), photo_text: on, ..Default::default() };
            assert!(ids(c, q(false)).is_empty());
            assert_eq!(ids(c, q(true)), vec![1]);
            let lines: i64 = c.query_row("SELECT count(*) FROM asset_text", [], |r| r.get(0))?;
            assert_eq!(lines, 1);
            // The other columns still match with the switch off, in both shapes of the clause.
            assert_eq!(ids(c, AssetQuery { text: Some("img 0002".into()), ..Default::default() }), vec![2]);
            let by_meaning = AssetQuery { text: Some("menu".into()), semantic: Some(vec![2]), ..Default::default() };
            assert_eq!(ids(c, by_meaning.clone()), vec![2]);
            assert_eq!(ids(c, AssetQuery { photo_text: true, ..by_meaning }), vec![1, 2]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn photo_text_does_not_make_a_query_non_empty() {
        assert!(AssetQuery { photo_text: true, ..Default::default() }.is_empty());
    }
}
