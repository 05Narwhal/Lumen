//! The develop pipeline (Phase 8 spec §3): one implementation for the live preview and the
//! export, in linear light, f32, Rec.709/sRGB primaries, unclamped.
//!
//! Order: geometry, Basic (white balance and exposure, then local contrast and dehaze, then
//! the tone ranges, contrast and colour), masks (each one's local Basic, mixed in by its
//! weights), tone curve, HSL, colour grading, sharpening,
//! vignette, grain. A stage whose settings are neutral is skipped.

pub mod analyze;
pub mod basic;
pub mod blur;
pub mod color;
pub mod curve;
pub mod detail;
pub mod effects;
pub mod geometry;
pub mod histogram;
pub mod linear;
pub mod local;
pub mod local_tools;
pub mod masks;
pub mod oklab;
pub mod raw;
pub mod stand_in;

pub use linear::Linear;

/// What a render needs besides the recipe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ctx<'a> {
    /// The rendered image's long edge ÷ the full image's: 1 for an export, less for a
    /// preview. Radii in full-resolution pixels (sharpening, grain) are scaled by it.
    pub scale: f32,
    /// Seeds the grain (from the content hash), so an export is reproducible.
    pub seed: u64,
    /// Where the image rendered sits in its frame, when it is only part of it (the 100 %
    /// view): radii are sized by the frame, the vignette is centred on it and the grain's
    /// lattice is anchored to it, so the part looks as it does in the whole.
    pub region: Option<Region>,
    /// Dehaze's airlight, when the image rendered is only part of the frame and so cannot
    /// find the haziest part of the whole.
    pub airlight: Option<[f32; 3]>,
    /// Where the unedited photo lies in the frame, for masks (`masks::FrameMap::new` of the
    /// frame's base and geometry). Without it masks are skipped. With `region`, the map is
    /// still the whole frame's: the stage moves it to the part.
    pub frame_map: Option<masks::FrameMap>,
    /// The AI rasters the recipe's masks name (`photos_core::ai_masks::rasters_for`). A shape
    /// whose raster isn't here weighs nothing, so every render of a photo must pass them.
    pub ai_rasters: Option<&'a photos_core::ai_masks::AiRasters>,
    /// Whether the render is no longer wanted (Round 24 §6.1: a newer slider position overtook
    /// it). Asked between stages, per mask and in the area tools' blurs; a cancelled render
    /// returns early and its pixels are thrown away.
    pub cancel: Cancel<'a>,
}

/// [`Ctx::cancel`]: a question asked often, so it must be cheap (an atomic load).
#[derive(Clone, Copy, Default)]
pub struct Cancel<'a>(pub Option<&'a (dyn Fn() -> bool + Sync)>);

impl Cancel<'_> {
    pub fn is_set(&self) -> bool {
        self.0.is_some_and(|f| f())
    }
}

impl std::fmt::Debug for Cancel<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "Cancel(Some)" } else { "Cancel(None)" })
    }
}

/// A cancel check never makes two contexts render differently.
impl PartialEq for Cancel<'_> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A part of a frame: its top-left corner and the frame's size, in the part's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub frame_width: u32,
    pub frame_height: u32,
}

impl Default for Ctx<'_> {
    fn default() -> Self {
        Self { scale: 1.0, seed: 0, region: None, airlight: None, frame_map: None, ai_rasters: None, cancel: Cancel::default() }
    }
}

impl Ctx<'_> {
    /// [`Ctx::cancel`] asked.
    pub fn cancelled(&self) -> bool {
        self.cancel.is_set()
    }

    pub fn new(scale: f32, seed: u64) -> Self {
        Self { scale, seed, ..Default::default() }
    }

    /// The frame's long edge: the region's frame, or the image itself.
    pub fn frame_long(&self, img: &Linear) -> f32 {
        self.region.map(|r| r.frame_width.max(r.frame_height)).unwrap_or_else(|| img.long_edge()) as f32
    }

    /// The image's offset in its frame and the frame's size.
    pub fn frame(&self, img: &Linear) -> (u32, u32, u32, u32) {
        self.region.map(|r| (r.x, r.y, r.frame_width, r.frame_height)).unwrap_or((0, 0, img.width, img.height))
    }
}

use std::path::Path;

use image::{DynamicImage, RgbImage};
use photos_core::edits::{Develop, Recipe};

use crate::hdr::HdrKind;
use crate::{apply_orientation, decode_preview_hdr, Ffmpeg, MediaError, MediaKind};

/// A stable grain seed for a photo: the first 64 bits of its content hash, so the same photo
/// gets the same grain wherever it is rendered.
pub fn seed_for(content_hash: &str) -> u64 {
    u64::from_str_radix(content_hash.get(..16).unwrap_or(content_hash), 16).unwrap_or(0)
}

/// Whether a RAW file renders from its sensor data: when the recipe develops it. Unedited or
/// geometry-only RAW keeps the embedded preview, so a crop never changes a photo's colours.
pub fn develops_from_sensor(kind: MediaKind, recipe: Option<&Recipe>) -> bool {
    kind == MediaKind::Raw && recipe.is_some_and(|r| r.clone().normalized().develop.is_some())
}

/// Bake EXIF orientation (1–8) into a linear image, as `apply_orientation` does for decoded
/// ones.
pub fn orient(img: &Linear, orientation: u16) -> Linear {
    use geometry::{flip_h, flip_v, rotate180, rotate270, rotate90};
    match orientation {
        2 => flip_h(img),
        3 => rotate180(img),
        4 => flip_v(img),
        5 => flip_h(&rotate90(img)),
        6 => rotate90(img),
        7 => flip_h(&rotate270(img)),
        8 => rotate270(img),
        _ => img.clone(),
    }
}

/// What a base was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendering {
    /// Sensor data with the base look (a refused RAW's through its converted DNG).
    Sensor,
    /// A RAW asked for as sensor data that could only give its camera JPEG: rawler refused it
    /// and Adobe DNG Converter is not there or failed (`stand_in`). Develop says so.
    Camera,
    /// The decoded file (an image, or a RAW's preview when sensor data was not asked for).
    File,
}

/// A base and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub linear: Linear,
    /// The full-size long edge, which a downscaled base's `Ctx::scale` is measured against.
    pub full_long: u32,
    pub rendering: Rendering,
}

/// The geometry-free, display-oriented linear base a recipe renders from: the sensor data
/// with Lumen's base look when `sensor` (see [`develops_from_sensor`]; Develop always asks
/// for it on a RAW file), otherwise the decoded file. With `max_edge`, downscaled to it —
/// from the 8-bit decode directly, so a full-size float image is never held. Also returns
/// the full-size long edge, which a downscaled base's `Ctx::scale` is measured against.
pub fn load_base(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>, hdr: HdrKind, orientation: u16, sensor: bool, max_edge: Option<u32>) -> Result<(Linear, u32), MediaError> {
    load_base_as(path, kind, ffmpeg, hdr, orientation, sensor, max_edge).map(|l| (l.linear, l.full_long))
}

/// A RAW's sensor data; when rawler refuses it, what stands in for it, with `None` for the
/// camera's JPEG. Unconfigured, the refusal stands. An I/O error is returned as it is.
fn sensor_or_stand_in(path: &Path, decode: stand_in::Decode, stand_in: &dyn Fn(&Path) -> Option<Result<stand_in::Base, MediaError>>) -> Result<Option<Linear>, MediaError> {
    match decode(path) {
        Ok(lin) => Ok(Some(lin)),
        // Nothing can develop a converted DNG either, so nothing is converted. `stand_in::configured`
        // says whether the app is running, as opposed to a tool with no stand-in at all.
        Err(e) if raw::is_no_library(&e) => {
            if stand_in::configured() {
                Ok(None)
            } else {
                Err(e)
            }
        }
        Err(e) if raw::is_refused(&e) => match stand_in(path) {
            None => Err(e),
            Some(Err(io)) => Err(io),
            Some(Ok(stand_in::Base::Sensor(lin))) => Ok(Some(lin)),
            Some(Ok(stand_in::Base::Camera)) => Ok(None),
        },
        Err(e) => Err(e),
    }
}

/// [`load_base`], saying what the base was made from. A RAW that rawler refuses goes through
/// `stand_in` once it is configured: its converted DNG's sensor data, else its camera JPEG
/// (oriented by the same rule as every decoded preview). Unconfigured, the refusal stands.
/// An I/O error is not a refusal: it is returned as it is.
pub fn load_base_as(path: &Path, kind: MediaKind, ffmpeg: Option<&Ffmpeg>, hdr: HdrKind, orientation: u16, sensor: bool, max_edge: Option<u32>) -> Result<Loaded, MediaError> {
    let mut rendering = Rendering::File;
    if sensor && kind == MediaKind::Raw {
        let decoded = sensor_or_stand_in(path, &raw::decode, &stand_in::develop_base)?;
        if decoded.is_none() {
            rendering = Rendering::Camera;
        }
        if let Some(lin) = decoded {
            let lin = orient(&lin, orientation);
            let full_long = lin.long_edge();
            let linear = match max_edge {
                Some(e) => lin.resize_long_edge(e),
                None => lin,
            };
            return Ok(Loaded { linear, full_long, rendering: Rendering::Sensor });
        }
    }
    // An image too large to decode whole (Round 22 §2) is developed at its reduced size, which is
    // then the base's full size.
    let img = match decode_preview_hdr(path, kind, ffmpeg, hdr) {
        Err(e) if crate::decode::is_too_large(&e) => {
            crate::decode::decode_reduced(path, kind, crate::decode::REDUCED_EDGE, crate::decode::oversize_budget()).map(|r| r.image)
        }
        got => got,
    };
    let img = apply_orientation(if rendering == Rendering::Camera { camera_preview(img) } else { img }?, orientation);
    let full_long = img.width().max(img.height());
    let linear = match max_edge {
        Some(e) => Linear::from_rgb8_downscaled(&img.to_rgb8(), e),
        None => Linear::from_dynamic(&img),
    };
    Ok(Loaded { linear, full_long, rendering })
}

/// The camera's JPEG standing in for a refused RAW. A RAW with none (a GoPro GPR, a CHDK dump)
/// keeps its refusal, so Develop says it can't read the file rather than that something failed.
/// An I/O error and a missing RAW library pass as they are.
fn camera_preview(got: Result<image::DynamicImage, MediaError>) -> Result<image::DynamicImage, MediaError> {
    match got {
        Err(MediaError::Decode(why)) => Err(raw::refused(format!("no camera JPEG to stand in ({why})"))),
        other => other,
    }
}

/// Stage 2 (spec §3): the denoised base mixed into the base by `strength` (0 keeps the base,
/// 1 is the network's result). A denoised file of another size is resized to the base's.
pub fn blend_denoised(base: &Linear, denoised: &Linear, strength: f32) -> Linear {
    let s = strength.clamp(0.0, 1.0);
    if s <= 0.0 {
        return base.clone();
    }
    let resized;
    let dn = if (denoised.width, denoised.height) == (base.width, base.height) {
        denoised
    } else {
        resized = denoised.resize(base.width, base.height);
        &resized
    };
    let data = base.data.iter().zip(&dn.data).map(|(b, d)| std::array::from_fn(|c| b[c] + s * (d[c] - b[c]))).collect();
    Linear { width: base.width, height: base.height, data }
}

/// Render `recipe` on a linear base to 8-bit sRGB.
pub fn render_base(base: &Linear, recipe: &Recipe, ctx: &Ctx) -> RgbImage {
    process(base, recipe, ctx).to_rgb8()
}

/// Render `recipe` on a decoded, display-oriented image at full size. What the quick
/// editor's previews and the edited derivatives use when the source is not sensor data.
pub fn render(img: &DynamicImage, recipe: &Recipe, seed: u64) -> DynamicImage {
    DynamicImage::ImageRgb8(render_base(&Linear::from_dynamic(img), recipe, &Ctx::new(1.0, seed)))
}

/// Run the recipe on a geometry-free, display-oriented linear base: geometry, then every
/// develop stage in the spec's order. The recipe is normalised first.
pub fn process(base: &Linear, recipe: &Recipe, ctx: &Ctx) -> Linear {
    let r = recipe.clone().normalized();
    let mut img = geometry::apply(base, &r);
    if let Some(d) = &r.develop {
        let ctx = Ctx { frame_map: Some(masks::FrameMap::new(base.width, base.height, &r)), ..*ctx };
        develop_stages(&mut img, d, &ctx);
    }
    img
}

/// Every stage after geometry, in order, on an image already in its frame (or, with
/// `Ctx::region`, a part of one).
///
/// A cancelled render ([`Ctx::cancel`]) stops at its next check and leaves `img` part-done.
pub fn develop_stages(img: &mut Linear, d: &Develop, ctx: &Ctx) {
    // Before global Basic, so moving a slider never moves an AI mask's edge.
    let guide = masks::guide_for(img, &d.masks, ctx);
    if ctx.cancelled() {
        return;
    }
    basic::apply_gains(img, &d.basic);
    local::texture_clarity(img, d.basic.texture, d.basic.clarity, ctx);
    if ctx.cancelled() {
        return;
    }
    local::dehaze(img, d.basic.dehaze, ctx);
    basic::apply_tones(img, &d.basic);
    if ctx.cancelled() {
        return;
    }
    let masked = masks::apply(img, &d.masks, ctx, guide.as_ref());
    if ctx.cancelled() {
        return;
    }
    curve::apply(img, &d.curve);
    color::apply(img, &d.hsl, &d.grading);
    // Each area stage: the photo's, then each mask's own on the part it covers.
    use local_tools::Tool;
    let stages: [&dyn Fn(&mut Linear); 11] = [
        &|img| detail::sharpen_with(img, &d.detail.sharpen, ctx, masked.sharpness.as_deref()),
        &|img| masked.run(img, ctx, Tool::sharpen),
        &|img| effects::motion_blur(img, &d.effects.motion_blur, ctx),
        &|img| masked.run(img, ctx, Tool::motion_blur),
        &|img| effects::glow(img, &d.effects.glow, ctx),
        &|img| masked.run(img, ctx, Tool::glow),
        &|img| effects::halation(img, &d.effects.halation, ctx),
        &|img| masked.run(img, ctx, Tool::halation),
        &|img| effects::vignette(img, &d.effects.vignette, ctx),
        &|img| effects::grain(img, &d.effects.grain, ctx),
        &|img| masked.run(img, ctx, Tool::grain),
    ];
    for stage in stages {
        if ctx.cancelled() {
            return;
        }
        stage(img);
    }
}

/// Dehaze's airlight for a frame about to be rendered in parts: the frame's gains and local
/// contrast applied to a small copy, then its haziest cells.
pub fn frame_airlight(frame: &Linear, d: &Develop) -> [f32; 3] {
    let mut small = frame.resize_long_edge(512);
    basic::apply_gains(&mut small, &d.basic);
    local::texture_clarity(&mut small, d.basic.texture, d.basic.clarity, &Ctx::default());
    local::airlight(&small)
}

#[cfg(test)]
mod tests {
    #[test]
    fn blending_denoise_is_a_lerp_by_strength_and_resizes() {
        let base = super::Linear::new(40, 20, [0.2, 0.4, 0.6]);
        let dn = super::Linear::new(80, 40, [0.4, 0.4, 0.4]);
        let half = super::blend_denoised(&base, &dn, 0.5);
        assert_eq!((half.width, half.height), (40, 20));
        assert!((half.get(3, 3)[0] - 0.3).abs() < 1e-5 && (half.get(3, 3)[2] - 0.5).abs() < 1e-5);
        assert_eq!(super::blend_denoised(&base, &dn, 0.0).data, base.data);
    }

    #[test]
    fn a_refused_raw_without_a_camera_jpeg_keeps_its_refusal() {
        let none = super::camera_preview(Err(super::MediaError::Decode("no embedded preview in RAW".into()))).unwrap_err();
        assert!(super::raw::is_refused(&none) && !super::raw::is_no_library(&none), "{none}");
        let io = super::camera_preview(Err(super::MediaError::Io(std::io::Error::other("unplugged")))).unwrap_err();
        assert!(matches!(io, super::MediaError::Io(_)));
        assert!(super::raw::is_no_library(&super::camera_preview(Err(super::raw::no_library())).unwrap_err()));
        assert!(super::camera_preview(Ok(image::DynamicImage::new_rgb8(2, 2))).is_ok());
    }

    use super::*;
    use photos_core::edits::{Basic, Effects, Sharpen, Vignette};

    /// Structure at several scales, all coarser than the downscale can alias: gradients,
    /// waves and a few hard edges.
    fn scene(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let wave = (u * 31.0).sin() * 0.5 + (v * 19.0 + u * 7.0).sin() * 0.4;
                let edge = if (u * 6.0).floor() as i32 % 2 == 0 && v > 0.5 { 0.6 } else { 0.0 };
                let s = -1.5 + 2.2 * u - 1.0 * v + wave + edge;
                let l = 0.18 * 2f32.powf(s);
                img.data[(y * w + x) as usize] = [l * (0.8 + 0.4 * v), l, l * (1.2 - 0.4 * u)];
            }
        }
        img
    }

    fn psnr(a: &image::RgbImage, b: &image::RgbImage) -> f64 {
        let mse = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as f64 - *y as f64).powi(2)).sum::<f64>() / a.as_raw().len() as f64;
        10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
    }

    #[test]
    fn a_preview_looks_like_its_export() {
        let full = scene(3000, 2000);
        let recipe = Recipe {
            develop: Some(Develop {
                basic: Basic { exposure: 0.3, texture: 0.5, clarity: 0.6, dehaze: 0.3, highlights: -0.4, shadows: 0.3, contrast: 0.2, vibrance: 0.2, ..Default::default() },
                detail: photos_core::edits::Detail { sharpen: Sharpen { amount: 1.0, radius: 1.0, detail: 0.3, masking: 0.2 }, denoise: None },
                effects: Effects { vignette: Vignette { amount: -0.5, midpoint: 0.4, feather: 0.6 }, ..Default::default() },
                ..Default::default()
            }),
            ..Recipe::identity()
        };
        let exported = process(&full, &recipe, &Ctx::new(1.0, 1)).resize(1000, 667).to_rgb8();
        let preview = process(&full.resize(1000, 667), &recipe, &Ctx::new(1.0 / 3.0, 1)).to_rgb8();
        let db = psnr(&exported, &preview);
        assert!(db >= 32.0, "preview vs export: {db:.1} dB");
    }

    #[test]
    fn render_of_the_identity_returns_the_pixels() {
        let img = image::RgbImage::from_fn(7, 5, |x, y| image::Rgb([(x * 30) as u8, (y * 40) as u8, 200]));
        let out = render(&DynamicImage::ImageRgb8(img.clone()), &Recipe::identity(), 0);
        assert_eq!(out.to_rgb8().as_raw(), img.as_raw());
    }

    #[test]
    fn a_version_one_tone_recipe_still_brightens() {
        let r: Recipe = serde_json::from_str(r#"{"version":1,"crop":null,"rotate":0,"straighten":0,"flip_h":false,"exposure":1.0}"#).unwrap();
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(2, 2, image::Rgb([118, 118, 118])));
        let v = render(&img, &r, 0).to_rgb8().get_pixel(0, 0)[0];
        assert!((v as i32 - 161).abs() <= 2, "{v}");
    }

    #[test]
    fn orientation_matches_the_decoded_images_rule() {
        let mut img = image::RgbImage::new(3, 2);
        img.put_pixel(0, 0, image::Rgb([255, 0, 0]));
        img.put_pixel(2, 1, image::Rgb([0, 255, 0]));
        let lin = Linear::from_dynamic(&DynamicImage::ImageRgb8(img.clone()));
        for o in 1..=8u16 {
            let want = apply_orientation(DynamicImage::ImageRgb8(img.clone()), o).to_rgb8();
            assert_eq!(orient(&lin, o).to_rgb8(), want, "orientation {o}");
        }
    }

    #[test]
    fn only_a_developed_raw_reads_its_sensor() {
        let dev = Recipe { develop: Some(Develop { basic: Basic { exposure: 0.5, ..Default::default() }, ..Default::default() }), ..Recipe::identity() };
        let crop = Recipe { rotate: 90, ..Recipe::identity() };
        assert!(develops_from_sensor(MediaKind::Raw, Some(&dev)));
        assert!(!develops_from_sensor(MediaKind::Raw, Some(&crop)));
        assert!(!develops_from_sensor(MediaKind::Raw, None));
        assert!(!develops_from_sensor(MediaKind::Image, Some(&dev)));
        assert_eq!(seed_for("0123456789abcdef00"), 0x0123_4567_89ab_cdef);
    }

    #[test]
    fn a_region_renders_as_the_same_part_of_the_whole() {
        let frame = scene(600, 400);
        let d = Develop {
            basic: Basic { exposure: 0.2, clarity: 0.5, texture: 0.3, dehaze: 0.3, contrast: 0.2, ..Default::default() },
            effects: Effects { vignette: Vignette { amount: -0.6, midpoint: 0.3, feather: 0.4 }, grain: photos_core::edits::Grain { amount: 0.5, size: 0.3 }, ..Default::default() },
            ..Default::default()
        };
        let mut whole = frame.clone();
        develop_stages(&mut whole, &d, &Ctx::new(1.0, 9));
        // A 200 x 150 part at (330, 210), rendered with a 40 px margin the way the 100 % view does.
        let (x, y, w, h, m) = (330u32, 210u32, 200u32, 150u32, 40u32);
        let (px, py) = (x - m, y - m);
        let (pw, ph) = ((w + 2 * m).min(600 - px), (h + 2 * m).min(400 - py));
        let mut part = geometry::crop(&frame, px, py, pw, ph);
        let ctx = Ctx { region: Some(Region { x: px, y: py, frame_width: 600, frame_height: 400 }), airlight: Some(frame_airlight(&frame, &d)), ..Ctx::new(1.0, 9) };
        develop_stages(&mut part, &d, &ctx);
        let got = geometry::crop(&part, m, m, w, h).to_rgb8();
        let want = geometry::crop(&whole, x, y, w, h).to_rgb8();
        let db = psnr(&got, &want);
        assert!(db >= 35.0, "region vs whole: {db:.1} dB");
    }

    fn radial_mask(adjust: photos_core::edits::LocalAdjust) -> photos_core::edits::Mask {
        photos_core::edits::Mask {
            id: 1,
            name: "Spot".into(),
            shapes: vec![photos_core::edits::MaskShape::Radial { add: true, center: [0.3, 0.4], radius: [0.15, 0.2], angle: 0.0, feather: 0.0, inside: true }],
            invert: false,
            opacity: 1.0,
            adjust,
            visible: true,
        }
    }

    fn with_masks(masks: Vec<photos_core::edits::Mask>) -> Recipe {
        Recipe { develop: Some(Develop { basic: Basic { contrast: 0.2, ..Default::default() }, masks, ..Default::default() }), ..Recipe::identity() }
    }

    #[test]
    fn a_plus_one_ev_radial_brightens_only_inside_it() {
        use photos_core::edits::LocalAdjust;
        let img = Linear::new(200, 100, [0.1, 0.1, 0.1]);
        let plain = process(&img, &with_masks(vec![]), &Ctx::default());
        let lit = process(&img, &with_masks(vec![radial_mask(LocalAdjust { exposure: 1.0, ..Default::default() })]), &Ctx::default());
        let ratio = lit.get(60, 40)[1] / plain.get(60, 40)[1];
        assert!((ratio - 2.0).abs() < 0.05, "one stop inside: ×{ratio}");
        assert_eq!(lit.get(150, 80), plain.get(150, 80), "outside nothing moves");
        // Turned and cropped, the mask stays on the same pixels of the photo.
        let turned = Recipe { rotate: 90, crop: Some(photos_core::edits::Crop { x: 0.0, y: 0.1, w: 1.0, h: 0.7 }), ..with_masks(vec![radial_mask(LocalAdjust { exposure: 1.0, ..Default::default() })]) };
        let out = process(&img, &turned, &Ctx::default());
        let f = masks::FrameMap::new(200, 100, &turned);
        let p = f.to_frame(0.3, 0.4);
        assert!(out.get(p[0] as u32, p[1] as u32)[1] > 0.15, "the centre is still lit after a turn and a crop");
    }

    #[test]
    fn a_v2_recipe_renders_exactly_as_before() {
        let img = scene(120, 80);
        let mut plain = with_masks(vec![]);
        plain.develop.as_mut().unwrap().basic.clarity = 0.4;
        let mut idle = plain.clone();
        idle.develop.as_mut().unwrap().masks = vec![radial_mask(Default::default())];
        let want = process(&img, &plain, &Ctx::new(1.0, 3));
        assert_eq!(process(&img, &idle, &Ctx::new(1.0, 3)), want, "a mask with nothing to do changes no pixel");
        let mut hidden = radial_mask(photos_core::edits::LocalAdjust { exposure: 2.0, ..Default::default() });
        hidden.visible = false;
        idle.develop.as_mut().unwrap().masks = vec![hidden];
        assert_eq!(process(&img, &idle, &Ctx::new(1.0, 3)), want, "nor a hidden one");
    }

    #[test]
    fn local_sharpness_sharpens_only_under_the_mask() {
        use photos_core::edits::LocalAdjust;
        let mut img = Linear::new(200, 100, [0.1; 3]);
        for y in 0..100 {
            for x in (0..200).filter(|x| (x / 4) % 2 == 0) {
                img.data[y * 200 + x] = [0.3; 3];
            }
        }
        let plain = process(&img, &with_masks(vec![]), &Ctx::default());
        let sharp = process(&img, &with_masks(vec![radial_mask(LocalAdjust { sharpness: 1.0, ..Default::default() })]), &Ctx::default());
        let spread = |l: &Linear, x0: u32| (x0..x0 + 8).map(|x| l.get(x, 40)[1]).fold(0.0f32, f32::max) - (x0..x0 + 8).map(|x| l.get(x, 40)[1]).fold(1.0f32, f32::min);
        assert!(spread(&sharp, 56) > spread(&plain, 56) * 1.05, "inside the edges get crisper");
        assert_eq!(sharp.get(170, 90), plain.get(170, 90));
    }

    #[test]
    fn a_masked_region_renders_as_the_same_part_of_the_whole() {
        use photos_core::edits::{LocalAdjust, MaskShape};
        let frame = scene(600, 400);
        let mut soft = radial_mask(LocalAdjust { exposure: 0.7, clarity: 0.5, contrast: 0.3, saturation: -0.4, temperature: 20.0, ..Default::default() });
        soft.shapes = vec![
            MaskShape::Radial { add: true, center: [0.6, 0.6], radius: [0.25, 0.3], angle: 20.0, feather: 0.6, inside: true },
            MaskShape::Linear { add: false, from: [0.0, 0.9], to: [0.0, 0.5] },
            MaskShape::Brush { add: true, radius: 0.04, feather: 0.5, flow: 0.8, points: vec![[1500, 2000], [3000, 3200], [3500, 2500]] },
        ];
        let d = Develop { basic: Basic { exposure: 0.2, clarity: 0.3, ..Default::default() }, masks: vec![soft], ..Default::default() };
        let map = masks::FrameMap::identity(600, 400);
        let mut whole = frame.clone();
        develop_stages(&mut whole, &d, &Ctx { frame_map: Some(map), ..Ctx::new(1.0, 9) });
        let (x, y, w, h, m) = (330u32, 210u32, 200u32, 150u32, 40u32);
        let (px, py) = (x - m, y - m);
        let (pw, ph) = ((w + 2 * m).min(600 - px), (h + 2 * m).min(400 - py));
        let mut part = geometry::crop(&frame, px, py, pw, ph);
        let ctx = Ctx { region: Some(Region { x: px, y: py, frame_width: 600, frame_height: 400 }), airlight: Some(frame_airlight(&frame, &d)), frame_map: Some(map), ..Ctx::new(1.0, 9) };
        develop_stages(&mut part, &d, &ctx);
        let got = geometry::crop(&part, m, m, w, h).to_rgb8();
        let want = geometry::crop(&whole, x, y, w, h).to_rgb8();
        assert_ne!(want, geometry::crop(&{ let mut p = frame.clone(); develop_stages(&mut p, &Develop { masks: vec![], ..d.clone() }, &Ctx::new(1.0, 9)); p }, x, y, w, h).to_rgb8(), "the mask shows in this part");
        let db = psnr(&got, &want);
        assert!(db >= 35.0, "masked region vs whole: {db:.1} dB");
    }

    /// A bright top and a dark textured bottom, and a raster of the top that is a cell off.
    fn ai_scene(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        for (i, p) in img.data.iter_mut().enumerate() {
            let (u, v) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
            let line = 0.5 + 0.03 * (u * 25.0).sin();
            let l = if v < line { 0.6 } else { 0.05 + 0.02 * (u * 40.0).sin() * (v * 40.0).cos() };
            *p = [l, l, l * 0.9];
        }
        img
    }

    fn ai_store() -> photos_core::ai_masks::AiRasters {
        let (rw, rh) = (160u32, 107u32);
        let weights = (0..rh)
            .flat_map(|y| (0..rw).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (u, v) = ((x as f32 + 0.5) / rw as f32, (y as f32 + 0.5) / rh as f32);
                if v < 0.5 + 0.03 * (u * 25.0).sin() + 1.0 / rh as f32 { 255 } else { 0 }
            })
            .collect();
        let raster = photos_core::ai_masks::AiMaskRaster { kind: photos_core::ai_masks::AiMaskKind::Sky, instance: 0, width: rw, height: rh, weights };
        photos_core::ai_masks::AiRasters::from([("aa".to_string(), raster)])
    }

    fn ai_mask(adjust: photos_core::edits::LocalAdjust) -> photos_core::edits::Mask {
        let shape = photos_core::edits::MaskShape::Ai { add: true, target: photos_core::ai_masks::AiMaskKind::Sky, instance: 0, raster: "aa".into() };
        photos_core::edits::Mask { shapes: vec![shape], ..radial_mask(adjust) }
    }

    #[test]
    fn moving_a_global_slider_never_moves_an_ai_masks_edge() {
        use photos_core::edits::LocalAdjust;
        let img = ai_scene(600, 400);
        let store = ai_store();
        let ctx = Ctx { ai_rasters: Some(&store), ..Ctx::new(1.0, 1) };
        let at = |exposure: f32| {
            let d = Develop { basic: Basic { exposure, ..Default::default() }, masks: vec![ai_mask(LocalAdjust { exposure: -1.0, ..Default::default() })], ..Default::default() };
            process(&img, &Recipe { develop: Some(d), ..Recipe::identity() }, &ctx)
        };
        let (a, b) = (at(0.0), at(1.5));
        // Weights unchanged means every pixel moves by the global gain alone.
        let want = 2f32.powf(1.5);
        let worst = a.data.iter().zip(&b.data).filter(|(p, _)| p[1] > 0.01).map(|(p, q)| (q[1] / p[1] - want).abs() / want).fold(0.0f32, f32::max);
        assert!(worst < 1e-3, "a pixel moved by more than the gain: {worst}");
    }

    /// A mask over the whole frame: no shape, inverted.
    fn everywhere(adjust: photos_core::edits::LocalAdjust, opacity: f32) -> photos_core::edits::Mask {
        photos_core::edits::Mask { shapes: vec![], invert: true, opacity, ..radial_mask(adjust) }
    }

    fn assert_close(got: &Linear, want: &Linear, what: &str) {
        let worst = got.data.iter().zip(&want.data).flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]).abs() / q[c].abs().max(1.0))).fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "{what}: off by {worst}");
    }

    #[test]
    fn a_masks_curve_hsl_grading_and_vibrance_are_the_photos_own_at_the_mask_stage() {
        use photos_core::edits::{Curve, Grading, Hsl, LocalAdjust, Wheel};
        let img = scene(96, 64);
        let ctx = Ctx::new(1.0, 1);
        let base = Basic { contrast: 0.2, exposure: 0.3, ..Default::default() };
        let mut hsl = [Hsl::default(); 8];
        hsl[0] = Hsl { h: 0.3, s: 0.4, l: -0.2 };
        hsl[5] = Hsl { h: -0.2, s: -0.5, l: 0.3 };
        let curve = Curve { master: vec![[0.0, 0.05], [0.4, 0.55], [1.0, 0.95]], g: vec![[0.0, 0.0], [0.5, 0.45], [1.0, 1.0]], ..Curve::default() };
        let grading = Grading { shadows: Wheel { hue: 220.0, sat: 0.4, lum: -0.1 }, highlights: Wheel { hue: 40.0, sat: 0.3, lum: 0.1 }, ..Grading::default() };
        // (name, the photo's own develop with the tool, the tool as a mask's).
        let cases: [(&str, Develop, LocalAdjust); 4] = [
            ("vibrance", Develop { basic: Basic { vibrance: 0.6, ..base }, ..Default::default() }, LocalAdjust { vibrance: 0.6, ..Default::default() }),
            ("curve", Develop { basic: base, curve: curve.clone(), ..Default::default() }, LocalAdjust { curve, ..Default::default() }),
            ("hsl", Develop { basic: base, hsl, ..Default::default() }, LocalAdjust { hsl, ..Default::default() }),
            ("grading", Develop { basic: base, grading, ..Default::default() }, LocalAdjust { grading, ..Default::default() }),
        ];
        let render = |d: Develop| process(&img, &Recipe { develop: Some(d), ..Recipe::identity() }, &ctx);
        let plain = render(Develop { basic: base, ..Default::default() });
        for (name, global, local) in cases {
            let want = render(global);
            assert_ne!(want, plain, "{name} changes the photo");
            let full = render(Develop { basic: base, masks: vec![everywhere(local.clone(), 1.0)], ..Default::default() });
            assert_close(&full, &want, &format!("{name} at full weight"));
            let half = render(Develop { basic: base, masks: vec![everywhere(local, 0.5)], ..Default::default() });
            // The weight mixes the result in; glow and halation also take their light from the
            // mask's own pixels by it, so a uniform half weight adds a quarter of their light.
            let k = if name == "glow" || name == "halation" { 0.25 } else { 0.5 };
            let mix = Linear { data: plain.data.iter().zip(&full.data).map(|(p, q)| std::array::from_fn(|c| p[c] + k * (q[c] - p[c]))).collect(), ..plain.clone() };
            assert_close(&half, &mix, &format!("{name} at half weight"));
        }
    }

    #[test]
    fn a_masks_curve_runs_after_its_clarity_in_the_copied_box_too() {
        use photos_core::edits::{Curve, LocalAdjust};
        let img = scene(96, 64);
        let ctx = Ctx::new(1.0, 1);
        let curve = Curve { master: vec![[0.0, 0.1], [1.0, 0.9]], ..Curve::default() };
        let render = |a: LocalAdjust| process(&img, &with_masks(vec![everywhere(a, 1.0)]), &ctx);
        let clarity = render(LocalAdjust { clarity: 0.5, ..Default::default() });
        let both = render(LocalAdjust { clarity: 0.5, curve: curve.clone(), ..Default::default() });
        let mut want = clarity.clone();
        curve::apply(&mut want, &curve);
        assert_close(&both, &want, "clarity, then the curve");
    }

    /// Round 24 §6.1: a render whose request was overtaken stops at its next check, and one
    /// that is asked but never cancelled renders exactly as one that is never asked.
    #[test]
    fn a_cancelled_render_stops_and_an_uncancelled_one_is_unchanged() {
        use photos_core::edits::{Halation, LocalAdjust, Mask, MaskShape};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let img = scene(160, 100);
        let shape = MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.3, 0.35], angle: 0.0, feather: 0.5, inside: true };
        let adjust = LocalAdjust { exposure: 0.5, halation: Halation { amount: 1.0, size: 0.8 }, ..Default::default() };
        let mask = Mask { id: 1, name: "Ellipse".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust, visible: true };
        let d = Develop { basic: Basic { exposure: 1.5, ..Default::default() }, masks: vec![mask], ..Default::default() };
        let ctx = Ctx { frame_map: Some(masks::FrameMap::new(160, 100, &Recipe::identity())), ..Ctx::new(1.0, 7) };
        let mut plain = img.clone();
        develop_stages(&mut plain, &d, &ctx);
        assert_ne!(plain, img, "the recipe changes the photo");

        let never = || false;
        let mut asked = img.clone();
        develop_stages(&mut asked, &d, &Ctx { cancel: Cancel(Some(&never)), ..ctx });
        assert_eq!(asked, plain, "asking costs nothing");

        let checks = AtomicUsize::new(0);
        let at_once = || {
            checks.fetch_add(1, Ordering::Relaxed);
            true
        };
        let mut stopped = img.clone();
        develop_stages(&mut stopped, &d, &Ctx { cancel: Cancel(Some(&at_once)), ..ctx });
        assert_eq!(stopped, img, "nothing past the first check runs");
        assert_eq!(checks.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_masks_sharpening_grain_glow_halation_and_motion_blur_are_the_photos_own_inside_it_only() {
        use photos_core::edits::{Detail, Glow, Grain, Halation, LocalAdjust, LocalSharpen, MotionBlur};
        let img = scene(120, 80);
        let ctx = Ctx::new(1.0, 7);
        // Bright enough for halation's highlights.
        let base = Basic { contrast: 0.2, exposure: 1.5, ..Default::default() };
        let effects = |e: Effects| Develop { basic: base, effects: e, ..Default::default() };
        // (name, the photo's own develop with the tool, the tool as a mask's).
        let cases: [(&str, Develop, LocalAdjust); 5] = [
            (
                "sharpen",
                Develop { basic: base, detail: Detail { sharpen: Sharpen { amount: 0.8, radius: 2.0, detail: 0.6, masking: 0.2 }, denoise: None }, ..Default::default() },
                LocalAdjust { sharpness: 0.8, sharpen: Some(LocalSharpen { radius: 2.0, detail: 0.6, masking: 0.2 }), ..Default::default() },
            ),
            ("grain", effects(Effects { grain: Grain { amount: 0.6, size: 0.5 }, ..Default::default() }), LocalAdjust { grain: Grain { amount: 0.6, size: 0.5 }, ..Default::default() }),
            ("glow", effects(Effects { glow: Glow { amount: 0.7, size: 0.5 }, ..Default::default() }), LocalAdjust { glow: Glow { amount: 0.7, size: 0.5 }, ..Default::default() }),
            ("halation", effects(Effects { halation: Halation { amount: 0.8, size: 0.4 }, ..Default::default() }), LocalAdjust { halation: Halation { amount: 0.8, size: 0.4 }, ..Default::default() }),
            ("motion blur", effects(Effects { motion_blur: MotionBlur { amount: 0.5, angle: 30.0 }, ..Default::default() }), LocalAdjust { motion_blur: MotionBlur { amount: 0.5, angle: 30.0 }, ..Default::default() }),
        ];
        let recipe = |d: Develop| Recipe { develop: Some(d), ..Recipe::identity() };
        let render = |d: Develop| process(&img, &recipe(d), &ctx);
        let plain = render(Develop { basic: base, ..Default::default() });
        for (name, global, local) in cases {
            let want = render(global);
            assert_ne!(want, plain, "{name} changes the photo");
            let full = render(Develop { basic: base, masks: vec![everywhere(local.clone(), 1.0)], ..Default::default() });
            assert_close(&full, &want, &format!("{name} at full weight"));
            let half = render(Develop { basic: base, masks: vec![everywhere(local.clone(), 0.5)], ..Default::default() });
            // The weight mixes the result in; glow and halation also take their light from the
            // mask's own pixels by it, so a uniform half weight adds a quarter of their light.
            let k = if name == "glow" || name == "halation" { 0.25 } else { 0.5 };
            let mix = Linear { data: plain.data.iter().zip(&full.data).map(|(p, q)| std::array::from_fn(|c| p[c] + k * (q[c] - p[c]))).collect(), ..plain.clone() };
            assert_close(&half, &mix, &format!("{name} at half weight"));
            // A hard radial on the bright side (glow needs highlights): inside it changes,
            // outside it no pixel does.
            let shape = photos_core::edits::MaskShape::Radial { add: true, center: [0.7, 0.6], radius: [0.15, 0.2], angle: 0.0, feather: 0.0, inside: true };
            let spot = photos_core::edits::Mask { shapes: vec![shape], ..radial_mask(local) };
            let d = Develop { basic: base, masks: vec![spot.clone()], ..Default::default() };
            let out = render(d);
            let r = recipe(Develop::default());
            let weights = masks::rasterize(&spot, &masks::FrameMap::new(img.width, img.height, &r), img.width, img.height);
            assert!(weights.contains(&0.0) && weights.contains(&1.0));
            let (mut inside, mut outside) = (false, true);
            for ((o, p), w) in out.data.iter().zip(&plain.data).zip(&weights) {
                if *w == 0.0 {
                    outside &= o == p;
                } else if *w == 1.0 {
                    inside |= o != p;
                }
            }
            assert!(inside && outside, "{name}: changes inside {inside}, nothing outside {outside}");
        }
    }

    /// A hard-edged mask of exactly the right half of the photo.
    fn right_half(adjust: photos_core::edits::LocalAdjust) -> photos_core::edits::Mask {
        let shape = photos_core::edits::MaskShape::Linear { add: true, from: [0.5001, 0.5], to: [0.5, 0.5] };
        photos_core::edits::Mask { shapes: vec![shape], ..radial_mask(adjust) }
    }

    /// The pixels where `mask` weighs exactly 1, as indices into an image of `img`'s size.
    fn full_weight(img: &Linear, mask: &photos_core::edits::Mask) -> Vec<usize> {
        let r = Recipe::identity();
        let weights = masks::rasterize(mask, &masks::FrameMap::new(img.width, img.height, &r), img.width, img.height);
        assert!(weights.contains(&0.0) && weights.contains(&1.0));
        weights.iter().enumerate().filter(|(_, w)| **w == 1.0).map(|(i, _)| i).collect()
    }

    /// The largest channel difference between two renders over the pixels `at`.
    fn largest_difference(at: &[usize], a: &Linear, b: &Linear) -> f32 {
        at.iter().flat_map(|&i| (0..3).map(move |c| (a.data[i][c] - b.data[i][c]).abs())).fold(0.0f32, f32::max)
    }

    #[test]
    fn a_masks_motion_blur_never_drags_in_what_is_outside_it() {
        use photos_core::edits::{LocalAdjust, MotionBlur};
        // Left half red, right half blue; the streak (19 px) is far wider than the edge's reach.
        let (w, h) = (640u32, 64u32);
        let mut img = Linear::new(w, h, [0.0; 3]);
        for (i, p) in img.data.iter_mut().enumerate() {
            *p = if (i as u32 % w) < w / 2 { [0.8, 0.0, 0.0] } else { [0.0, 0.0, 0.8] };
        }
        let ctx = Ctx::new(1.0, 1);
        let mask = right_half(LocalAdjust { motion_blur: MotionBlur { amount: 0.5, angle: 0.0 }, ..Default::default() });
        let render = |m: Vec<photos_core::edits::Mask>| process(&img, &Recipe { develop: Some(Develop { masks: m, ..Default::default() }), ..Recipe::identity() }, &ctx);
        let (plain, blurred) = (render(vec![]), render(vec![mask.clone()]));
        let inside = full_weight(&img, &mask);
        let worst = largest_difference(&inside, &blurred, &plain);
        assert!(worst < 1e-3, "red crept {worst} into the masked blue");
    }

    /// Round 22 A1: hovering *B&W* over a photo with a motion-blurred mask showed a coloured
    /// square (the mask's box). The preset's global values and the mask's patch must come from
    /// one frame.
    #[test]
    fn a_preset_over_a_masked_area_tool_leaves_no_patch() {
        use photos_core::edits::{LocalAdjust, Mask, MaskShape, MotionBlur};
        let (w, h) = (400u32, 300u32);
        let mut img = Linear::new(w, h, [0.0; 3]);
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                img.data[(y * w + x) as usize] = [0.05 + 0.8 * u, 0.05 + 0.8 * v, 0.05 + 0.8 * (1.0 - u) * (1.0 - v)];
            }
        }
        let ctx = Ctx::new(1.0, 1);
        // x 100–300, y 80–220 of 400 × 300: the box a hard ellipse of these radii covers.
        let shape = MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.25, 140.0 / 600.0], angle: 0.0, feather: 0.0, inside: true };
        let mask = Mask { shapes: vec![shape], ..radial_mask(LocalAdjust { motion_blur: MotionBlur { amount: 0.5, angle: 0.0 }, ..Default::default() }) };
        let bw = photos_core::develop_presets::builtin_presets().into_iter().find(|p| p.name == "B&W").expect("the B&W preset").develop;
        let recipe = |d: Develop| Recipe { develop: Some(d), ..Recipe::identity() };
        // The hover: the draft's develop with the preset's Basic merged in, masks untouched.
        let hovered = Develop { basic: bw.basic, masks: vec![mask.clone()], ..Default::default() };
        let out = process(&img, &recipe(hovered), &ctx);
        let worst = out.data.iter().map(|p| (p[0] - p[1]).abs().max((p[1] - p[2]).abs())).fold(0.0f32, f32::max);
        assert!(worst <= 2.0 / 255.0, "a coloured pixel survives the preset: {worst}");
        // Applying it for real: the same Develop, built by the click's merge.
        let mut applied = Develop { masks: vec![mask], ..Default::default() };
        applied.basic = bw.basic;
        let again = process(&img, &recipe(applied), &ctx);
        let apart = out.data.iter().zip(&again.data).flat_map(|(p, q)| (0..3).map(move |c| (p[c] - q[c]).abs())).fold(0.0f32, f32::max);
        assert!(apart <= 1.0 / 255.0, "hover and click differ by {apart}");
    }

    #[test]
    fn a_masks_glow_takes_no_light_from_outside_it() {
        use photos_core::edits::{Glow, LocalAdjust};
        // Mid grey, with a white square just outside the mask's left edge.
        let (w, h) = (320u32, 96u32);
        let mut img = Linear::new(w, h, [0.1; 3]);
        for y in 30..66 {
            for x in 120..156 {
                img.data[(y * w + x) as usize] = [1.0; 3];
            }
        }
        let ctx = Ctx::new(1.0, 1);
        let mask = right_half(LocalAdjust { glow: Glow { amount: 1.0, size: 0.5 }, ..Default::default() });
        let render = |m: Vec<photos_core::edits::Mask>| process(&img, &Recipe { develop: Some(Develop { masks: m, ..Default::default() }), ..Recipe::identity() }, &ctx);
        let (plain, glowing) = (render(vec![]), render(vec![mask.clone()]));
        let inside = full_weight(&img, &mask);
        let worst = largest_difference(&inside, &glowing, &plain);
        assert!(worst < 1e-5, "the square outside lit the mask by {worst}");
    }

    #[test]
    fn a_masks_area_tools_at_their_defaults_change_nothing() {
        use photos_core::edits::{Glow, Grain, Halation, LocalAdjust, MotionBlur};
        let img = scene(96, 64);
        let ctx = Ctx::new(1.0, 1);
        let a = LocalAdjust { exposure: 0.5, sharpness: 0.4, ..Default::default() };
        // Amount 0 with another size or angle is still nothing.
        let idle = LocalAdjust { grain: Grain { amount: 0.0, size: 0.9 }, glow: Glow { amount: 0.0, size: 0.1 }, halation: Halation { amount: 0.0, size: 0.9 }, motion_blur: MotionBlur { amount: 0.0, angle: 45.0 }, ..a.clone() };
        let render = |a: LocalAdjust| process(&img, &with_masks(vec![radial_mask(a)]), &ctx);
        assert_eq!(render(idle), render(a));
    }

    #[test]
    fn a_mask_whose_raster_is_missing_renders_as_if_it_werent_there() {
        use photos_core::edits::LocalAdjust;
        let img = ai_scene(240, 160);
        let lit = with_masks(vec![ai_mask(LocalAdjust { exposure: 2.0, ..Default::default() })]);
        let want = process(&img, &with_masks(vec![]), &Ctx::new(1.0, 1));
        assert_eq!(process(&img, &lit, &Ctx::new(1.0, 1)), want, "no rasters given");
        let none = photos_core::ai_masks::AiRasters::new();
        assert_eq!(process(&img, &lit, &Ctx { ai_rasters: Some(&none), ..Ctx::new(1.0, 1) }), want, "none for this photo");
        let store = ai_store();
        assert_ne!(process(&img, &lit, &Ctx { ai_rasters: Some(&store), ..Ctx::new(1.0, 1) }), want, "and with it, it shows");
    }

    #[test]
    fn an_ai_masked_region_renders_as_the_same_part_of_the_whole() {
        use photos_core::edits::LocalAdjust;
        let frame = ai_scene(1500, 1000);
        let store = ai_store();
        let d = Develop { basic: Basic { exposure: 0.2, ..Default::default() }, masks: vec![ai_mask(LocalAdjust { exposure: 0.8, saturation: -0.3, temperature: 15.0, ..Default::default() })], ..Default::default() };
        let map = masks::FrameMap::identity(1500, 1000);
        let mut whole = frame.clone();
        develop_stages(&mut whole, &d, &Ctx { frame_map: Some(map), ai_rasters: Some(&store), ..Ctx::new(1.0, 9) });
        let (x, y, w, h) = (500u32, 400u32, 400u32, 200u32);
        let m = masks::refine_pad(&map);
        let (px, py) = (x - m, y - m);
        let (pw, ph) = (w + 2 * m, h + 2 * m);
        let mut part = geometry::crop(&frame, px, py, pw, ph);
        let ctx = Ctx { region: Some(Region { x: px, y: py, frame_width: 1500, frame_height: 1000 }), airlight: Some(frame_airlight(&frame, &d)), frame_map: Some(map), ai_rasters: Some(&store), ..Ctx::new(1.0, 9) };
        develop_stages(&mut part, &d, &ctx);
        let got = geometry::crop(&part, m, m, w, h).to_rgb8();
        let want = geometry::crop(&whole, x, y, w, h).to_rgb8();
        let worst = got.as_raw().iter().zip(want.as_raw()).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "region vs whole: {worst}/255");
    }

    #[test]
    fn an_ai_masked_preview_matches_its_export() {
        use photos_core::edits::{Crop, LocalAdjust};
        let full = ai_scene(3000, 2000);
        let store = ai_store();
        let mut recipe = with_masks(vec![ai_mask(LocalAdjust { exposure: 1.0, ..Default::default() })]);
        recipe.rotate = 90;
        recipe.flip_h = true;
        recipe.crop = Some(Crop { x: 0.05, y: 0.1, w: 0.9, h: 0.8 });
        let ctx = |scale| Ctx { ai_rasters: Some(&store), ..Ctx::new(scale, 7) };
        let exported = process(&full, &recipe, &ctx(1.0));
        let small = full.resize(1000, 667);
        let preview = process(&small, &recipe, &ctx(1.0 / 3.0)).to_rgb8();
        let exported = exported.resize(preview.width(), preview.height()).to_rgb8();
        let db = psnr(&preview, &exported);
        assert!(db >= 32.0, "preview vs export: {db:.1} dB");
        let plain = process(&small, &Recipe { develop: Some(Develop { masks: vec![], ..recipe.develop.clone().unwrap() }), ..recipe.clone() }, &ctx(1.0 / 3.0)).to_rgb8();
        assert!(psnr(&preview, &plain) < 25.0, "the mask shows");
    }

    #[test]
    fn no_develop_is_geometry_alone() {
        let img = scene(40, 30);
        assert_eq!(process(&img, &Recipe::identity(), &Ctx::default()), img);
        let flat = Recipe { develop: Some(Develop::default()), ..Recipe::identity() };
        assert_eq!(process(&img, &flat, &Ctx::default()), img);
    }
}
