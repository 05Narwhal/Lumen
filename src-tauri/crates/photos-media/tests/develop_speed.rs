//! Spec §3's budgets: a 2048 px preview with every stage on in ≤ 120 ms; a 24 MP export
//! printed (target ≤ 3 s without denoise). Phase 8e: with three masks (a 200-dab brush and two
//! gradients) on top, ≤ 150 ms and ≤ 1.5 s.
//!
//! `cargo test --release -p photos-media --test develop_speed -- --ignored --nocapture`

use std::time::{Duration, Instant};

use photos_core::ai_masks::{AiMaskKind, AiMaskRaster, AiRasters};
use photos_core::edits::{Basic, Curve, Detail, Develop, Effects, Glow, Grading, Grain, Hsl, LocalAdjust, Mask, MaskShape, Recipe, Sharpen, Vignette, Wheel};
use photos_media::develop::{process, Ctx, Linear};

fn everything() -> Recipe {
    let mut hsl = [Hsl::default(); 8];
    hsl[0] = Hsl { h: 0.2, s: 0.3, l: -0.1 };
    hsl[5] = Hsl { h: -0.1, s: -0.2, l: 0.1 };
    Recipe {
        straighten: 1.5,
        develop: Some(Develop {
            masks: Vec::new(),
            basic: Basic {
                temperature: 12.0,
                tint: -4.0,
                exposure: 0.4,
                contrast: 0.2,
                highlights: -0.5,
                shadows: 0.4,
                whites: 0.1,
                blacks: -0.1,
                texture: 0.3,
                clarity: 0.3,
                dehaze: 0.2,
                vibrance: 0.2,
                saturation: 0.05,
            },
            curve: Curve { master: vec![[0.25, 0.2], [0.75, 0.8]], ..Default::default() },
            hsl,
            grading: Grading { shadows: Wheel { hue: 220.0, sat: 0.3, lum: 0.0 }, highlights: Wheel { hue: 50.0, sat: 0.2, lum: 0.0 }, ..Default::default() },
            detail: Detail { sharpen: Sharpen { amount: 0.6, radius: 1.0, detail: 0.3, masking: 0.3 }, denoise: None },
            effects: Effects { vignette: Vignette { amount: -0.3, midpoint: 0.5, feather: 0.5 }, grain: Grain { amount: 0.2, size: 0.3 }, ..Default::default() },
            auto: false,
        }),
        ..Recipe::identity()
    }
}

/// `everything()` plus three masks: a brush stroke of about 200 dabs, a linear gradient over
/// the sky and a radial one, each with several local sliders.
fn with_three_masks() -> Recipe {
    let mut r = everything();
    // 200 points ~ a quarter radius apart (radius 0.03 of the long edge): ~200 dabs.
    let points = (0..200).map(|i| {
        let t = i as f32 / 199.0;
        [(400.0 + 3000.0 * t) as u16, (2400.0 + 600.0 * (t * 12.0).sin()) as u16]
    });
    let mask = |id, shapes, adjust| Mask { id, name: format!("Mask {id}"), shapes, invert: false, opacity: 1.0, adjust, visible: true };
    r.develop.as_mut().unwrap().masks = vec![
        mask(1, vec![MaskShape::Brush { add: true, radius: 0.03, feather: 0.5, flow: 0.8, points: points.collect() }], LocalAdjust { exposure: 0.4, clarity: 0.3, saturation: 0.2, ..Default::default() }),
        mask(2, vec![MaskShape::Linear { add: true, from: [0.5, 0.0], to: [0.5, 0.45] }], LocalAdjust { exposure: -0.5, highlights: -0.3, dehaze: 0.2, temperature: -10.0, ..Default::default() }),
        mask(3, vec![MaskShape::Radial { add: true, center: [0.6, 0.6], radius: [0.2, 0.25], angle: 15.0, feather: 0.5, inside: true }], LocalAdjust { exposure: 0.3, shadows: 0.3, sharpness: 0.3, ..Default::default() }),
    ];
    r
}

#[test]
#[ignore = "timing; run with --release"]
fn a_2048_preview_with_three_masks_renders_inside_the_budget() {
    let base = scene(2048, 1365);
    let r = with_three_masks();
    let ctx = Ctx::new(2048.0 / 6048.0, 1);
    process(&base, &r, &ctx);
    let t = Instant::now();
    for _ in 0..5 {
        let _ = process(&base, &r, &ctx).to_rgb8();
    }
    let per = t.elapsed() / 5;
    eprintln!("2048 px preview, every stage and three masks: {per:?}");
    assert!(per <= Duration::from_millis(150), "{per:?}");
}

#[test]
#[ignore = "timing; run with --release"]
fn a_24_mp_export_with_three_masks_is_inside_the_budget() {
    let base = scene(6048, 4024);
    let r = with_three_masks();
    let t = Instant::now();
    let _ = process(&base, &r, &Ctx::new(1.0, 1)).to_rgb8();
    let took = t.elapsed();
    eprintln!("24 MP export, every stage and three masks (decode not included): {took:?}");
    assert!(took <= Duration::from_millis(1500), "{took:?}");
}

/// The three masks with an AI one on top, and a raster of the size Facet stores.
fn with_an_ai_mask() -> (Recipe, AiRasters) {
    let mut r = with_three_masks();
    let shape = MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: "aa".into() };
    r.develop.as_mut().unwrap().masks.push(Mask { id: 4, name: "Sky".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: -0.4, highlights: -0.3, saturation: 0.2, ..Default::default() }, visible: true });
    let (rw, rh) = (160u32, 107u32);
    let weights = (0..rh).flat_map(|y| (0..rw).map(move |x| (x, y))).map(|(x, y)| if (y as f32) < 45.0 + 8.0 * (x as f32 / 9.0).sin() { 255 } else { 0 }).collect();
    let raster = AiMaskRaster { kind: AiMaskKind::Sky, instance: 0, width: rw, height: rh, weights };
    (r, AiRasters::from([("aa".to_string(), raster)]))
}

#[test]
#[ignore = "timing; run with --release"]
fn a_2048_preview_with_an_ai_mask_is_inside_the_budget() {
    let base = scene(2048, 1365);
    let (r, rasters) = with_an_ai_mask();
    let without = with_three_masks();
    let plain = Ctx::new(2048.0 / 6048.0, 1);
    let ctx = Ctx { ai_rasters: Some(&rasters), ..plain };
    process(&base, &r, &ctx);
    let time = |recipe: &Recipe, ctx: &Ctx| {
        let t = Instant::now();
        for _ in 0..5 {
            let _ = process(&base, recipe, ctx).to_rgb8();
        }
        t.elapsed() / 5
    };
    let (three, four) = (time(&without, &plain), time(&r, &ctx));
    eprintln!("2048 px preview: three masks {three:?}, with an AI one on top {four:?} (provisional)");
    assert!(four <= three + Duration::from_millis(40), "{three:?} → {four:?}");
}

#[test]
#[ignore = "timing; run with --release"]
fn a_24_mp_export_with_an_ai_mask_is_measured_and_matches_its_preview() {
    let base = scene(6048, 4024);
    let (r, rasters) = with_an_ai_mask();
    let ctx = Ctx { ai_rasters: Some(&rasters), ..Ctx::new(1.0, 1) };
    let t = Instant::now();
    let out = process(&base, &r, &ctx);
    let took = t.elapsed();
    let without = with_three_masks();
    let t = Instant::now();
    let _ = process(&base, &without, &Ctx::new(1.0, 1));
    eprintln!("24 MP export: three masks {:?}, with an AI one on top {took:?} (provisional)", t.elapsed());
    let small = base.resize(2048, 1363);
    let preview = process(&small, &r, &Ctx { ai_rasters: Some(&rasters), ..Ctx::new(2048.0 / 6048.0, 1) }).to_rgb8();
    let export = out.resize(2048, 1363).to_rgb8();
    let mse = preview.as_raw().iter().zip(export.as_raw()).map(|(a, b)| (*a as f64 - *b as f64).powi(2)).sum::<f64>() / preview.as_raw().len() as f64;
    let db = 10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10();
    eprintln!("2048 px preview vs 24 MP export: {db:.1} dB");
    assert!(db >= 32.0, "{db:.1} dB");
}

fn scene(w: u32, h: u32) -> Linear {
    let mut img = Linear::new(w, h, [0.0; 3]);
    for (i, p) in img.data.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
        let l = 0.18 * 2f32.powf(-2.0 + 3.0 * x + (y * 40.0).sin() * 0.3);
        *p = [l * 1.1, l, l * (0.8 + 0.4 * y)];
    }
    img
}

#[test]
#[ignore = "timing; run with --release"]
fn a_2048_preview_renders_inside_the_budget() {
    let base = scene(2048, 1365);
    let r = everything();
    let ctx = Ctx::new(2048.0 / 6048.0, 1);
    process(&base, &r, &ctx); // warm the tables
    let t = Instant::now();
    for _ in 0..5 {
        let _ = process(&base, &r, &ctx).to_rgb8();
    }
    let per = t.elapsed() / 5;
    eprintln!("2048 px preview, every stage: {per:?}");
    assert!(per <= Duration::from_millis(120), "{per:?}");
}

#[test]
#[ignore = "timing; run with --release"]
fn a_24_mp_export_is_measured() {
    let base = scene(6048, 4024);
    let t = Instant::now();
    let _ = process(&base, &everything(), &Ctx::new(1.0, 1)).to_rgb8();
    eprintln!("24 MP export, every stage (decode not included): {:?}", t.elapsed());
}

#[test]
#[ignore = "profiling: where the masks' time goes"]
fn mask_times() {
    use photos_media::develop::masks::{self, FrameMap};
    let base = scene(2048, 1365);
    let r = with_three_masks().normalized();
    let d = r.develop.clone().unwrap();
    let map = FrameMap::new(2048, 1365, &r);
    let img = photos_media::develop::geometry::apply(&base, &r);
    for m in &d.masks {
        let s = Instant::now();
        let _ = masks::rasterize(m, &map, img.width, img.height);
        eprintln!("rasterize {}: {:?}", m.name, s.elapsed());
        let mut i = img.clone();
        let s = Instant::now();
        let _ = masks::apply(&mut i, std::slice::from_ref(m), &Ctx { frame_map: Some(map), ..Ctx::new(2048.0 / 6048.0, 1) }, None);
        eprintln!("    apply {}: {:?}", m.name, s.elapsed());
    }
}

#[test]
#[ignore = "profiling: where a preview's time goes, stage by stage"]
fn stage_times() {
    use photos_media::develop::*;
    let base = scene(2048, 1365);
    let r = everything().normalized();
    let d = r.develop.clone().unwrap();
    let ctx = Ctx::new(2048.0 / 6048.0, 1);
    let t = |name: &str, s: Instant| eprintln!("{name:>10}: {:?}", s.elapsed());
    let s = Instant::now(); let mut img = geometry::apply(&base, &r); t("geometry", s);
    let s = Instant::now(); basic::apply_gains(&mut img, &d.basic); t("gains", s);
    let s = Instant::now(); local::texture_clarity(&mut img, d.basic.texture, d.basic.clarity, &ctx); t("tex/clar", s);
    let s = Instant::now(); local::dehaze(&mut img, d.basic.dehaze, &ctx); t("dehaze", s);
    let s = Instant::now(); basic::apply_tones(&mut img, &d.basic); t("tones", s);
    let s = Instant::now(); curve::apply(&mut img, &d.curve); t("curve", s);
    let s = Instant::now(); color::apply(&mut img, &d.hsl, &d.grading); t("color", s);
    let s = Instant::now(); detail::sharpen(&mut img, &d.detail.sharpen, &ctx); t("sharpen", s);
    let s = Instant::now(); effects::vignette(&mut img, &d.effects.vignette, &ctx); t("vignette", s);
    let s = Instant::now(); effects::grain(&mut img, &d.effects.grain, &ctx); t("grain", s);
    let s = Instant::now(); let _ = img.to_rgb8(); t("encode", s);
}

/// Round 19 §1 (b): one mask with every per-pixel tool adds ≤ 30 % to the 2048 px preview.
/// Printed next to the no-mask baseline, a mask with only local Basic and one with glow.
#[test]
#[ignore = "timing; run with --release"]
fn a_2048_preview_with_one_mask_using_every_local_tool() {
    let base = scene(2048, 1365);
    let ctx = Ctx::new(2048.0 / 6048.0, 1);
    let with = |adjust: LocalAdjust| {
        let mut r = everything();
        let shape = MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.35, 0.4], angle: 0.0, feather: 0.5, inside: true };
        r.develop.as_mut().unwrap().masks = vec![Mask { id: 1, name: "Mask".into(), shapes: vec![shape], invert: false, opacity: 1.0, adjust, visible: true }];
        r
    };
    let basic = LocalAdjust { exposure: 0.4, contrast: 0.2, shadows: 0.3, saturation: 0.1, ..Default::default() };
    let mut hsl = [Hsl::default(); 8];
    hsl[2] = Hsl { h: 0.2, s: -0.3, l: 0.1 };
    hsl[6] = Hsl { h: -0.1, s: 0.4, l: -0.2 };
    let per_pixel = LocalAdjust {
        vibrance: 0.3,
        curve: Curve { master: vec![[0.25, 0.2], [0.75, 0.85]], r: vec![[0.5, 0.55]], ..Default::default() },
        hsl,
        grading: Grading { shadows: Wheel { hue: 200.0, sat: 0.3, lum: 0.0 }, highlights: Wheel { hue: 40.0, sat: 0.2, lum: 0.0 }, ..Default::default() },
        ..basic.clone()
    };
    let glow = LocalAdjust { glow: Glow { amount: 0.5, size: 0.5 }, ..basic.clone() };
    // Interleaved, the fastest of nine rounds each: the least disturbed by whatever else the
    // machine runs, and a busy spell slows every case alike.
    let recipes = [everything(), with(basic), with(per_pixel), with(glow)];
    recipes.iter().for_each(|r| drop(process(&base, r, &ctx)));
    let mut best = [Duration::MAX; 4];
    for _ in 0..9 {
        for (r, b) in recipes.iter().zip(best.iter_mut()) {
            let t = Instant::now();
            let _ = process(&base, r, &ctx).to_rgb8();
            *b = (*b).min(t.elapsed());
        }
    }
    let [none, b, p, g] = best;
    let pct = |d: Duration| 100.0 * (d.as_secs_f64() / none.as_secs_f64() - 1.0);
    eprintln!("2048 px preview: no mask {none:?}; local Basic {b:?} (+{:.0} %); every per-pixel tool {p:?} (+{:.0} %); glow {g:?} (+{:.0} %)", pct(b), pct(p), pct(g));
    assert!(p.as_secs_f64() <= none.as_secs_f64() * 1.3, "{p:?} vs {none:?}");
}
