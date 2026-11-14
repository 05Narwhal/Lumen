//! Export: what the user asked for, and what the exported file is called.

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ExportFormat {
    Jpeg,
    Png,
    Webp,
    Avif,
    Heic,
    /// Copies made for an external editor (Round 22 §8); the Export dialog does not offer it.
    Tiff,
    /// The imported file as it is. An edited photo cannot be one, so it becomes a JPEG.
    Original,
}

impl ExportFormat {
    /// AVIF and HEIC are encoded by FFmpeg; without it they cannot be written.
    pub fn needs_ffmpeg(self) -> bool {
        matches!(self, ExportFormat::Avif | ExportFormat::Heic)
    }

    /// The formats a quality applies to.
    pub fn is_lossy(self) -> bool {
        matches!(self, ExportFormat::Jpeg | ExportFormat::Webp | ExportFormat::Avif | ExportFormat::Heic)
    }
}

/// Where exported files go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[ts(export)]
pub enum ExportDestination {
    /// A folder; `None` is the library's `exports` folder.
    Folder { dir: Option<String> },
    /// Back into the library as new photos, each remembering what it was exported from, and
    /// into an album as well when one is given (Round 5).
    Library {
        #[serde(default, rename = "albumId")]
        #[ts(optional, type = "number")]
        album_id: Option<i64>,
    },
}

impl Default for ExportDestination {
    fn default() -> Self {
        ExportDestination::Folder { dir: None }
    }
}

/// How a trimmed video is cut (spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum VideoCut {
    /// Re-encoded, so every cut is on the frame asked for.
    #[default]
    Precise,
    /// Streams copied, cut at key frames: nothing re-encoded, nearly instant.
    Fast,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportOptions {
    pub format: ExportFormat,
    /// The lossy formats, 60–100.
    pub quality: u8,
    /// WebP only.
    pub lossless: bool,
    /// `None` exports at full size.
    pub max_long_edge: Option<u32>,
    /// Carry the original's metadata into the export and verify it (spec §5). On by default.
    pub metadata: bool,
    pub destination: ExportDestination,
    /// Trimmed videos only.
    pub video_cut: VideoCut,
}

/// What the frontend sends, including the shape from before Phase 8b (`destDir`, no
/// `destination`, `lossless` or `metadata`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportOptionsWire {
    format: ExportFormat,
    quality: u8,
    #[serde(default)]
    lossless: bool,
    #[serde(default)]
    max_long_edge: Option<u32>,
    #[serde(default = "yes")]
    metadata: bool,
    #[serde(default)]
    destination: Option<ExportDestination>,
    #[serde(default)]
    dest_dir: Option<String>,
    #[serde(default)]
    video_cut: VideoCut,
}

fn yes() -> bool {
    true
}

impl<'de> Deserialize<'de> for ExportOptions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = ExportOptionsWire::deserialize(d)?;
        Ok(ExportOptions {
            format: w.format,
            quality: w.quality,
            lossless: w.lossless,
            max_long_edge: w.max_long_edge,
            metadata: w.metadata,
            destination: w.destination.unwrap_or(ExportDestination::Folder { dir: w.dest_dir }),
            video_cut: w.video_cut,
        })
    }
}

/// What Lumen knows about a photo that the exported file should say too (spec §5, step 3).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportMeta {
    /// 0–5 stars; 0 writes nothing.
    pub rating: i64,
    /// 0 none, 1 red … 5 purple (`xmp:Label`).
    pub color_label: i64,
    pub title: Option<String>,
    pub caption: Option<String>,
    pub keywords: Vec<String>,
}

/// Keywords as the catalogue keeps them: one per line (older rows joined them with spaces,
/// so a row without a line break is read a word at a time).
pub fn split_keywords(stored: &str) -> Vec<String> {
    let parts: Vec<&str> = if stored.contains('\n') { stored.lines().collect() } else { stored.split_whitespace().collect() };
    parts.into_iter().map(str::trim).filter(|k| !k.is_empty()).map(String::from).collect()
}

/// The photo's rating, label, title, caption and keywords, for the exported file.
pub fn export_meta(conn: &Connection, id: i64) -> crate::Result<ExportMeta> {
    let row = conn
        .query_row("SELECT rating, color_label, title, caption, keywords FROM assets WHERE id = ?1", [id], |r| {
            Ok(ExportMeta {
                rating: r.get(0)?,
                color_label: r.get(1)?,
                title: r.get(2)?,
                caption: r.get(3)?,
                keywords: r.get::<_, Option<String>>(4)?.as_deref().map(split_keywords).unwrap_or_default(),
            })
        })
        .optional()?;
    Ok(row.unwrap_or_default())
}

/// A tag the original had that the export does not carry the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LostTag {
    pub tag: String,
    pub before: String,
    /// Empty when the export has no such tag at all.
    pub after: String,
}

/// One exported file whose verification found something missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportLoss {
    #[ts(type = "number")]
    pub asset_id: i64,
    /// The exported file's name.
    pub file: String,
    pub lost: Vec<LostTag>,
}

/// What happened to a batch's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum MetadataOutcome {
    /// Carried and read back from every file (losses, if any, are listed).
    Verified,
    /// The user turned it off, or everything was a byte-for-byte copy.
    Skipped,
    /// ExifTool is not installed, so nothing could be carried.
    Unavailable,
}

/// JPEG quality for an edit exported as "Original": as close to the original as JPEG goes.
pub const EDITED_ORIGINAL_QUALITY: u8 = 95;

impl ExportOptions {
    /// The format actually written for a photo that is (or is not) edited.
    pub fn effective_format(&self, edited: bool) -> ExportFormat {
        match self.format {
            ExportFormat::Original if edited => ExportFormat::Jpeg,
            f => f,
        }
    }
}

/// The extension a format is written with.
pub fn extension(format: ExportFormat) -> Option<&'static str> {
    match format {
        ExportFormat::Jpeg => Some("jpg"),
        ExportFormat::Png => Some("png"),
        ExportFormat::Webp => Some("webp"),
        ExportFormat::Avif => Some("avif"),
        ExportFormat::Heic => Some("heic"),
        ExportFormat::Tiff => Some("tif"),
        ExportFormat::Original => None,
    }
}

/// `IMG_1.HEIC` exported as an edited JPEG is `IMG_1-edited.jpg`: the name says what the
/// file is, so an export never passes for the untouched original.
pub fn export_name(filename: &str, format: ExportFormat, edited: bool) -> String {
    let path = std::path::Path::new(filename);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| filename.to_string());
    let suffix = if edited { "-edited" } else { "" };
    let format = if format == ExportFormat::Original && edited { ExportFormat::Jpeg } else { format };
    match extension(format) {
        None => filename.to_string(),
        Some(ext) => format!("{stem}{suffix}.{ext}"),
    }
}

/// What a selection holds, so the export sheet can name it and offer only what applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExportSummary {
    pub photos: u32,
    pub videos: u32,
    /// Videos with a trim, exported as their kept segments.
    pub trimmed: u32,
    /// Photos with an edit, exported as edited.
    pub edited: u32,
}

pub fn export_summary(conn: &Connection, ids: &[i64]) -> crate::Result<ExportSummary> {
    let mut sum = ExportSummary::default();
    let mut stmt = conn.prepare("SELECT media_type FROM assets WHERE id = ?1 AND state = 0")?;
    for &id in ids {
        let Some(media_type) = stmt.query_row([id], |r| r.get::<_, i64>(0)).optional()? else { continue };
        // What the export renders: Develop's draft when there is one (Round 6).
        let recipe = crate::edits::export_recipe(conn, id)?.map(|(r, _)| r);
        if media_type == 1 {
            sum.videos += 1;
            if crate::edits::get_trim(conn, id)?.is_some() {
                sum.trimmed += 1;
            }
        } else {
            sum.photos += 1;
            if recipe.is_some_and(|r| r.changes_pixels()) {
                sum.edited += 1;
            }
        }
    }
    Ok(sum)
}

/// A trimmed video is `IMG_2.mov` → `IMG_2-trimmed.mov`: never mistaken for the whole clip.
pub fn trimmed_name(filename: &str, extension: &str) -> String {
    let stem = std::path::Path::new(filename).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| filename.to_string());
    format!("{stem}-trimmed.{extension}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_say_what_the_file_is() {
        assert_eq!(export_name("IMG_1.HEIC", ExportFormat::Jpeg, false), "IMG_1.jpg");
        assert_eq!(export_name("IMG_1.HEIC", ExportFormat::Jpeg, true), "IMG_1-edited.jpg");
        assert_eq!(export_name("DSC_0676.NEF", ExportFormat::Png, true), "DSC_0676-edited.png");
        assert_eq!(export_name("DSC_0676.NEF", ExportFormat::Webp, false), "DSC_0676.webp");
        assert_eq!(export_name("DSC_0676.NEF", ExportFormat::Avif, true), "DSC_0676-edited.avif");
        assert_eq!(export_name("IMG_2.jpg", ExportFormat::Heic, false), "IMG_2.heic");
        assert_eq!(export_name("DSC_0676.NEF", ExportFormat::Original, false), "DSC_0676.NEF");
        // An edit cannot live in an original file, so it is exported as a JPEG.
        assert_eq!(export_name("DSC_0676.NEF", ExportFormat::Original, true), "DSC_0676-edited.jpg");
        assert_eq!(export_name("no-extension", ExportFormat::Jpeg, false), "no-extension.jpg");
        assert_eq!(trimmed_name("IMG_2.MOV", "mov"), "IMG_2-trimmed.mov");
    }

    #[test]
    fn options_read_like_the_frontend_sends_them() {
        let o: ExportOptions = serde_json::from_str(
            r#"{"format":"webp","quality":90,"lossless":true,"maxLongEdge":2048,"metadata":false,"destination":{"kind":"library"}}"#,
        )
        .unwrap();
        assert_eq!(o.format, ExportFormat::Webp);
        assert!(o.lossless && !o.metadata);
        assert_eq!(o.max_long_edge, Some(2048));
        assert_eq!(o.destination, ExportDestination::Library { album_id: None });
        let o: ExportOptions = serde_json::from_str(r#"{"format":"jpeg","quality":90,"destination":{"kind":"library","albumId":3}}"#).unwrap();
        assert_eq!(o.destination, ExportDestination::Library { album_id: Some(3) });
        let o: ExportOptions = serde_json::from_str(r#"{"format":"avif","quality":80,"destination":{"kind":"folder","dir":"D:/out"}}"#).unwrap();
        assert_eq!(o.destination, ExportDestination::Folder { dir: Some("D:/out".into()) });
        assert!(o.metadata, "metadata is carried unless the user says otherwise");
        assert_eq!(o.video_cut, VideoCut::Precise);
        let o: ExportOptions = serde_json::from_str(r#"{"format":"original","quality":90,"videoCut":"fast"}"#).unwrap();
        assert_eq!(o.video_cut, VideoCut::Fast);
    }

    #[test]
    fn options_from_before_8b_still_parse() {
        let o: ExportOptions = serde_json::from_str(r#"{"format":"png","quality":90,"maxLongEdge":2048,"destDir":null}"#).unwrap();
        assert_eq!(o.format, ExportFormat::Png);
        assert_eq!(o.destination, ExportDestination::Folder { dir: None });
        assert!(o.metadata && !o.lossless);
        assert_eq!(o.effective_format(true), ExportFormat::Png);
        let o: ExportOptions = serde_json::from_str(r#"{"format":"original","quality":90,"maxLongEdge":null,"destDir":"E:/x"}"#).unwrap();
        assert_eq!(o.destination, ExportDestination::Folder { dir: Some("E:/x".into()) });
        assert_eq!(o.effective_format(true), ExportFormat::Jpeg);
        assert_eq!(o.effective_format(false), ExportFormat::Original);
    }

    #[test]
    fn a_summary_counts_what_the_sheet_needs() {
        use crate::edits::{save_recipe, Recipe, Segment, Trim};
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, byte_size, media_type, imported_at) VALUES
                 (1, 'a', 'a', 'a.jpg', 'jpg', 1, 0, 0), (2, 'b', 'b', 'b.jpg', 'jpg', 1, 0, 0),
                 (3, 'c', 'c', 'c.mov', 'mov', 1, 1, 0), (4, 'd', 'd', 'd.mov', 'mov', 1, 1, 0)",
            )?;
            save_recipe(c, 2, &Recipe { rotate: 90, ..Recipe::identity() }, 1)?;
            save_recipe(c, 3, &Recipe { trim: Some(Trim { segments: vec![Segment { start_ms: 0, end_ms: 500 }] }), ..Recipe::identity() }, 1)?;
            assert_eq!(export_summary(c, &[1, 2, 3, 4, 99])?, ExportSummary { photos: 2, videos: 2, trimmed: 1, edited: 1 });
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn keywords_split_by_line_and_old_rows_by_word() {
        assert_eq!(split_keywords("New York\nbeach\n\n"), ["New York", "beach"]);
        assert_eq!(split_keywords("beach  Anna"), ["beach", "Anna"]);
        assert!(split_keywords(" ").is_empty());
    }

    #[test]
    fn export_meta_reads_the_catalogue() {
        let db = crate::db::Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            c.execute_batch(
                "INSERT INTO assets (id, content_hash, relative_path, filename, extension, byte_size, media_type, imported_at, rating, color_label, title, keywords)
                 VALUES (7, 'h', 'a/b.jpg', 'b.jpg', 'jpg', 1, 0, 0, 4, 2, 'Harbour', 'New York\nboat')",
            )?;
            let m = export_meta(c, 7)?;
            assert_eq!(m, ExportMeta { rating: 4, color_label: 2, title: Some("Harbour".into()), caption: None, keywords: vec!["New York".into(), "boat".into()] });
            assert_eq!(export_meta(c, 99)?, ExportMeta::default());
            Ok(())
        })
        .unwrap();
    }
}
