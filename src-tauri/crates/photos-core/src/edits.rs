//! Non-destructive edits: the recipe, the frame it produces, and where it is kept.
//!
//! The recipe is a domain value rather than a renderer detail because the justified layout
//! must know an edited photo's shape. Rendering it is `photos_media::develop`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ai_masks::AiMaskKind;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Crop {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Spec §12, version 2 (Phase 8 spec §2). Field names are the spec's JSON (snake_case), so
/// a stored recipe reads the same in the database, in a bug report and in TypeScript.
///
/// Geometry stays flat on the recipe — the layout, the renderer and every stored recipe use
/// it — and the Develop adjustments and a video's trim sit beside it. A version-1 recipe
/// parses: its tone fields become `develop.basic` (see `RecipeWire`).
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Recipe {
    pub version: u32,
    pub crop: Option<Crop>,
    pub rotate: u16,
    pub straighten: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    pub develop: Option<Develop>,
    pub trim: Option<Trim>,
}

/// White-balance gains for temperature and tint (±100 each), normalised so white keeps its
/// luminance: a von Kries adaptation in the working space.
pub fn wb_gains(temperature: f32, tint: f32) -> [f32; 3] {
    let t = temperature / 100.0;
    let m = tint / 100.0;
    let g = [2f32.powf(0.35 * t), 2f32.powf(-0.25 * m), 2f32.powf(-0.35 * t)];
    let y = g[0] * 0.2126 + g[1] * 0.7152 + g[2] * 0.0722;
    [g[0] / y, g[1] / y, g[2] / y]
}

/// The temperature and tint whose gains make `mean` grey (the inverse of `wb_gains`; its
/// luminance normalisation cancels out of these log ratios).
/// log2(gR/gB) = 0.7·t/100, log2 gG − (log2 gR + log2 gB)/2 = −0.25·m/100.
pub fn wb_from_grey(mean: [f32; 3]) -> (f32, f32) {
    let l = |v: f32| v.max(1e-6).log2();
    let (r, g, b) = (-l(mean[0]), -l(mean[1]), -l(mean[2])); // the gains wanted, in stops
    let t = (r - b) / 0.7 * 100.0;
    let m = -(g - (r + b) / 2.0) / 0.25 * 100.0;
    (t.clamp(-100.0, 100.0), m.clamp(-100.0, 100.0))
}

/// The Basic panel. Ranges: exposure in stops (±5), temperature and tint ±100, the rest ±1
/// (the panel shows them ×100, as Lightroom does).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Basic {
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub vibrance: f32,
    pub saturation: f32,
}

/// Point curves, each a list of `[x, y]` in 0–1 sorted by x. Empty is the identity.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Curve {
    #[ts(type = "Array<[number, number]>")]
    pub master: Vec<[f32; 2]>,
    #[ts(type = "Array<[number, number]>")]
    pub r: Vec<[f32; 2]>,
    #[ts(type = "Array<[number, number]>")]
    pub g: Vec<[f32; 2]>,
    #[ts(type = "Array<[number, number]>")]
    pub b: Vec<[f32; 2]>,
}

/// One HSL band's hue shift, saturation and luminance, each ±1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Hsl {
    pub h: f32,
    pub s: f32,
    pub l: f32,
}

/// The eight HSL bands in order: red, orange, yellow, green, aqua, blue, purple, magenta.
pub const HSL_BANDS: [&str; 8] = ["red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"];

/// A colour-grading wheel: hue in degrees, saturation 0–1, luminance ±1.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Wheel {
    pub hue: f32,
    pub sat: f32,
    pub lum: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Grading {
    pub shadows: Wheel,
    pub midtones: Wheel,
    pub highlights: Wheel,
    /// How far the three ranges overlap, 0–1.
    pub blending: f32,
    /// Moves the shadow/highlight split, ±1.
    pub balance: f32,
}

impl Default for Grading {
    fn default() -> Self {
        Self { shadows: Wheel::default(), midtones: Wheel::default(), highlights: Wheel::default(), blending: 0.5, balance: 0.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Sharpen {
    /// 0–1.5.
    pub amount: f32,
    /// Pixels, 0.5–3.
    pub radius: f32,
    /// 0–1: how much fine detail is sharpened against edges.
    pub detail: f32,
    /// 0–1: how much of the flat areas is left alone.
    pub masking: f32,
}

impl Default for Sharpen {
    fn default() -> Self {
        Self { amount: 0.0, radius: 1.0, detail: 0.25, masking: 0.0 }
    }
}

/// On-demand ML denoise: the model's output is cached and blended in at `strength` (0–1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Denoise {
    pub strength: f32,
    pub model: String,
}

impl Default for Denoise {
    fn default() -> Self {
        Self { strength: 0.5, model: String::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Detail {
    pub sharpen: Sharpen,
    pub denoise: Option<Denoise>,
}

/// Post-crop vignette: amount ±1 (negative darkens), midpoint and feather 0–1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Vignette {
    pub amount: f32,
    pub midpoint: f32,
    pub feather: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self { amount: 0.0, midpoint: 0.5, feather: 0.5 }
    }
}

/// Film grain: amount and size, 0–1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Grain {
    pub amount: f32,
    pub size: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self { amount: 0.0, size: 0.25 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Effects {
    pub vignette: Vignette,
    pub grain: Grain,
    // Left out while off, so a recipe from before them writes back byte for byte.
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<MotionBlur>")]
    pub motion_blur: MotionBlur,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Glow>")]
    pub glow: Glow,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Halation>")]
    pub halation: Halation,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

/// Motion blur: a streak `amount` 0–1 long (up to 6 % of the frame's long side) along
/// `angle` degrees, 0 horizontal, 90 vertical (−90–90).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct MotionBlur {
    pub amount: f32,
    pub angle: f32,
}

/// Glow: the highlights bloom into their surroundings, `amount` and `size` 0–1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Glow {
    pub amount: f32,
    pub size: f32,
}

impl Default for Glow {
    fn default() -> Self {
        Self { amount: 0.0, size: 0.4 }
    }
}

/// Halation: film's warm red fringe around bright highlights, `amount` and `size` 0–1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Halation {
    pub amount: f32,
    pub size: f32,
}

impl Default for Halation {
    fn default() -> Self {
        Self { amount: 0.0, size: 0.3 }
    }
}

/// Everything the Develop screen sets. Every field defaults, so a partial document parses.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Develop {
    pub basic: Basic,
    pub curve: Curve,
    pub hsl: [Hsl; 8],
    pub grading: Grading,
    pub detail: Detail,
    pub effects: Effects,
    /// The Basic values came from *Auto*.
    pub auto: bool,
    /// Local adjustments (recipe v3). Left out of the JSON when there are none, so a v2
    /// recipe writes back byte for byte.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[ts(as = "Option<Vec<Mask>>", optional)]
    pub masks: Vec<Mask>,
}

/// Where a mask's points are quantised to: 0–`MASK_GRID` of the unedited photo's sides.
pub const MASK_GRID: u16 = 4096;

/// A local adjustment: where (`shapes`, combined in order, then `invert` and `opacity`) and
/// what (`adjust`). Shapes are in the coordinates of the unedited, display-oriented photo
/// (0–1), so a crop, a turn or a flip never moves a mask off its pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Mask {
    pub id: u32,
    pub name: String,
    pub shapes: Vec<MaskShape>,
    #[serde(default)]
    pub invert: bool,
    /// 0–1.
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub adjust: LocalAdjust,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

/// One part of a mask. `add: false` subtracts it from what the shapes before it made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[ts(export)]
pub enum MaskShape {
    /// One stroke: dabs along `points` (0–`MASK_GRID`). `radius` is a share of the photo's
    /// long edge; inside `radius·(1 − feather)` a dab is full; `flow` is each dab's strength.
    Brush {
        add: bool,
        radius: f32,
        feather: f32,
        flow: f32,
        #[ts(type = "Array<[number, number]>")]
        points: Vec<[u16; 2]>,
    },
    /// Full effect at `from`, none at `to` (0–1 of the photo; either may lie outside it).
    Linear {
        add: bool,
        #[ts(type = "[number, number]")]
        from: [f32; 2],
        #[ts(type = "[number, number]")]
        to: [f32; 2],
    },
    /// An ellipse: `radius` as shares of the photo's width and height, turned by `angle`
    /// degrees, full inside `radius·(1 − feather)`; `inside: false` affects the outside.
    Radial {
        add: bool,
        #[ts(type = "[number, number]")]
        center: [f32; 2],
        #[ts(type = "[number, number]")]
        radius: [f32; 2],
        angle: f32,
        feather: f32,
        inside: bool,
    },
    /// What Lumen Facet found on this photo (Phase 8f): `raster` is its hash in `ai_masks`, kept
    /// out of the recipe because recipes cross `invoke` and sit in every history step. `target`
    /// is the spec's `kind`, a name `MaskShape`'s tag already holds.
    Ai {
        add: bool,
        target: AiMaskKind,
        /// Which person, from 0; 0 for the other targets.
        instance: u32,
        raster: String,
    },
}

/// What a mask does where it applies: Basic's ranges (temperature and tint ±100, exposure
/// ±5 EV, the rest ±1), plus `sharpness`, added to the global sharpening amount, and (Round
/// 19) every other tool that can be local, in the global panels' ranges. The Round 19 fields
/// are left out of the JSON while default, so a mask written before them writes back byte
/// for byte.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct LocalAdjust {
    pub temperature: f32,
    pub tint: f32,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub saturation: f32,
    pub sharpness: f32,
    /// ±1, as Basic's.
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<f32>")]
    pub vibrance: f32,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Curve>")]
    pub curve: Curve,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<[Hsl; 8]>")]
    pub hsl: [Hsl; 8],
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Grading>")]
    pub grading: Grading,
    /// The mask's own sharpening radius, detail and masking; `None` sharpens it as the photo.
    /// The amount is `sharpness`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub sharpen: Option<LocalSharpen>,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Grain>")]
    pub grain: Grain,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Glow>")]
    pub glow: Glow,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<Halation>")]
    pub halation: Halation,
    #[serde(skip_serializing_if = "is_default")]
    #[ts(optional, as = "Option<MotionBlur>")]
    pub motion_blur: MotionBlur,
}

/// A mask's sharpening, but its amount (`LocalAdjust::sharpness`): `Sharpen`'s other fields
/// and ranges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct LocalSharpen {
    /// Pixels, 0.5–3.
    pub radius: f32,
    /// 0–1.
    pub detail: f32,
    /// 0–1.
    pub masking: f32,
}

impl Default for LocalSharpen {
    fn default() -> Self {
        let s = Sharpen::default();
        Self { radius: s.radius, detail: s.detail, masking: s.masking }
    }
}

impl LocalAdjust {
    pub fn normalized(self) -> Self {
        let u = |v: f32| cl(v, -1.0, 1.0, 0.0);
        Self {
            temperature: cl(self.temperature, -100.0, 100.0, 0.0),
            tint: cl(self.tint, -100.0, 100.0, 0.0),
            exposure: cl(self.exposure, -5.0, 5.0, 0.0),
            contrast: u(self.contrast),
            highlights: u(self.highlights),
            shadows: u(self.shadows),
            whites: u(self.whites),
            blacks: u(self.blacks),
            texture: u(self.texture),
            clarity: u(self.clarity),
            dehaze: u(self.dehaze),
            saturation: u(self.saturation),
            sharpness: u(self.sharpness),
            vibrance: u(self.vibrance),
            curve: norm_curve(self.curve),
            hsl: norm_hsl(self.hsl),
            grading: norm_grading(self.grading),
            sharpen: self.sharpen.map(|s| {
                let n = norm_sharpen(Sharpen { amount: 0.0, radius: s.radius, detail: s.detail, masking: s.masking });
                LocalSharpen { radius: n.radius, detail: n.detail, masking: n.masking }
            }),
            grain: norm_grain(self.grain),
            glow: norm_glow(self.glow),
            halation: norm_halation(self.halation),
            motion_blur: norm_motion_blur(self.motion_blur),
        }
    }

    /// Changes no pixel.
    pub fn is_zero(&self) -> bool {
        Self { sharpen: None, ..self.clone() } == Self::default()
    }

    /// The Basic panel these values amount to: its tone and colour sliders, vibrance included.
    pub fn basic(&self) -> Basic {
        Basic {
            temperature: self.temperature,
            tint: self.tint,
            exposure: self.exposure,
            contrast: self.contrast,
            highlights: self.highlights,
            shadows: self.shadows,
            whites: self.whites,
            blacks: self.blacks,
            texture: self.texture,
            clarity: self.clarity,
            dehaze: self.dehaze,
            vibrance: self.vibrance,
            saturation: self.saturation,
        }
    }
}

impl MaskShape {
    pub fn normalized(self) -> Option<Self> {
        let p = |v: [f32; 2]| [cl(v[0], -1.0, 2.0, 0.5), cl(v[1], -1.0, 2.0, 0.5)];
        Some(match self {
            MaskShape::Brush { add, radius, feather, flow, mut points } => {
                if points.is_empty() {
                    return None;
                }
                points.truncate(20_000);
                for q in &mut points {
                    *q = [q[0].min(MASK_GRID), q[1].min(MASK_GRID)];
                }
                MaskShape::Brush { add, radius: cl(radius, 0.001, 0.5, 0.05), feather: cl(feather, 0.0, 1.0, 0.5), flow: cl(flow, 0.0, 1.0, 1.0), points }
            }
            MaskShape::Linear { add, from, to } => MaskShape::Linear { add, from: p(from), to: p(to) },
            MaskShape::Radial { add, center, radius, angle, feather, inside } => MaskShape::Radial {
                add,
                center: p(center),
                radius: [cl(radius[0], 0.001, 2.0, 0.25), cl(radius[1], 0.001, 2.0, 0.25)],
                angle: if angle.is_finite() { angle.rem_euclid(360.0) } else { 0.0 },
                feather: cl(feather, 0.0, 1.0, 0.5),
                inside,
            },
            MaskShape::Ai { add, target, instance, raster } => {
                if raster.is_empty() || raster.len() > 64 || !raster.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                MaskShape::Ai { add, target, instance, raster }
            }
        })
    }
}

impl Mask {
    pub fn normalized(self) -> Self {
        Self {
            id: self.id,
            name: self.name.chars().take(64).collect(),
            shapes: self.shapes.into_iter().take(500).filter_map(MaskShape::normalized).collect(),
            invert: self.invert,
            opacity: cl(self.opacity, 0.0, 1.0, 1.0),
            adjust: self.adjust.normalized(),
            visible: self.visible,
        }
    }

    /// Whether rendering it changes any pixel: shown, with something to do, somewhere.
    pub fn is_effective(&self) -> bool {
        self.is_effective_with(&|_| true)
    }

    /// [`Self::is_effective`] knowing which rasters the photo has: a shape whose raster it lacks
    /// weighs nothing, as if it weren't there.
    pub fn is_effective_with(&self, has_raster: &dyn Fn(&str) -> bool) -> bool {
        let weighs = |s: &MaskShape| !matches!(s, MaskShape::Ai { raster, .. } if !has_raster(raster));
        self.visible && self.opacity > 0.0 && !self.adjust.is_zero() && (self.shapes.iter().any(weighs) || self.invert)
    }
}

/// A kept stretch of a video, in milliseconds from its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Segment {
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
}

/// A video's trim: the segments that are kept, in order. Cutting a range out of the middle
/// leaves two.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct Trim {
    pub segments: Vec<Segment>,
}

impl Trim {
    /// Sorted, non-empty, non-overlapping (touching or overlapping segments merge).
    pub fn normalized(self) -> Self {
        let mut segs: Vec<Segment> =
            self.segments.into_iter().map(|s| Segment { start_ms: s.start_ms.max(0), end_ms: s.end_ms }).filter(|s| s.end_ms > s.start_ms).collect();
        segs.sort_by_key(|s| s.start_ms);
        let mut out: Vec<Segment> = Vec::with_capacity(segs.len());
        for s in segs {
            match out.last_mut() {
                Some(last) if s.start_ms <= last.end_ms => last.end_ms = last.end_ms.max(s.end_ms),
                _ => out.push(s),
            }
        }
        Self { segments: out }
    }

    /// How long the kept segments play for.
    pub fn kept_ms(&self) -> i64 {
        self.segments.iter().map(|s| s.end_ms - s.start_ms).sum()
    }
}

/// What is on disk: a v2 recipe, or a v1 one with its tone fields at the top level.
#[derive(Deserialize, Default)]
#[serde(default)]
struct RecipeWire {
    crop: Option<Crop>,
    rotate: u16,
    straighten: f32,
    flip_h: bool,
    flip_v: bool,
    develop: Option<Develop>,
    trim: Option<Trim>,
    // Version 1.
    exposure: Option<f32>,
    contrast: Option<f32>,
    saturation: Option<f32>,
    temperature: Option<f32>,
    tint: Option<f32>,
    highlights: Option<f32>,
    shadows: Option<f32>,
}

impl<'de> Deserialize<'de> for Recipe {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        RecipeWire::deserialize(d).map(Recipe::from)
    }
}

impl From<RecipeWire> for Recipe {
    fn from(w: RecipeWire) -> Self {
        let v1 = [w.exposure, w.contrast, w.saturation, w.temperature, w.tint, w.highlights, w.shadows];
        let develop = match w.develop {
            Some(d) => Some(d),
            None if v1.iter().any(|v| v.is_some_and(|v| v != 0.0)) => {
                let g = |v: Option<f32>| v.unwrap_or(0.0);
                Some(Develop {
                    basic: Basic {
                        exposure: g(w.exposure),
                        contrast: g(w.contrast),
                        saturation: g(w.saturation),
                        temperature: g(w.temperature),
                        tint: g(w.tint),
                        highlights: g(w.highlights),
                        shadows: g(w.shadows),
                        ..Basic::default()
                    },
                    ..Develop::default()
                })
            }
            None => None,
        };
        let version = if develop.as_ref().is_some_and(|d| !d.masks.is_empty()) { 3 } else { 2 };
        Recipe { version, crop: w.crop, rotate: w.rotate, straighten: w.straighten, flip_h: w.flip_h, flip_v: w.flip_v, develop, trim: w.trim }
    }
}

impl Default for Recipe {
    fn default() -> Self {
        Self::identity()
    }
}

/// Clamp a finite value; anything else becomes `fallback`.
fn cl(v: f32, lo: f32, hi: f32, fallback: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        fallback
    }
}

fn curve_points(points: Vec<[f32; 2]>) -> Vec<[f32; 2]> {
    let mut p: Vec<[f32; 2]> = points.into_iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    p.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-4);
    p.truncate(16);
    // The straight line is no curve at all.
    let straight = p.iter().all(|q| (q[0] - q[1]).abs() < 1e-4);
    if p.len() < 2 || straight {
        Vec::new()
    } else {
        p
    }
}

// One clamp per panel, shared by the photo's own values and a mask's.

fn norm_curve(c: Curve) -> Curve {
    Curve { master: curve_points(c.master), r: curve_points(c.r), g: curve_points(c.g), b: curve_points(c.b) }
}

fn norm_hsl(h: [Hsl; 8]) -> [Hsl; 8] {
    let u = |v: f32| cl(v, -1.0, 1.0, 0.0);
    h.map(|x| Hsl { h: u(x.h), s: u(x.s), l: u(x.l) })
}

fn norm_grading(g: Grading) -> Grading {
    let wheel = |w: Wheel| Wheel { hue: if w.hue.is_finite() { w.hue.rem_euclid(360.0) } else { 0.0 }, sat: cl(w.sat, 0.0, 1.0, 0.0), lum: cl(w.lum, -1.0, 1.0, 0.0) };
    Grading { shadows: wheel(g.shadows), midtones: wheel(g.midtones), highlights: wheel(g.highlights), blending: cl(g.blending, 0.0, 1.0, 0.5), balance: cl(g.balance, -1.0, 1.0, 0.0) }
}

fn norm_sharpen(s: Sharpen) -> Sharpen {
    Sharpen { amount: cl(s.amount, 0.0, 1.5, 0.0), radius: cl(s.radius, 0.5, 3.0, 1.0), detail: cl(s.detail, 0.0, 1.0, 0.25), masking: cl(s.masking, 0.0, 1.0, 0.0) }
}

fn norm_grain(g: Grain) -> Grain {
    Grain { amount: cl(g.amount, 0.0, 1.0, 0.0), size: cl(g.size, 0.0, 1.0, 0.25) }
}

fn norm_motion_blur(m: MotionBlur) -> MotionBlur {
    MotionBlur { amount: cl(m.amount, 0.0, 1.0, 0.0), angle: cl(m.angle, -90.0, 90.0, 0.0) }
}

fn norm_glow(g: Glow) -> Glow {
    Glow { amount: cl(g.amount, 0.0, 1.0, 0.0), size: cl(g.size, 0.0, 1.0, 0.4) }
}

fn norm_halation(h: Halation) -> Halation {
    Halation { amount: cl(h.amount, 0.0, 1.0, 0.0), size: cl(h.size, 0.0, 1.0, 0.3) }
}

impl Develop {
    pub fn normalized(self) -> Self {
        let b = self.basic;
        let u = |v: f32| cl(v, -1.0, 1.0, 0.0);
        let basic = Basic {
            temperature: cl(b.temperature, -100.0, 100.0, 0.0),
            tint: cl(b.tint, -100.0, 100.0, 0.0),
            exposure: cl(b.exposure, -5.0, 5.0, 0.0),
            contrast: u(b.contrast),
            highlights: u(b.highlights),
            shadows: u(b.shadows),
            whites: u(b.whites),
            blacks: u(b.blacks),
            texture: u(b.texture),
            clarity: u(b.clarity),
            dehaze: u(b.dehaze),
            vibrance: u(b.vibrance),
            saturation: u(b.saturation),
        };
        let detail = Detail {
            sharpen: norm_sharpen(self.detail.sharpen),
            denoise: self.detail.denoise.map(|d| Denoise { strength: cl(d.strength, 0.0, 1.0, 0.5), model: d.model }),
        };
        let e = self.effects;
        let effects = Effects {
            vignette: Vignette { amount: u(e.vignette.amount), midpoint: cl(e.vignette.midpoint, 0.0, 1.0, 0.5), feather: cl(e.vignette.feather, 0.0, 1.0, 0.5) },
            grain: norm_grain(e.grain),
            motion_blur: norm_motion_blur(e.motion_blur),
            glow: norm_glow(e.glow),
            halation: norm_halation(e.halation),
        };
        let masks = self.masks.into_iter().take(64).map(Mask::normalized).collect();
        Self { basic, curve: norm_curve(self.curve), hsl: norm_hsl(self.hsl), grading: norm_grading(self.grading), detail, effects, auto: self.auto, masks }
    }

    /// Changes no pixel (whether or not it came from *Auto*).
    pub fn is_identity(&self) -> bool {
        Self { auto: false, ..self.clone().normalized() } == Self::default()
    }
}

impl Recipe {
    pub fn identity() -> Self {
        Self { version: 2, crop: None, rotate: 0, straighten: 0.0, flip_h: false, flip_v: false, develop: None, trim: None }
    }

    /// Every field inside its range. Called on every recipe that crosses `invoke`.
    pub fn normalized(self) -> Self {
        let crop = self.crop.and_then(|k| {
            let x = cl(k.x, 0.0, 1.0, 0.0);
            let y = cl(k.y, 0.0, 1.0, 0.0);
            let w = cl(k.w, 0.0, 1.0 - x, 0.0);
            let h = cl(k.h, 0.0, 1.0 - y, 0.0);
            // A crop of (almost) nothing is a mistake; a crop of everything is no crop.
            if w < 0.01 || h < 0.01 || (x == 0.0 && y == 0.0 && w >= 0.9999 && h >= 0.9999) {
                None
            } else {
                Some(Crop { x, y, w, h })
            }
        });
        let develop = self.develop.map(Develop::normalized).filter(|d| !d.is_identity());
        let trim = self.trim.map(Trim::normalized).filter(|t| !t.segments.is_empty());
        // v3 only once there is a mask, so every other recipe writes back as it always has.
        let version = if develop.as_ref().is_some_and(|d| !d.masks.is_empty()) { 3 } else { 2 };
        Self {
            version,
            crop,
            rotate: (self.rotate / 90 % 4) * 90,
            straighten: cl(self.straighten, -45.0, 45.0, 0.0),
            flip_h: self.flip_h,
            flip_v: self.flip_v,
            develop,
            trim,
        }
    }

    pub fn is_identity(&self) -> bool {
        self.clone().normalized() == Self::identity()
    }

    /// Whether rendering the recipe changes the picture — false for no edit, and for a trim
    /// alone, which only changes which part of a video plays.
    pub fn changes_pixels(&self) -> bool {
        self.changes_pixels_with(&|_| true)
    }

    /// [`Self::changes_pixels`] knowing which rasters the photo has, so a mask pasted from
    /// another photo doesn't count as an edit here.
    pub fn changes_pixels_with(&self, has_raster: &dyn Fn(&str) -> bool) -> bool {
        let mut r = self.clone().normalized();
        if let Some(d) = &mut r.develop {
            d.masks.retain(|m| m.is_effective_with(has_raster));
        }
        r.develop = r.develop.filter(|d| !d.is_identity());
        Recipe { trim: None, version: 2, ..r } != Self::identity()
    }

    /// The hashes of the AI rasters the recipe's masks name, each once, in order.
    pub fn ai_rasters(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for m in self.develop.iter().flat_map(|d| &d.masks) {
            for s in &m.shapes {
                if let MaskShape::Ai { raster, .. } = s {
                    if !out.contains(&raster.as_str()) {
                        out.push(raster);
                    }
                }
            }
        }
        out
    }

    /// The Basic panel's values, or zeros when there is no develop.
    pub fn basic(&self) -> Basic {
        self.develop.as_ref().map(|d| d.basic).unwrap_or_default()
    }

    /// The frame the recipe produces from a display-oriented `w × h`. Order is the
    /// renderer's: flips, quarter turns, straighten (same frame, zoomed in), then crop.
    pub fn output_size(&self, w: u32, h: u32) -> (u32, u32) {
        let (w, h) = if self.rotate % 180 == 90 { (h, w) } else { (w, h) };
        match self.crop {
            Some(k) => (((w as f32 * k.w).round() as u32).max(1), ((h as f32 * k.h).round() as u32).max(1)),
            None => (w, h),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EditVersion {
    #[ts(type = "number")]
    pub asset_id: i64,
    #[ts(type = "number")]
    pub version: i64,
}

pub fn get_recipe(conn: &Connection, asset_id: i64) -> Result<Option<(Recipe, i64)>> {
    let row: Option<(String, i64)> = conn
        .query_row("SELECT recipe_json, version FROM edits WHERE asset_id = ?1", [asset_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((json, v)) => Some((serde_json::from_str::<Recipe>(&json)?.normalized(), v)),
        None => None,
    })
}

/// A video's kept segments, when it is trimmed to less than itself (for export).
pub fn get_trim(conn: &Connection, asset_id: i64) -> Result<Option<Vec<Segment>>> {
    Ok(get_recipe(conn, asset_id)?.and_then(|(r, _)| r.trim).map(|t| t.normalized().segments).filter(|s| !s.is_empty()))
}

/// Store `recipe` and return its version. Versions only grow — across reverts too — because
/// they end up in `immutable` image URLs. An identity recipe is a revert.
pub fn save_recipe(conn: &Connection, asset_id: i64, recipe: &Recipe, now_ms: i64) -> Result<i64> {
    let recipe = recipe.clone().normalized();
    if recipe.is_identity() {
        delete_recipe(conn, asset_id)?;
        return Ok(0);
    }
    let (w, h, orientation): (Option<i64>, Option<i64>, i64) =
        conn.query_row("SELECT width, height, orientation FROM assets WHERE id = ?1", [asset_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let (w, h) = (w.unwrap_or(4) as u32, h.unwrap_or(3) as u32);
    let (w, h) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
    let (ow, oh) = recipe.output_size(w, h);
    let version = next_version(conn, now_ms)?;
    conn.execute(
        "INSERT INTO edits (asset_id, recipe_json, version, updated_at, aspect) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(asset_id) DO UPDATE SET recipe_json = ?2, version = ?3, updated_at = ?4, aspect = ?5",
        params![asset_id, serde_json::to_string(&recipe)?, version, now_ms / 1000, ow as f64 / oh as f64],
    )?;
    Ok(version)
}

/// Revert: remove the recipe, returning it so the caller can offer *Undo*.
pub fn delete_recipe(conn: &Connection, asset_id: i64) -> Result<Option<Recipe>> {
    let old = get_recipe(conn, asset_id)?.map(|(r, _)| r);
    conn.execute("DELETE FROM edits WHERE asset_id = ?1", [asset_id])?;
    Ok(old)
}

/// A version no edit or draft ever had, recorded as the new high-water mark: monotonic per
/// library, surviving rows being deleted, because versions end up in `immutable` URLs and in
/// the cache keys of edited renders.
fn next_version(conn: &Connection, now_ms: i64) -> Result<i64> {
    let floor: i64 = conn.query_row(
        "SELECT MAX(COALESCE((SELECT MAX(version) FROM edits), 0), COALESCE((SELECT MAX(version) FROM develop_drafts), 0),
                    COALESCE((SELECT value FROM edit_meta WHERE key = 'max_version'), 0))",
        [],
        |r| r.get(0),
    )?;
    let version = now_ms.max(floor + 1);
    conn.execute(
        "INSERT INTO edit_meta (key, value) VALUES ('max_version', ?1) ON CONFLICT(key) DO UPDATE SET value = MAX(value, excluded.value)",
        [version],
    )?;
    Ok(version)
}

// Develop's drafts (Round 6): what Develop shows and an export renders. The library, the
// viewer and every thumbnail use `edits` alone.

/// The draft Develop saved for a photo, and its version.
pub fn get_draft(conn: &Connection, asset_id: i64) -> Result<Option<(Recipe, i64)>> {
    let row: Option<(String, i64)> = conn
        .query_row("SELECT recipe_json, version FROM develop_drafts WHERE asset_id = ?1", [asset_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    Ok(match row {
        Some((json, v)) => Some((serde_json::from_str::<Recipe>(&json)?.normalized(), v)),
        None => None,
    })
}

/// What Develop opens a photo with: its draft, or else the library's edit (a crop or turn made
/// in the viewer carries into Develop).
pub fn develop_recipe(conn: &Connection, asset_id: i64) -> Result<Option<Recipe>> {
    Ok(match get_draft(conn, asset_id)? {
        Some((r, _)) => Some(r),
        None => get_recipe(conn, asset_id)?.map(|(r, _)| r),
    })
}

/// Keep Develop's settings for a photo, leaving the library's look alone. An identity draft is
/// kept too: it is what Develop was left at, even over a library edit.
pub fn save_draft(conn: &Connection, asset_id: i64, recipe: &Recipe, now_ms: i64) -> Result<i64> {
    let recipe = recipe.clone().normalized();
    let version = next_version(conn, now_ms)?;
    conn.execute(
        "INSERT INTO develop_drafts (asset_id, recipe_json, version, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(asset_id) DO UPDATE SET recipe_json = ?2, version = ?3, updated_at = ?4",
        params![asset_id, serde_json::to_string(&recipe)?, version, now_ms / 1000],
    )?;
    Ok(version)
}

pub fn delete_draft(conn: &Connection, asset_id: i64) -> Result<Option<Recipe>> {
    let old = get_draft(conn, asset_id)?.map(|(r, _)| r);
    conn.execute("DELETE FROM develop_drafts WHERE asset_id = ?1", [asset_id])?;
    Ok(old)
}

/// What an export renders: Develop's draft when there is one, else the library's edit.
pub fn export_recipe(conn: &Connection, asset_id: i64) -> Result<Option<(Recipe, i64)>> {
    Ok(match get_draft(conn, asset_id)? {
        Some(d) => Some(d),
        None => get_recipe(conn, asset_id)?,
    })
}

pub fn list_versions(conn: &Connection) -> Result<Vec<EditVersion>> {
    let mut stmt = conn.prepare("SELECT asset_id, version FROM edits ORDER BY asset_id")?;
    let rows = stmt.query_map([], |r| Ok(EditVersion { asset_id: r.get(0)?, version: r.get(1)? }))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Develop's panels reset to these, so the frontend imports them rather than keep a copy.
    #[test]
    fn writes_the_develop_defaults_for_the_frontend() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/develop-defaults.json");
        std::fs::write(path, serde_json::to_string_pretty(&Develop::default()).unwrap() + "\n").unwrap();
    }
    use crate::Db;

    fn insert_asset(c: &Connection, w: i64, h: i64, orientation: i64) -> i64 {
        c.execute(
            "INSERT INTO assets (content_hash, relative_path, filename, extension, media_type, byte_size, width, height, orientation, imported_at)
             VALUES (hex(randomblob(16)), hex(randomblob(8)), 'a.jpg', 'jpg', 0, 1, ?1, ?2, ?3, 1)",
            rusqlite::params![w, h, orientation],
        )
        .unwrap();
        c.last_insert_rowid()
    }

    fn exposed(ev: f32) -> Recipe {
        Recipe { develop: Some(Develop { basic: Basic { exposure: ev, ..Basic::default() }, ..Develop::default() }), ..Recipe::identity() }
    }

    #[test]
    fn a_partial_develop_parses_and_an_empty_one_is_no_edit() {
        let r: Recipe = serde_json::from_str(r#"{"version":2,"develop":{"basic":{"clarity":0.4},"hsl":[{"h":0.1},{},{},{},{},{},{},{}]}}"#).unwrap();
        let d = r.develop.clone().unwrap();
        assert_eq!(d.basic.clarity, 0.4);
        assert_eq!(d.hsl[0].h, 0.1);
        assert_eq!(d.grading.blending, 0.5, "missing fields take their defaults");
        let blank = Recipe { develop: Some(Develop { auto: true, ..Develop::default() }), flip_v: false, ..Recipe::identity() };
        assert!(blank.is_identity(), "an Auto that changed nothing changes nothing");
        assert!(!Recipe { flip_v: true, ..Recipe::identity() }.is_identity());
        let straight = Recipe { develop: Some(Develop { curve: Curve { master: vec![[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]], ..Curve::default() }, ..Develop::default() }), ..Recipe::identity() };
        assert!(straight.is_identity(), "a straight curve is no curve");
    }

    #[test]
    fn trims_are_sorted_merged_and_emptied_away() {
        let seg = |a, b| Segment { start_ms: a, end_ms: b };
        let t = Trim { segments: vec![seg(5000, 8000), seg(0, 2000), seg(1500, 3000), seg(9000, 9000)] }.normalized();
        assert_eq!(t.segments, vec![seg(0, 3000), seg(5000, 8000)]);
        assert_eq!(t.kept_ms(), 6000);
        assert!(Recipe { trim: Some(Trim::default()), ..Recipe::identity() }.is_identity());
        assert!(!Recipe { trim: Some(t.clone()), ..Recipe::identity() }.changes_pixels(), "a trim alone changes no pixel");
        assert!(Recipe { trim: Some(t.clone()), flip_v: true, ..Recipe::identity() }.changes_pixels());
        let r = Recipe { trim: Some(t.clone()), ..Recipe::identity() };
        let back: Recipe = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.trim, Some(t));
    }

    #[test]
    fn identity_is_identity_and_serialises_like_the_spec() {
        let r = Recipe::identity();
        assert!(r.is_identity());
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["version"], 2);
        assert_eq!(json["rotate"], 0);
        assert_eq!(json["flip_h"], false);
        // The spec's example parses.
        let spec: Recipe = serde_json::from_str(r#"{"version":1,"crop":{"x":0.1,"y":0.0,"w":0.8,"h":1.0},"rotate":90,"straighten":-1.4,"flip_h":false,"exposure":0.3,"contrast":0.1,"saturation":-0.05,"temperature":120,"tint":-4,"highlights":-0.2,"shadows":0.15}"#).unwrap();
        assert_eq!(spec.rotate, 90);
        assert!(!spec.is_identity());
        assert_eq!(spec.version, 2, "a v1 recipe reads as v2");
        let b = spec.basic();
        assert_eq!((b.exposure, b.temperature, b.shadows), (0.3, 120.0, 0.15));
        assert_eq!(spec.clone().normalized().basic().temperature, 100.0, "and is clamped");
        // Written back, it is v2 and reads the same.
        let again: Recipe = serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(again, spec);
        assert!(serde_json::to_value(&spec).unwrap().get("exposure").is_none());
    }

    #[test]
    fn normalised_clamps_every_field() {
        let r = Recipe {
            rotate: 450,
            straighten: 90.0,
            develop: Some(Develop { basic: Basic { exposure: 9.0, contrast: -4.0, temperature: 500.0, ..Basic::default() }, ..Develop::default() }),
            crop: Some(Crop { x: -0.2, y: 0.5, w: 2.0, h: 0.8 }),
            ..Recipe::identity()
        }
        .normalized();
        assert_eq!(r.rotate, 90);
        assert_eq!(r.straighten, 45.0);
        let b = r.basic();
        assert_eq!((b.exposure, b.contrast, b.temperature), (5.0, -1.0, 100.0));
        let c = r.crop.unwrap();
        assert!(c.x >= 0.0 && c.x + c.w <= 1.0 && c.y + c.h <= 1.0, "{c:?}");
    }

    #[test]
    fn output_size_follows_rotate_then_crop() {
        let mut r = Recipe::identity();
        assert_eq!(r.output_size(4000, 3000), (4000, 3000));
        r.rotate = 90;
        assert_eq!(r.output_size(4000, 3000), (3000, 4000));
        r.crop = Some(Crop { x: 0.0, y: 0.0, w: 0.5, h: 0.25 });
        assert_eq!(r.output_size(4000, 3000), (1500, 1000));
        r.straighten = 5.0; // straighten keeps the frame size; it zooms in instead
        assert_eq!(r.output_size(4000, 3000), (1500, 1000));
    }

    /// Round 6: Develop's edits stay in Develop. Its draft starts from the library's edit (a
    /// viewer crop carries in), saving it leaves the library's recipe alone, and an export
    /// renders the draft.
    #[test]
    fn a_develop_draft_never_changes_the_library_until_export() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = insert_asset(c, 4000, 3000, 1);
            let turned = Recipe { rotate: 90, ..Recipe::identity() };
            let v = save_recipe(c, id, &turned, 1_000)?;
            assert_eq!(develop_recipe(c, id)?, Some(turned.clone().normalized()), "Develop starts from the library's edit");
            let developed = Recipe { develop: exposed(0.5).develop, ..turned.clone() };
            let dv = save_draft(c, id, &developed, 1_000)?;
            assert!(dv > v, "a draft's version is never one the library used");
            assert_eq!(get_recipe(c, id)?, Some((turned.clone().normalized(), v)), "the library is untouched");
            assert_eq!(develop_recipe(c, id)?.map(|r| r.basic().exposure), Some(0.5));
            assert_eq!(export_recipe(c, id)?.map(|(r, ver)| (r.basic().exposure, ver)), Some((0.5, dv)));
            assert_eq!(delete_draft(c, id)?.map(|r| r.basic().exposure), Some(0.5));
            assert_eq!(export_recipe(c, id)?, Some((turned.normalized(), v)), "without a draft, the library's edit");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn versions_only_ever_grow_even_across_a_revert() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = insert_asset(c, 4000, 3000, 1);
            let r = exposed(0.5);
            let v1 = save_recipe(c, id, &r, 1_000)?;
            let v2 = save_recipe(c, id, &r, 1_000)?; // same millisecond
            assert!(v2 > v1);
            assert_eq!(delete_recipe(c, id)?.map(|r| r.basic().exposure), Some(0.5));
            assert_eq!(get_recipe(c, id)?, None);
            let v3 = save_recipe(c, id, &r, 500)?; // clock went backwards
            assert!(v3 > v2, "a revert must not let a version be reused: {v3} <= {v2}");
            assert_eq!(list_versions(c)?, vec![EditVersion { asset_id: id, version: v3 }]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn saving_records_the_edited_aspect_in_display_orientation() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            // Stored landscape, EXIF says turn it: displayed portrait 3000x4000.
            let id = insert_asset(c, 4000, 3000, 6);
            save_recipe(c, id, &Recipe { crop: Some(Crop { x: 0.0, y: 0.0, w: 1.0, h: 0.5 }), ..Recipe::identity() }, 1)?;
            let aspect: f64 = c.query_row("SELECT aspect FROM edits WHERE asset_id = ?1", [id], |r| r.get(0))?;
            assert!((aspect - 3000.0 / 2000.0).abs() < 1e-6, "{aspect}");
            Ok(())
        })
        .unwrap();
    }

    /// Written by 8c's code before masks existed.
    const RECIPE_V2: &str = include_str!("testdata/recipe-v2.json");

    #[test]
    fn a_v2_recipe_parses_and_writes_back_byte_for_byte() {
        let r: Recipe = serde_json::from_str(RECIPE_V2).unwrap();
        assert_eq!(r.version, 2);
        assert!(r.develop.as_ref().unwrap().masks.is_empty());
        assert_eq!(serde_json::to_string(&r.clone().normalized()).unwrap(), RECIPE_V2.trim_end());
    }

    /// A masked recipe written by Round 18's code, before masks took every local tool.
    const RECIPE_V3: &str = include_str!("testdata/recipe-v3.json");

    #[test]
    fn a_mask_written_before_round_19_parses_and_writes_back_byte_for_byte() {
        let r: Recipe = serde_json::from_str(RECIPE_V3).unwrap();
        let a = &r.develop.as_ref().unwrap().masks[0].adjust;
        assert_eq!((a.exposure, a.clarity, a.sharpness), (1.0, -0.25, 0.5));
        assert_eq!(*a, LocalAdjust { exposure: 1.0, clarity: -0.25, sharpness: 0.5, ..LocalAdjust::default() }, "the new tools default");
        assert_eq!(serde_json::to_string(&r.normalized()).unwrap(), RECIPE_V3.trim_end());
    }

    fn every_local_tool() -> LocalAdjust {
        let mut hsl = [Hsl::default(); 8];
        hsl[5] = Hsl { h: 0.25, s: -0.5, l: 0.125 };
        LocalAdjust {
            exposure: 0.5,
            vibrance: 0.25,
            curve: Curve { master: vec![[0.0, 0.0625], [0.5, 0.625], [1.0, 1.0]], b: vec![[0.0, 0.0], [0.5, 0.375], [1.0, 1.0]], ..Curve::default() },
            hsl,
            grading: Grading { shadows: Wheel { hue: 210.0, sat: 0.25, lum: 0.0 }, balance: 0.5, ..Grading::default() },
            sharpen: Some(LocalSharpen { radius: 2.0, detail: 0.5, masking: 0.75 }),
            grain: Grain { amount: 0.5, size: 0.75 },
            glow: Glow { amount: 0.25, size: 0.5 },
            halation: Halation { amount: 0.5, size: 0.25 },
            motion_blur: MotionBlur { amount: 0.5, angle: 45.0 },
            ..LocalAdjust::default()
        }
    }

    #[test]
    fn each_local_tool_round_trips_and_only_it_is_written() {
        let all = every_local_tool();
        let r = masked(all.clone()).normalized();
        let back: Recipe = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back.develop.unwrap().masks[0].adjust, all, "every field survives");
        let one_each: [(&str, LocalAdjust); 9] = [
            ("vibrance", LocalAdjust { vibrance: all.vibrance, ..LocalAdjust::default() }),
            ("curve", LocalAdjust { curve: all.curve.clone(), ..LocalAdjust::default() }),
            ("hsl", LocalAdjust { hsl: all.hsl, ..LocalAdjust::default() }),
            ("grading", LocalAdjust { grading: all.grading, ..LocalAdjust::default() }),
            ("sharpen", LocalAdjust { sharpen: all.sharpen, ..LocalAdjust::default() }),
            ("grain", LocalAdjust { grain: all.grain, ..LocalAdjust::default() }),
            ("glow", LocalAdjust { glow: all.glow, ..LocalAdjust::default() }),
            ("halation", LocalAdjust { halation: all.halation, ..LocalAdjust::default() }),
            ("motion_blur", LocalAdjust { motion_blur: all.motion_blur, ..LocalAdjust::default() }),
        ];
        let new_keys: Vec<&str> = one_each.iter().map(|(k, _)| *k).collect();
        for (key, adjust) in one_each {
            let json = serde_json::to_value(&adjust).unwrap();
            let obj = json.as_object().unwrap();
            assert!(obj.contains_key(key), "{key}: {json}");
            assert!(new_keys.iter().filter(|k| **k != key).all(|k| !obj.contains_key(*k)), "{key} alone: {json}");
            assert_eq!(serde_json::from_value::<LocalAdjust>(json).unwrap(), adjust, "{key}");
        }
    }

    #[test]
    fn the_local_tools_are_clamped_like_the_photos() {
        let mut hsl = [Hsl::default(); 8];
        hsl[0] = Hsl { h: 3.0, s: f32::NAN, l: -2.0 };
        let wild = LocalAdjust {
            vibrance: 4.0,
            curve: Curve { master: vec![[0.5, 0.5], [0.0, 0.0]], r: vec![[-1.0, 0.25], [2.0, 0.5]], ..Curve::default() },
            hsl,
            grading: Grading { highlights: Wheel { hue: 400.0, sat: 2.0, lum: -3.0 }, blending: 7.0, balance: f32::INFINITY, ..Grading::default() },
            sharpen: Some(LocalSharpen { radius: 9.0, detail: -1.0, masking: f32::NAN }),
            grain: Grain { amount: 2.0, size: -1.0 },
            glow: Glow { amount: -1.0, size: f32::NAN },
            halation: Halation { amount: 5.0, size: 5.0 },
            motion_blur: MotionBlur { amount: 2.0, angle: 400.0 },
            ..LocalAdjust::default()
        };
        let n = wild.normalized();
        assert_eq!(n.vibrance, 1.0);
        assert!(n.curve.master.is_empty(), "the straight line is no curve");
        assert_eq!(n.curve.r, vec![[0.0, 0.25], [1.0, 0.5]]);
        assert_eq!((n.hsl[0].h, n.hsl[0].s, n.hsl[0].l), (1.0, 0.0, -1.0));
        let g = n.grading;
        assert!((g.highlights.hue - 40.0).abs() < 1e-4);
        assert_eq!((g.highlights.sat, g.highlights.lum, g.blending, g.balance), (1.0, -1.0, 1.0, 0.0));
        assert_eq!(n.sharpen, Some(LocalSharpen { radius: 3.0, detail: 0.0, masking: 0.0 }));
        assert_eq!((n.grain.amount, n.grain.size), (1.0, 0.0));
        assert_eq!((n.glow.amount, n.glow.size), (0.0, 0.4));
        assert_eq!((n.halation.amount, n.halation.size), (1.0, 1.0));
        assert_eq!((n.motion_blur.amount, n.motion_blur.angle), (1.0, 90.0));
    }

    #[test]
    fn a_mask_with_only_a_local_tool_is_effective() {
        let curve = LocalAdjust { curve: every_local_tool().curve, ..LocalAdjust::default() };
        assert!(!curve.is_zero() && masked(curve).changes_pixels());
        let sharpen_only = LocalAdjust { sharpen: Some(LocalSharpen::default()), ..LocalAdjust::default() };
        assert!(sharpen_only.is_zero(), "a radius with no amount sharpens nothing");
        assert_eq!(LocalAdjust { vibrance: 0.5, ..LocalAdjust::default() }.basic().vibrance, 0.5);
    }

    pub(crate) fn brush(points: Vec<[u16; 2]>) -> MaskShape {
        MaskShape::Brush { add: true, radius: 0.05, feather: 0.5, flow: 1.0, points }
    }

    pub(crate) fn masked(adjust: LocalAdjust) -> Recipe {
        let mask = Mask {
            id: 1,
            name: "Sky".into(),
            shapes: vec![
                brush(vec![[0, 0], [4096, 2048], [1234, 4095]]),
                MaskShape::Linear { add: false, from: [0.5, 0.0], to: [0.5, 0.4] },
                MaskShape::Radial { add: true, center: [0.3, 0.6], radius: [0.2, 0.1], angle: 30.0, feather: 0.4, inside: true },
            ],
            invert: false,
            opacity: 0.8,
            adjust,
            visible: true,
        };
        Recipe { develop: Some(Develop { masks: vec![mask], ..Develop::default() }), ..Recipe::identity() }
    }

    #[test]
    fn a_v3_recipe_round_trips_with_its_quantised_points() {
        let r = masked(LocalAdjust { exposure: 1.0, ..LocalAdjust::default() }).normalized();
        assert_eq!(r.version, 3, "a recipe with a mask is v3");
        let json = serde_json::to_string(&r).unwrap();
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        let MaskShape::Brush { points, .. } = &back.develop.as_ref().unwrap().masks[0].shapes[0] else { panic!("a brush") };
        assert_eq!(points, &vec![[0, 0], [4096, 2048], [1234, 4095]]);
        let removed = Recipe { develop: Some(Develop { masks: vec![], ..r.develop.clone().unwrap() }), ..r.clone() };
        assert_eq!(removed.normalized().version, 2, "no mask, back to v2");
    }

    fn ai_shape(raster: &str) -> MaskShape {
        MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: raster.into() }
    }

    fn ai_masked(shapes: Vec<MaskShape>, adjust: LocalAdjust) -> Recipe {
        let mask = Mask { id: 1, name: "Sky".into(), shapes, invert: false, opacity: 1.0, adjust, visible: true };
        Recipe { develop: Some(Develop { masks: vec![mask], ..Develop::default() }), ..Recipe::identity() }
    }

    #[test]
    fn an_ai_shape_round_trips_in_a_v3_recipe() {
        let r = ai_masked(vec![ai_shape("00ff"), MaskShape::Ai { add: false, target: AiMaskKind::Person, instance: 2, raster: "abcd".into() }], LocalAdjust { exposure: 1.0, ..LocalAdjust::default() }).normalized();
        assert_eq!(r.version, 3);
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#""shapes":[{"kind":"ai","add":true,"target":"sky","instance":0,"raster":"00ff"},{"kind":"ai","add":false,"target":"person","instance":2,"raster":"abcd"}]"#), "{json}");
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.ai_rasters(), ["00ff", "abcd"]);
    }

    #[test]
    fn a_recipe_without_an_ai_shape_writes_what_it_always_wrote() {
        let r = masked(LocalAdjust { exposure: 1.0, ..LocalAdjust::default() }).normalized();
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains(r#"{"kind":"brush","add":true,"radius":0.05,"feather":0.5,"flow":1.0,"points":[[0,0],[4096,2048],[1234,4095]]}"#), "{json}");
        assert!(json.contains(r#"{"kind":"linear","add":false,"from":[0.5,0.0],"to":[0.5,0.4]}"#), "{json}");
        assert!(json.contains(r#"{"kind":"radial","add":true,"center":[0.3,0.6],"radius":[0.2,0.1],"angle":30.0,"feather":0.4,"inside":true}"#), "{json}");
        assert!(r.ai_rasters().is_empty());
    }

    #[test]
    fn a_shape_naming_no_real_raster_is_dropped() {
        for bad in ["", "not hex", &"a".repeat(65)] {
            let r = ai_masked(vec![ai_shape(bad)], LocalAdjust::default()).normalized();
            assert!(r.develop.unwrap().masks[0].shapes.is_empty(), "{bad:?}");
        }
    }

    #[test]
    fn changes_pixels_knows_which_rasters_the_photo_has() {
        let mut r = ai_masked(vec![ai_shape("00ff")], LocalAdjust { exposure: 1.0, ..LocalAdjust::default() });
        assert!(r.changes_pixels(), "it may well have the raster");
        assert!(r.changes_pixels_with(&|h| h == "00ff"));
        assert!(!r.changes_pixels_with(&|_| false), "a raster it doesn't have weighs nothing");
        r.develop.as_mut().unwrap().masks[0].invert = true;
        assert!(r.changes_pixels_with(&|_| false), "inverted, nothing is everything, as an empty mask");
        let idle = ai_masked(vec![ai_shape("00ff")], LocalAdjust::default());
        assert!(!idle.changes_pixels(), "zero adjustments change nothing");
    }

    #[test]
    fn a_mask_with_nothing_to_do_changes_no_pixel_but_is_kept() {
        let empty = masked(LocalAdjust::default());
        assert!(!empty.changes_pixels(), "zero adjustments change nothing");
        assert!(!empty.is_identity(), "but the mask is the user's work and is saved");
        let mut hidden = masked(LocalAdjust { exposure: 1.0, ..LocalAdjust::default() });
        assert!(hidden.changes_pixels());
        hidden.develop.as_mut().unwrap().masks[0].visible = false;
        assert!(!hidden.changes_pixels(), "a hidden mask changes nothing");
    }

    #[test]
    fn masks_are_clamped_like_every_other_field() {
        let mut r = masked(LocalAdjust { exposure: 9.0, contrast: -3.0, temperature: 400.0, ..LocalAdjust::default() });
        let m = &mut r.develop.as_mut().unwrap().masks[0];
        m.opacity = 2.0;
        m.shapes.push(MaskShape::Brush { add: true, radius: f32::NAN, feather: 3.0, flow: -1.0, points: vec![[9000, 5]] });
        m.shapes.push(MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.0, 0.0], angle: 400.0, feather: 0.5, inside: true });
        let r = r.normalized();
        let m = &r.develop.as_ref().unwrap().masks[0];
        assert_eq!((m.adjust.exposure, m.adjust.contrast, m.adjust.temperature, m.opacity), (5.0, -1.0, 100.0, 1.0));
        let MaskShape::Brush { radius, feather, flow, points, .. } = &m.shapes[3] else { panic!() };
        assert!(radius.is_finite() && *radius > 0.0);
        assert_eq!((*feather, *flow, points[0]), (1.0, 0.0, [4096, 5]));
        let MaskShape::Radial { angle, radius, .. } = &m.shapes[4] else { panic!() };
        assert!((*angle - 40.0).abs() < 1e-4 && radius[0] > 0.0);
    }

    #[test]
    fn a_two_thousand_dab_mask_stays_small() {
        let points = (0..2000u32).map(|i| [((i * 37) % 4097) as u16, ((i * 91) % 4097) as u16]).collect();
        let mut r = masked(LocalAdjust { exposure: 1.0, ..LocalAdjust::default() });
        r.develop.as_mut().unwrap().masks[0].shapes = vec![brush(points)];
        let n = serde_json::to_string(&r.normalized()).unwrap().len();
        assert!(n < 64 * 1024, "{n} bytes");
    }

    #[test]
    fn an_identity_recipe_is_not_stored() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn(|c| {
            let id = insert_asset(c, 10, 10, 1);
            save_recipe(c, id, &exposed(1.0), 1)?;
            save_recipe(c, id, &Recipe::identity(), 2)?;
            assert_eq!(get_recipe(c, id)?, None, "saving no changes is a revert");
            Ok(())
        })
        .unwrap();
    }
}
