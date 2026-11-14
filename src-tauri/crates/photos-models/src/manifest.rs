//! The compiled-in list of models: where each is fetched from, how it is verified, and the
//! preprocessing it expects. Written by `scripts/ml/export_models.py`.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub release: String,
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    /// What the app calls the model (`"Lumen Ray-high 1.0"`); Lumen's own models only.
    #[serde(default)]
    pub name: Option<String>,
    pub role: String,
    /// A file name, never a path: the store joins it under the models directory.
    pub file: String,
    pub bytes: u64,
    pub sha256: String,
    pub licence: String,
    #[serde(default)]
    pub input: Option<ClipInput>,
    #[serde(default)]
    pub aesthetic: Option<Aesthetic>,
    #[serde(default)]
    pub audio: Option<AudioSpec>,
    /// Face models' thresholds (`score`, `nms`, `min_side_px`, `size`, `same_person`).
    #[serde(default)]
    pub faces: Option<serde_json::Value>,
    /// The text tower's context length and the image model it pairs with.
    #[serde(default)]
    pub text: Option<serde_json::Value>,
    /// How the denoise model is tiled (Phase 8d).
    #[serde(default)]
    pub denoise: Option<DenoiseSpec>,
    /// The pet detector's settings (`size`, `score`, `min_side_px`; Round 4C).
    #[serde(default)]
    pub pets: Option<serde_json::Value>,
    /// Lumen's own Retouch model's ONNX contract (Round 12; dev entries only).
    #[serde(default)]
    pub retouch: Option<RetouchSpec>,
    /// How a model Lumen trained was made (`run`, `val_de`, `sources`): shown, never used.
    #[serde(default)]
    pub trained: Option<serde_json::Value>,
    /// The text detector's post-processing (Round 13; PaddleOCR's `text_det_*`).
    #[serde(default)]
    pub ocr_det: Option<OcrDetSpec>,
    /// The text recogniser's input size and dictionary (Round 13).
    #[serde(default)]
    pub ocr_rec: Option<OcrRecSpec>,
}

/// PaddleOCR `DBPostProcess` settings; `Default` is PaddleOCR's own `text_det_*` defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrDetSpec {
    pub thresh: f32,
    pub box_thresh: f32,
    pub unclip_ratio: f32,
    pub max_candidates: usize,
    pub min_side: f32,
}

impl Default for OcrDetSpec {
    fn default() -> Self {
        Self { thresh: 0.3, box_thresh: 0.6, unclip_ratio: 1.5, max_candidates: 1000, min_side: 3.0 }
    }
}

/// The recogniser's input height, its base width, and its dictionary (`ppocr_keys` lines,
/// without the CTC blank and the trailing space the decoder adds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrRecSpec {
    pub height: u32,
    pub base_width: u32,
    pub chars: Vec<String>,
}

/// The Retouch model's inputs and output (Round 12 spec §4): `image` is `size`² with the
/// channels `photos_media::retouch_input::prepare` makes, `layout` the parameter layout of
/// `v` (`photos_core::retouch::RETOUCH_LAYOUT` when made for this Lumen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetouchSpec {
    pub size: u32,
    pub layout: u32,
    pub inputs: Vec<String>,
    pub output: String,
}

/// A denoise model's tiling: `tile`-pixel squares overlapping by `overlap`, padded to a
/// multiple of `multiple`, fed `input`-encoded values (sRGB 0–1).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct DenoiseSpec {
    pub tile: u32,
    pub overlap: u32,
    pub multiple: u32,
    pub input: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ClipInput {
    pub size: u32,
    pub mean: [f32; 3],
    pub std: [f32; 3],
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Aesthetic {
    pub weights: Vec<f32>,
    pub bias: f32,
    pub offset: f32,
    pub scale: f32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub n_fft: usize,
    pub hop: usize,
    pub n_mels: usize,
    pub fmin: f32,
    pub fmax: f32,
    pub mel_scale: String,
    pub norm: String,
    /// torchaudio's `normalized="frame_length"`: magnitudes divided by √n_fft.
    pub frame_length_norm: bool,
    pub power: f32,
    pub log_multiplier: f32,
    pub chunk_frames: usize,
    pub border_frames: usize,
    pub beat_output: String,
    pub downbeat_output: String,
}

impl ModelEntry {
    /// The model's name for people: its `name`, else its id.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Roles fetched only once their Experimental switch is on (Facet, Phase 8f). Setup leaves
/// them out of its default download.
pub const EXPERIMENTAL_ROLES: &[&str] = &["segmentation"];

impl Manifest {
    /// The models every install fetches: everything not behind an Experimental switch.
    pub fn default_set(&self) -> impl Iterator<Item = &ModelEntry> {
        self.models.iter().filter(|m| !EXPERIMENTAL_ROLES.contains(&m.role.as_str()))
    }

    /// Setup's Options size: the bytes of [`Manifest::default_set`].
    pub fn default_bytes(&self) -> u64 {
        self.default_set().map(|m| m.bytes).sum()
    }

    pub fn by_role(&self, role: &str) -> Option<&ModelEntry> {
        self.models.iter().find(|m| m.role == role)
    }

    pub fn url_of(&self, e: &ModelEntry) -> String {
        format!("{}{}", self.release, e.file)
    }
}

pub fn manifest() -> &'static Manifest {
    static M: OnceLock<Manifest> = OnceLock::new();
    M.get_or_init(|| serde_json::from_str(include_str!("../models.json")).expect("models.json is valid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiled_in_manifest_names_files_not_paths() {
        let m = manifest();
        let clip = m.by_role("image_embedding").expect("an image model");
        assert_eq!(clip.input.as_ref().unwrap().size, 224);
        assert!(!clip.aesthetic.as_ref().unwrap().weights.is_empty());
        for e in &m.models {
            assert!(!e.file.contains('/') && !e.file.contains('\\'), "{} is a path", e.file);
            assert_eq!(e.sha256.len(), 64, "{}", e.id);
            assert!(e.bytes > 0, "{}", e.id);
        }
        if let Some(beat) = m.by_role("beat_activation") {
            let a = beat.audio.as_ref().unwrap();
            assert!(a.chunk_frames > 2 * a.border_frames);
        }
        // Phase 8d: tiles overlap on both sides and are a size the network accepts.
        let dn = m.by_role("denoise").expect("a denoise model").denoise.as_ref().expect("its tiling");
        assert!(dn.tile > 2 * dn.overlap);
        assert_eq!(dn.tile % dn.multiple, 0);
        assert_eq!(dn.input, "srgb");
    }

    #[test]
    fn default_set_leaves_out_segmentation() {
        let m = manifest();
        assert!(m.by_role("segmentation").is_some(), "Facet is in the list");
        assert!(m.default_set().all(|e| e.role != "segmentation"));
        assert_eq!(m.default_set().count(), m.models.iter().filter(|e| e.role != "segmentation").count());
    }

    #[test]
    fn default_bytes_sums_default_set() {
        let m = manifest();
        let all: u64 = m.models.iter().map(|e| e.bytes).sum();
        let facet: u64 = m.models.iter().filter(|e| e.role == "segmentation").map(|e| e.bytes).sum();
        assert_eq!(m.default_bytes(), all - facet);
    }

    #[test]
    fn a_dev_entry_carries_its_retouch_contract_and_training_notes() {
        let e: ModelEntry = serde_json::from_str(
            r#"{ "id": "retouch-dev-2026-10-12-r2", "role": "retouch", "file": "retouch-dev.onnx", "bytes": 10,
                 "sha256": "ab", "licence": "Lumen (own weights)",
                 "retouch": { "size": 256, "layout": 1, "inputs": ["image", "clip", "clip_flag"], "output": "v" },
                 "trained": { "run": "r2", "val_de": 3.81, "sources": ["contributor-finals"] } }"#,
        )
        .unwrap();
        assert_eq!(e.label(), "retouch-dev-2026-10-12-r2", "unnamed: the id");
        let r = e.retouch.unwrap();
        assert_eq!((r.size, r.layout, r.output.as_str()), (256, 1, "v"));
        assert_eq!(r.inputs, ["image", "clip", "clip_flag"]);
        assert_eq!(e.trained.unwrap()["val_de"], 3.81);
        assert!(manifest().models.iter().all(|m| m.retouch.is_none() && m.trained.is_none()));
    }

    const OCR_BASE: &str = r#""role": "ocr_recognition", "file": "ocr_recognition.onnx", "bytes": 10, "sha256": "ab", "licence": "Apache-2.0""#;

    #[test]
    fn old_entries_have_no_ocr_specs() {
        let e: ModelEntry = serde_json::from_str(&format!(r#"{{ "id": "x@1", {OCR_BASE} }}"#)).unwrap();
        assert!(e.ocr_det.is_none() && e.ocr_rec.is_none());
        assert!(manifest().models.iter().all(|m| m.ocr_det.is_none() && m.ocr_rec.is_none()));
    }

    #[test]
    fn ocr_specs_round_trip() {
        let e: ModelEntry = serde_json::from_str(&format!(
            r#"{{ "id": "pp-ocrv5-latin-mobile-rec@1", {OCR_BASE},
                 "ocr_rec": {{ "height": 48, "base_width": 320, "chars": ["a", "č"] }},
                 "ocr_det": {{ "thresh": 0.3, "box_thresh": 0.6, "unclip_ratio": 1.5, "max_candidates": 1000, "min_side": 3.0 }} }}"#
        ))
        .unwrap();
        let rec = e.ocr_rec.unwrap();
        assert_eq!((rec.height, rec.base_width), (48, 320));
        assert_eq!(rec.chars, ["a", "č"]);
        let back: OcrRecSpec = serde_json::from_value(serde_json::to_value(&rec).unwrap()).unwrap();
        assert_eq!(back, rec);
        assert_eq!(e.ocr_det.unwrap(), OcrDetSpec::default(), "PaddleOCR's text_det_* defaults");
        let det: OcrDetSpec = serde_json::from_str(&serde_json::to_string(&OcrDetSpec::default()).unwrap()).unwrap();
        assert_eq!(det, OcrDetSpec::default());
    }

    #[test]
    fn a_named_entry_is_labelled_by_its_name() {
        let e: ModelEntry = serde_json::from_str(
            r#"{ "id": "lumen-ray-1.0-high", "name": "Lumen Ray-high 1.0", "role": "retouch", "file": "r.onnx",
                 "bytes": 10, "sha256": "ab", "licence": "Lumen" }"#,
        )
        .unwrap();
        assert_eq!(e.label(), "Lumen Ray-high 1.0");
    }
}
