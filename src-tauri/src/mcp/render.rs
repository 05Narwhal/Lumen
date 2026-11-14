//! What AI assistants see (Round 16): renders through Develop's pipeline with a one-slot base
//! cache of their own, so they never evict the UI's bases or overtake its renders
//! (`DevelopState`'s latest-wins counters are left alone).

use std::sync::{Arc, Mutex};

use image::RgbImage;
use photos_core::ai_masks::{self, AiMaskInfo, AiMaskKind, AiRasters};
use photos_core::edits::{Crop, Mask, MaskShape, Recipe};
use photos_core::Library;
use photos_media::develop::analyze::{analyze, Analysis, FaceBox};
use photos_media::develop::masks::{self, FrameMap};

use super::fit::{photo_colour, region_stats, sky_share, PhotoColour, RegionStats};
use photos_media::develop::{self, geometry, Ctx, Linear, Region};
use photos_media::{encode_jpeg_bytes, Ffmpeg, MediaKind};
use photos_media::store::DerivativeStore;

use crate::develop_preview::{base_edge, finish_region, load, rasters_of, recipe_margin, region_box, source, Base, Source, PREVIEW_EDGE};
use crate::error::{AppError, AppResult};

pub const JPEG_QUALITY: u8 = 82;
/// `analyze` measures a render this large: plenty for percentiles, quick to make.
const ANALYZE_EDGE: u32 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    After,
    Before,
    /// Before on the left half, after on the right.
    Split,
}

pub struct Jpeg {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct Renders {
    slot: Mutex<Option<(i64, u32, Arc<Base>)>>,
}

fn failed(e: impl std::fmt::Display) -> AppError {
    crate::develop_preview::render_failed(e)
}

fn ctx_for(base: &Base, src: &Source) -> Ctx<'static> {
    Ctx::new(base.linear.long_edge() as f32 / base.full_long.max(1) as f32, develop::seed_for(&src.hash))
}

fn before(recipe: &Recipe) -> Recipe {
    Recipe { develop: None, ..recipe.clone() }
}

/// Before on the left of the divide, after on the right.
fn split(before: &RgbImage, after: &RgbImage) -> RgbImage {
    let mut out = after.clone();
    let mid = out.width() / 2;
    for (x, y, p) in out.enumerate_pixels_mut() {
        if x < mid {
            *p = *before.get_pixel(x.min(before.width() - 1), y.min(before.height() - 1));
        } else if x < mid + 2 {
            *p = image::Rgb([255, 255, 255]);
        }
    }
    out
}

/// Red at 50 % where the mask weighs fully, less where it weighs less.
fn paint_red(img: &mut RgbImage, weights: &[f32]) {
    for (p, &w) in img.pixels_mut().zip(weights) {
        let a = 0.5 * w.clamp(0.0, 1.0);
        for (c, target) in p.0.iter_mut().zip([255.0, 0.0, 0.0]) {
            *c = (*c as f32 * (1.0 - a) + target * a).round() as u8;
        }
    }
}

fn encode(img: RgbImage) -> AppResult<Jpeg> {
    let (width, height) = img.dimensions();
    let bytes = encode_jpeg_bytes(&image::DynamicImage::ImageRgb8(img), JPEG_QUALITY).map_err(failed)?;
    Ok(Jpeg { bytes, width, height })
}

impl Renders {
    /// The photo's base at the size Develop's preview would use, kept for the last photo asked.
    fn base(&self, lib: &Library, ffmpeg: Option<&Ffmpeg>, id: i64, recipe: &Recipe) -> AppResult<(Source, Arc<Base>)> {
        let src = source(lib, id)?;
        let edge = base_edge(if src.long > 0 { src.long } else { PREVIEW_EDGE }, recipe.crop);
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((sid, sedge, base)) = slot.as_ref() {
            if (*sid, *sedge) == (id, edge) {
                return Ok((src, base.clone()));
            }
        }
        // Release the old base before decoding the next.
        *slot = None;
        let (linear, full_long) = load(&src, ffmpeg, Some(edge))?;
        let base = Arc::new(Base::new(linear, full_long, src.kind == MediaKind::Raw));
        *slot = Some((id, edge, base.clone()));
        Ok((src, base))
    }

    /// The whole frame with `recipe`, its long edge at most `edge`.
    fn frame(&self, lib: &Library, id: i64, src: &Source, base: &Base, recipe: &Recipe, edge: u32) -> AppResult<Linear> {
        let linear = base.for_recipe(&DerivativeStore::new(&lib.root), &src.hash, recipe);
        let rasters = rasters_of(lib, id, recipe)?;
        let out = develop::process(&linear, recipe, &Ctx { ai_rasters: Some(&rasters), ..ctx_for(base, src) });
        Ok(if out.long_edge() > edge { out.resize_long_edge(edge) } else { out })
    }

    /// `overlay` names one of the recipe's masks to paint in red over the after image (over
    /// the before image when that is all there is); the whole frame only, not a region.
    #[allow(clippy::too_many_arguments)]
    pub fn view(&self, lib: &Library, ffmpeg: Option<&Ffmpeg>, id: i64, recipe: &Recipe, size: u32, compare: Compare, region: Option<Crop>, overlay: Option<u32>) -> AppResult<Jpeg> {
        let recipe = recipe.clone().normalized();
        if let Some(rect) = region {
            return encode(self.region(lib, ffmpeg, id, &recipe, rect, size, compare)?);
        }
        let (src, base) = self.base(lib, ffmpeg, id, &recipe)?;
        let render = |r: &Recipe| self.frame(lib, id, &src, &base, r, size).map(|l| l.to_rgb8());
        let mask = overlay.and_then(|m| recipe.develop.as_ref()?.masks.iter().find(|k| k.id == m).cloned());
        let paint = |mut img: RgbImage| -> AppResult<RgbImage> {
            if let Some(mask) = &mask {
                let [fw, _] = FrameMap::frame_size(base.linear.width, base.linear.height, &recipe);
                let map = FrameMap::new(base.linear.width, base.linear.height, &recipe).scaled(img.width() as f32 / fw.max(1.0));
                let rasters = rasters_of(lib, id, &recipe)?;
                let weights = masks::rasterize_with(mask, &map, img.width(), img.height(), Some(&rasters), None);
                paint_red(&mut img, &weights);
            }
            Ok(img)
        };
        encode(match compare {
            Compare::After => paint(render(&recipe)?)?,
            Compare::Before => paint(render(&before(&recipe))?)?,
            Compare::Split => split(&render(&before(&recipe))?, &paint(render(&recipe)?)?),
        })
    }

    /// A part of the edited frame (0–1) at 100 %, scaled down only to fit `size`. Read from
    /// the full-resolution file each time: no cache competes with Develop's 100 % slot.
    #[allow(clippy::too_many_arguments)]
    fn region(&self, lib: &Library, ffmpeg: Option<&Ffmpeg>, id: i64, recipe: &Recipe, rect: Crop, size: u32, compare: Compare) -> AppResult<RgbImage> {
        let ok = |v: f32| v.is_finite() && (0.0..=1.0).contains(&v);
        if !(ok(rect.x) && ok(rect.y) && ok(rect.w) && ok(rect.h)) || rect.w <= 0.0 || rect.h <= 0.0 {
            return Err(AppError { code: "invalid_region".into(), message: "region needs x, y, w, h in 0–1 with w, h above 0.".into(), detail: None });
        }
        let src = source(lib, id)?;
        let (full, _) = load(&src, ffmpeg, None)?;
        let frame = geometry::apply(&full, recipe);
        let map = FrameMap::new(full.width, full.height, recipe);
        let denoised = crate::denoise::denoise_of(recipe).and_then(|d| {
            let dn = crate::denoise::load_for(&DerivativeStore::new(&lib.root), &src.hash, recipe)?;
            Some((geometry::apply(&dn, recipe), d.strength))
        });
        drop(full);
        let (fw, fh) = (frame.width, frame.height);
        let ((px, py, pw, ph), inner) = region_box(rect, fw, fh, recipe_margin(recipe, &map, fw.max(fh)));
        let rasters = rasters_of(lib, id, recipe)?;
        let render = |r: &Recipe| -> RgbImage {
            let mut part = geometry::crop(&frame, px, py, pw, ph);
            let Some(dv) = r.develop.as_ref() else { return finish_region(&part, inner, 1.0) };
            if let Some((dn, strength)) = denoised.as_ref().filter(|(dn, _)| (dn.width, dn.height) == (fw, fh)) {
                part = develop::blend_denoised(&part, &geometry::crop(dn, px, py, pw, ph), *strength);
            }
            let ctx = Ctx {
                region: Some(Region { x: px, y: py, frame_width: fw, frame_height: fh }),
                airlight: Some(develop::frame_airlight(&frame, dv)),
                frame_map: Some(map),
                ai_rasters: Some(&rasters),
                ..Ctx::new(1.0, develop::seed_for(&src.hash))
            };
            develop::develop_stages(&mut part, dv, &ctx);
            finish_region(&part, inner, 1.0)
        };
        let img = match compare {
            Compare::After => render(recipe),
            Compare::Before => render(&before(recipe)),
            Compare::Split => split(&render(&before(recipe)), &render(recipe)),
        };
        let long = img.width().max(img.height());
        Ok(if long > size {
            let k = size as f32 / long as f32;
            let (w, h) = (((img.width() as f32 * k).round() as u32).max(1), ((img.height() as f32 * k).round() as u32).max(1));
            image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle)
        } else {
            img
        })
    }

    /// Measurements of the edited frame; `faces` are 0–1 of the unedited photo and are
    /// moved into the edited frame (those cropped away are left out). `ai` are Facet's current
    /// masks, measured where they land in the edited frame.
    #[allow(clippy::too_many_arguments)]
    pub fn analyze(&self, lib: &Library, ffmpeg: Option<&Ffmpeg>, id: i64, recipe: &Recipe, faces: &[FaceBox], ai: &[AiMaskInfo]) -> AppResult<(Analysis, u32, u32, Regions)> {
        let recipe = recipe.clone().normalized();
        let (src, base) = self.base(lib, ffmpeg, id, &recipe)?;
        let full = self.frame(lib, id, &src, &base, &recipe, u32::MAX)?;
        let (w, h) = (full.width, full.height);
        let map = FrameMap::new(base.linear.width, base.linear.height, &recipe);
        let moved: Vec<FaceBox> = faces.iter().filter_map(|f| face_in_frame(&map, f, w as f32, h as f32)).collect();
        let small = if full.long_edge() > ANALYZE_EDGE { full.resize_long_edge(ANALYZE_EDGE) } else { full };
        let regions = regions(lib, id, map.scaled(small.width as f32 / w.max(1) as f32), &small.to_rgb8(), ai)?;
        Ok((analyze(&small, &moved), w, h, regions))
    }
}

impl Renders {
    /// What `suggest_looks` scores: the colour summary of the photo's *before* render (its
    /// crop, no Develop edits), so asking again after an edit gives the same numbers. `faces`
    /// are 0–1 of the unedited photo; a face with skin hue 20–50° marks the photo as having skin.
    pub fn colour(&self, lib: &Library, ffmpeg: Option<&Ffmpeg>, id: i64, recipe: &Recipe, faces: &[FaceBox], ai: &[AiMaskInfo]) -> AppResult<PhotoColour> {
        let recipe = before(&recipe.clone().normalized());
        let (src, base) = self.base(lib, ffmpeg, id, &recipe)?;
        let small = self.frame(lib, id, &src, &base, &recipe, ANALYZE_EDGE)?;
        let [fw, fh] = FrameMap::frame_size(base.linear.width, base.linear.height, &recipe);
        let map = FrameMap::new(base.linear.width, base.linear.height, &recipe);
        let moved: Vec<FaceBox> = faces.iter().filter_map(|f| face_in_frame(&map, f, fw, fh)).collect();
        let skin = analyze(&small, &moved).faces.iter().any(|f| (20.0..=50.0).contains(&f.skin_hue));
        let img = small.to_rgb8();
        let sky = sky_share(&regions(lib, id, map.scaled(img.width() as f32 / fw.max(1.0)), &img, ai)?);
        Ok(photo_colour(&img, skin, sky))
    }
}

/// Facet's regions by name, measured in the edited frame.
pub type Regions = Vec<(String, RegionStats)>;

/// "sky", "subject", "background", "person 0": how the tools name a Facet mask.
pub fn ai_name(m: &AiMaskInfo) -> String {
    match m.kind {
        AiMaskKind::Person => format!("person {}", m.instance),
        k => k.as_str().to_string(),
    }
}

/// Each of Facet's masks drawn over `img` (a frame mapped by `map`) as Develop draws an AI
/// shape at render size, so a crop or rotation is honoured, and measured there.
pub fn regions(lib: &Library, id: i64, map: FrameMap, img: &RgbImage, ai: &[AiMaskInfo]) -> AppResult<Regions> {
    if ai.is_empty() {
        return Ok(Vec::new());
    }
    let rasters: AiRasters = lib.db.with_conn(|c| -> photos_core::Result<_> {
        let mut out = AiRasters::new();
        for m in ai {
            if let Some(r) = ai_masks::get(c, id, &m.hash)? {
                out.insert(m.hash.clone(), r);
            }
        }
        Ok(out)
    })?;
    Ok(ai
        .iter()
        .filter(|m| rasters.contains_key(&m.hash))
        .map(|m| {
            let weights = masks::rasterize_with(&ai_mask(m), &map, img.width(), img.height(), Some(&rasters), None);
            (ai_name(m), region_stats(img, &weights))
        })
        .collect())
}

/// A one-shape mask selecting Facet's `m`, as the `mask` tool would add it.
fn ai_mask(m: &AiMaskInfo) -> Mask {
    Mask {
        id: 0,
        name: ai_name(m),
        shapes: vec![MaskShape::Ai { add: true, target: m.kind, instance: m.instance, raster: m.hash.clone() }],
        invert: false,
        opacity: 1.0,
        adjust: Default::default(),
        visible: true,
    }
}

/// A face box (0–1 of the unedited photo) in the edited frame, 0–1; `None` when cropped away.
fn face_in_frame(map: &FrameMap, f: &FaceBox, w: f32, h: f32) -> Option<FaceBox> {
    let corners = [map.to_frame(f.x, f.y), map.to_frame(f.x + f.w, f.y), map.to_frame(f.x, f.y + f.h), map.to_frame(f.x + f.w, f.y + f.h)];
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y] in corners {
        (x0, y0, x1, y1) = (x0.min(x / w), y0.min(y / h), x1.max(x / w), y1.max(y / h));
    }
    let (x0, y0, x1, y1) = (x0.max(0.0), y0.max(0.0), x1.min(1.0), y1.min(1.0));
    (x1 - x0 > 0.01 && y1 - y0 > 0.01).then_some(FaceBox { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_face_follows_the_crop_and_leaves_with_it() {
        let crop = Recipe { crop: Some(Crop { x: 0.5, y: 0.0, w: 0.5, h: 1.0 }), ..Recipe::default() };
        let map = FrameMap::new(200, 100, &crop);
        let inside = face_in_frame(&map, &FaceBox { x: 0.6, y: 0.2, w: 0.1, h: 0.2 }, 100.0, 100.0).unwrap();
        assert!((inside.x - 0.2).abs() < 0.02 && (inside.w - 0.2).abs() < 0.02, "{inside:?}");
        assert_eq!(face_in_frame(&map, &FaceBox { x: 0.1, y: 0.2, w: 0.1, h: 0.2 }, 100.0, 100.0), None);
    }

    #[test]
    fn split_shows_before_left_and_after_right() {
        let b = RgbImage::from_pixel(10, 4, image::Rgb([0, 0, 0]));
        let a = RgbImage::from_pixel(10, 4, image::Rgb([200, 200, 200]));
        let s = split(&b, &a);
        assert_eq!((s.get_pixel(1, 1)[0], s.get_pixel(9, 1)[0]), (0, 200));
    }
}
