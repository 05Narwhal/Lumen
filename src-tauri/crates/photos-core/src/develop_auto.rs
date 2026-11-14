//! Develop's *Auto* (Phase 8d, develop spec §4): measure the photo, bend the result by what
//! CLIP reads the scene as, then search the Basic sliders for the best aesthetic score.
//! Everything here is pure; the binary supplies the renders and the model.

use crate::edits::{wb_from_grey, Basic};

/// Middle grey in linear light: the tone sliders pivot here.
pub const MID: f32 = 0.18;
/// Where Auto puts the median (Round 5): 0.22 linear shows as about half brightness. Middle
/// grey itself (0.18) looked dark on screen.
pub const TARGET: f32 = 0.22;
/// How far exposure may move either way.
pub const MAX_EV: f32 = 3.0;
/// At most this many pixels are sorted; a larger frame is sampled evenly.
const MAX_SAMPLES: usize = 65_536;
/// Pixels more colourful than this (`(max − min) / max`) do not vote on white balance: a red
/// sign is excluded, a strong tungsten or shade cast (≈ 0.4) is what grey world must see.
const MAX_GREY_CHROMA: f32 = 0.5;

/// What Auto's first step reads off the photo: luminance percentiles and tails, and the mean
/// colour of its near-grey pixels (for white balance).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measure {
    /// NaN for an empty frame.
    pub median: f32,
    /// The 0.1th and 99.9th percentiles: whites and blacks aim to clip no more than these.
    pub p_lo: f32,
    pub p_hi: f32,
    /// Luminance's lower quartile and 95th percentile: what Shadows lifts and Highlights
    /// holds (Round 5).
    pub p_25: f32,
    pub p_95: f32,
    /// The 95th percentile of each pixel's brightest channel — what clips first, a tungsten
    /// red say: what holds exposure back.
    pub peak_95: f32,
    /// Shares below 0.01 (deep shadow) and above 0.9 (near white).
    pub frac_dark: f32,
    pub frac_bright: f32,
    /// The interquartile range in stops: how much contrast the photo already has.
    pub iqr_stops: f32,
    /// Grey world over pixels that are neither clipped, nearly black nor saturated; `None`
    /// when under 2 % of the frame qualifies.
    pub grey: Option<[f32; 3]>,
}

/// What CLIP reads the scene as (weights 0–1), which bends the prior (spec §4 step 2).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Scene {
    pub sunset: f32,
    pub night: f32,
    pub snow: f32,
    pub beach: f32,
    pub backlit: f32,
    pub portrait: f32,
    pub foggy: f32,
    pub indoor: f32,
}

/// The softmax temperature over `scene:*`: CLIP's own trained logit scale (100). At 0.05 (the
/// mood softmax's) nine phrases split a real photo almost evenly — top shares 0.15–0.28 on 20
/// of Julian's photos — and no scene was ever read.
pub const SCENE_TEMPERATURE: f32 = 0.01;
/// Grey world is applied at this strength and within these limits: a frame full of foliage
/// or sky is not a grey world, and at full strength it pushed tint to +77.
const WB_STRENGTH: f32 = 0.5;
const WB_MAX_TEMPERATURE: f32 = 60.0;
const WB_MAX_TINT: f32 = 25.0;

/// What a photo's embedding reads as (spec §4 step 2): the softmax over `scene:*`, each share
/// counted only above 0.25 and rescaled so a confident reading approaches 1. With no scene
/// prototypes it is `Scene::default()`, the plain measured prior.
pub fn scene_from(v: &[i8], protos: &crate::embedding::Prototypes) -> Scene {
    let mut s = Scene::default();
    for (name, share) in crate::embedding::softmax(v, protos, "scene:", SCENE_TEMPERATURE) {
        let w = ((share - 0.25) / 0.5).clamp(0.0, 1.0);
        match name.as_str() {
            "sunset" => s.sunset = w,
            "night" => s.night = w,
            "snow" => s.snow = w,
            "beach" => s.beach = w,
            "backlit" => s.backlit = w,
            "portrait" => s.portrait = w,
            "foggy" => s.foggy = w,
            "indoor" => s.indoor = w,
            _ => {} // daylight: an ordinary photo, no bend
        }
    }
    s
}

impl Scene {
    /// The strongest reading over 0.5, for the toast ("Auto — read as a sunset").
    pub fn strongest(&self) -> Option<&'static str> {
        let all = [
            ("sunset", self.sunset),
            ("night", self.night),
            ("snow", self.snow),
            ("beach", self.beach),
            ("backlit", self.backlit),
            ("portrait", self.portrait),
            ("foggy", self.foggy),
            ("indoor", self.indoor),
        ];
        all.into_iter().filter(|(_, w)| *w > 0.5).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(n, _)| n)
    }
}

/// What the judge says of one candidate: CLIP's aesthetic score (0–1) and the share of pixels
/// clipped in linear light (any channel ≥ 1, or all ≤ 0.002).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eval {
    pub aesthetic: f32,
    pub clipping: f32,
    /// The median of the render's display luminance (sRGB-encoded, 0–1), Round 5.
    pub median: f32,
}

/// Rounds of 16 candidates after the prior: 1 + 3 × 16 = 49 evaluations (spec: "≈ 50").
pub const ROUNDS: u32 = 3;
const PER_ROUND: usize = 16;
const FIELDS_MOVED: usize = 3;
/// How far a candidate may stray from the prior before it costs more than it gains.
pub const LAMBDA: f32 = 0.02;
/// What a clipped share costs.
pub const MU: f32 = 2.0;
/// What a candidate pays for each unit its median falls below the prior's, past `DARK_SLACK`
/// (Round 5): the judge's taste ran dark.
pub const DARKEN: f32 = 2.0;
const DARK_SLACK: f32 = 0.03;
/// A candidate clipping more than the prior by this much is not considered.
const CLIP_SLACK: f32 = 0.001;

/// The fields Auto searches and their first-round steps. Texture, clarity and dehaze are
/// left as the centre has them.
const STEPS: [f32; 10] = [0.5, 0.25, 0.25, 0.25, 0.25, 0.25, 12.0, 12.0, 0.15, 0.15];

fn get(b: &Basic) -> [f32; 10] {
    [b.exposure, b.contrast, b.highlights, b.shadows, b.whites, b.blacks, b.temperature, b.tint, b.vibrance, b.saturation]
}

fn with(b: &Basic, v: [f32; 10]) -> Basic {
    Basic {
        exposure: v[0],
        contrast: v[1],
        highlights: v[2],
        shadows: v[3],
        whites: v[4],
        blacks: v[5],
        temperature: v[6],
        tint: v[7],
        vibrance: v[8],
        saturation: v[9],
        ..*b
    }
}

/// SplitMix64: a small, fixed stream, so the same photo always gets the same candidates.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Sixteen perturbations of `centre`: each moves three fields, chosen by a stream seeded by
/// `seed ^ round`, by ± that field's step halved each round, clamped to the sliders' ranges.
pub fn candidates(centre: &Basic, round: u32, seed: u64) -> Vec<Basic> {
    let mut rng = SplitMix(seed ^ u64::from(round));
    let scale = 0.5f32.powi(round as i32);
    let base = get(centre);
    (0..PER_ROUND)
        .map(|_| {
            let mut v = base;
            let mut moved = [usize::MAX; FIELDS_MOVED];
            for slot in 0..FIELDS_MOVED {
                let mut f = (rng.next() % STEPS.len() as u64) as usize;
                while moved.contains(&f) {
                    f = (f + 1) % STEPS.len();
                }
                moved[slot] = f;
                let sign = if rng.next() & 1 == 0 { 1.0 } else { -1.0 };
                v[f] += sign * STEPS[f] * scale;
            }
            crate::edits::Develop { basic: with(centre, v), ..Default::default() }.normalized().basic
        })
        .collect()
}

/// How far `b` is from the prior, each field in units of its first step.
fn distance(b: &Basic, prior: &Basic) -> f32 {
    get(b).iter().zip(get(prior)).zip(STEPS).map(|((x, p), s)| ((x - p) / s).powi(2)).sum::<f32>().sqrt()
}

/// A candidate's worth: `aesthetic − λ·distance(prior) − μ·clipping` (spec §4 step 3), less
/// `DARKEN` for each unit it shows darker than the prior did (`prior_median`), Round 5.
pub fn score(e: &Eval, b: &Basic, prior: &Basic, prior_median: f32) -> f32 {
    let darker = (prior_median - e.median - DARK_SLACK).max(0.0);
    e.aesthetic - LAMBDA * distance(b, prior) - MU * e.clipping - DARKEN * darker
}

/// Auto's search (spec §4 step 3): the prior, then `rounds` rounds of 16 candidates around
/// the best so far, one `evaluate` call per round so the caller can batch. Never returns
/// anything that clips more than the prior; ties go to the earlier. Deterministic in `seed`.
pub fn search<E>(prior: Basic, seed: u64, rounds: u32, mut evaluate: impl FnMut(&[Basic]) -> Result<Vec<Eval>, E>) -> Result<Basic, E> {
    let first = evaluate(&[prior])?;
    let Some(e0) = first.first() else { return Ok(prior) };
    let limit = e0.clipping + CLIP_SLACK;
    let (mut best, mut best_score) = (prior, score(e0, &prior, &prior, e0.median));
    for round in 0..rounds {
        let cands = candidates(&best, round, seed);
        let evals = evaluate(&cands)?;
        let mut next = (best, best_score);
        for (b, e) in cands.iter().zip(&evals) {
            if e.clipping > limit {
                continue;
            }
            let s = score(e, b, &prior, e0.median);
            if s > next.1 {
                next = (*b, s);
            }
        }
        (best, best_score) = next;
    }
    Ok(best)
}

fn clean(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

/// Measures linear sRGB pixels (a ≤ 256 px render of the cropped, undeveloped photo).
pub fn measure(pixels: &[[f32; 3]]) -> Measure {
    let step = pixels.len().div_ceil(MAX_SAMPLES).max(1);
    let sample: Vec<[f32; 3]> = pixels.iter().step_by(step).map(|p| [clean(p[0]), clean(p[1]), clean(p[2])]).collect();
    let n = sample.len();
    if n == 0 {
        return Measure { median: f32::NAN, p_lo: f32::NAN, p_hi: f32::NAN, p_25: f32::NAN, p_95: f32::NAN, peak_95: f32::NAN, frac_dark: 0.0, frac_bright: 0.0, iqr_stops: 0.0, grey: None };
    }
    let mut lum: Vec<f32> = sample.iter().map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]).collect();
    lum.sort_by(f32::total_cmp);
    let at = |q: f32| lum[((q * (n - 1) as f32).round() as usize).min(n - 1)];
    let mut peak: Vec<f32> = sample.iter().map(|p| p[0].max(p[1]).max(p[2])).collect();
    peak.sort_by(f32::total_cmp);
    let (mut sum, mut count) = ([0f32; 3], 0usize);
    for (p, l) in sample.iter().zip(sample.iter().map(|p| 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2])) {
        let (hi, lo) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
        if (0.05..=0.8).contains(&l) && hi < 0.98 && (hi - lo) / hi < MAX_GREY_CHROMA {
            for c in 0..3 {
                sum[c] += p[c];
            }
            count += 1;
        }
    }
    let grey = (count * 50 >= n).then(|| sum.map(|s| s / count as f32));
    Measure {
        median: at(0.5),
        p_lo: at(0.001),
        p_hi: at(0.999),
        p_25: at(0.25),
        p_95: at(0.95),
        peak_95: peak[((0.95 * (n - 1) as f32).round() as usize).min(n - 1)],
        frac_dark: lum.iter().filter(|&&l| l < 0.01).count() as f32 / n as f32,
        frac_bright: lum.iter().filter(|&&l| l > 0.9).count() as f32 / n as f32,
        iqr_stops: (at(0.75).max(1e-4) / at(0.25).max(1e-4)).log2(),
        grey,
    }
}

/// Auto's first step (spec §4): exposure toward a mid-grey median, highlights and shadows
/// from where the quartiles land, whites and blacks for little clipping, grey-world white
/// balance — bent by the scene (a sunset stays warm, a night stays dark, snow may be bright).
///
/// Round 5 opened it up (it was dark, contrasty and timid): a brighter target, exposure held
/// back where it would blow what Highlights cannot recover (a backlit sky) with Shadows doing
/// the rest, whites and blacks gentle and two-sided, contrast only for a flat photo, and a
/// bright photo darkened only half way. The tests fix directions and bounds; the scene table
/// in the binary's `develop_auto` tests fixes what it looks like.
pub fn prior(m: &Measure, s: &Scene) -> Basic {
    if m.median.is_nan() {
        return Basic::default();
    }
    let lg = |v: f32| v.max(1e-5).log2();
    // Where the median should land: darker for night, brighter for snow and beaches.
    let target = TARGET * 2f32.powf(-1.5 * s.night + 0.6 * s.snow + 0.3 * s.beach);
    let mut exposure = lg(target / m.median.max(1e-4));
    // A bright photo is often meant bright (mist, snow, a white wall): darken half way.
    if exposure < 0.0 {
        exposure *= 0.5;
    }
    // Do not push the bright twentieth past white: Highlights and Whites cannot bring back
    // what is far over (a backlit sky). Shadows does the rest of the lifting.
    exposure = exposure.min(lg(1.1 / m.peak_95.max(1e-4)).max(0.0));
    let exposure = exposure.clamp(-MAX_EV, MAX_EV) * (1.0 - 0.6 * s.night);
    let k = 2f32.powf(exposure);
    let (hi, lo, q1, q95) = (m.p_hi * k, m.p_lo * k, m.p_25 * k, m.p_95 * k);
    // Whites: a little down when the brightest thousandth clips, up when nothing reaches white.
    let whites = if hi > 1.0 { -(0.4 * lg(hi)).clamp(0.0, 0.5) } else { lg(0.97 / hi.max(1e-4)).clamp(0.0, 0.8) };
    // Highlights hold what the bright twentieth would lose.
    let highlights = -(0.6 * lg(q95 / 0.7)).clamp(0.0, 0.9) - 0.3 * s.backlit;
    // Shadows open the lower quartile up toward 0.06 (about a quarter brightness on screen),
    // and take over whatever exposure could not do (held back by the highlights or the clamp).
    let short = lg(target / (m.median.max(1e-5) * k)).max(0.0);
    let shadows = ((0.35 * lg(0.06 / q1.max(1e-5))).max(0.0) + 0.6 * short + 0.35 * s.backlit).clamp(0.0, 1.0) * (1.0 - s.night);
    // Blacks deepen only a washed-out photo, and lift only a crushed one.
    let blacks = if lo > 0.008 { -(0.25 * lg(lo / 0.008)).clamp(0.0, 0.3) } else { (-0.1 * lg(lo.max(1e-6) / 0.0015)).clamp(0.0, 0.2) };
    // Contrast only for a flat photo, and less for one that already has plenty.
    let contrast = ((1.8 - m.iqr_stops) / 3.0).clamp(-0.2, 0.35) + 0.15 * s.foggy;
    let (t, tint) = m
        .grey
        .map(wb_from_grey)
        .map(|(t, m)| ((t * WB_STRENGTH).clamp(-WB_MAX_TEMPERATURE, WB_MAX_TEMPERATURE), (m * WB_STRENGTH).clamp(-WB_MAX_TINT, WB_MAX_TINT)))
        .unwrap_or((0.0, 0.0));
    // Keep what the light was: a sunset warm, candlelight warm.
    let keep = (s.sunset + 0.6 * s.indoor).clamp(0.0, 1.0);
    let mut temperature = t * (1.0 - keep);
    if s.sunset > 0.0 {
        temperature = temperature.max(t) + 15.0 * s.sunset;
    }
    let vibrance = (0.15 + 0.1 * s.sunset + 0.1 * s.beach - 0.1 * s.portrait).clamp(0.0, 0.35);
    let dehaze = 0.2 * s.foggy;
    Basic {
        temperature: temperature.clamp(-100.0, 100.0),
        tint: tint * (1.0 - keep),
        exposure,
        contrast,
        highlights,
        shadows,
        whites,
        blacks,
        vibrance,
        dehaze,
        ..Basic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edits::{wb_from_grey, wb_gains, Basic};

    fn flat(v: [f32; 3], n: usize) -> Vec<[f32; 3]> {
        vec![v; n]
    }
    fn ramp(n: usize, lo: f32, hi: f32) -> Vec<[f32; 3]> {
        (0..n)
            .map(|i| {
                let t = lo * (hi / lo).powf(i as f32 / (n - 1) as f32);
                [t, t, t]
            })
            .collect()
    }

    fn unit(i: usize) -> Vec<f32> {
        let mut v = vec![0f32; crate::embedding::DIM];
        v[i] = 1.0;
        v
    }

    #[test]
    fn a_photo_nearest_a_scene_phrase_reads_as_that_scene() {
        use crate::embedding::{quantize, Prototypes};
        let protos = Prototypes {
            model: "m".into(),
            items: vec![("scene:sunset".into(), quantize(&unit(0))), ("scene:night".into(), quantize(&unit(1))), ("scene:daylight".into(), quantize(&unit(2)))],
        };
        let s = scene_from(&quantize(&unit(0)), &protos);
        assert!(s.sunset > 0.8 && s.night < 0.1, "{s:?}");
        assert_eq!(scene_from(&quantize(&unit(0)), &Prototypes::empty()), Scene::default());
    }

    /// A fake judge that likes exposure 0.7 and contrast 0.2, and clips above exposure 1.2.
    fn judge(bs: &[Basic]) -> Result<Vec<Eval>, ()> {
        Ok(bs
            .iter()
            .map(|b| Eval { aesthetic: 0.8 - (b.exposure - 0.7).powi(2) - (b.contrast - 0.2).powi(2), clipping: if b.exposure > 1.2 { 0.05 } else { 0.0 }, median: 0.5 })
            .collect())
    }

    #[test]
    fn the_search_is_deterministic_and_improves_on_the_prior() {
        let prior = Basic { exposure: 0.2, ..Default::default() };
        let a = search(prior, 7, ROUNDS, judge).unwrap();
        assert_eq!(a, search(prior, 7, ROUNDS, judge).unwrap());
        let s = |b: &Basic| score(&judge(&[*b]).unwrap()[0], b, &prior, 0.5);
        assert!(s(&a) > s(&prior));
        assert!((a.exposure - 0.7).abs() < (prior.exposure - 0.7).abs());
    }

    /// Round 5: the aesthetic judge liked dark, moody versions, and the search followed it.
    /// A candidate darker than the prior on screen now pays for it.
    #[test]
    fn the_search_does_not_follow_a_judge_into_the_dark() {
        let prior = Basic { exposure: 0.5, ..Default::default() };
        let moody = |bs: &[Basic]| -> Result<Vec<Eval>, ()> {
            Ok(bs.iter().map(|b| Eval { aesthetic: 0.5 - 0.03 * b.exposure, clipping: 0.0, median: 0.45 + 0.08 * b.exposure }).collect())
        };
        let a = search(prior, 5, ROUNDS, moody).unwrap();
        assert!(a.exposure >= prior.exposure - 0.4, "{a:?}");
    }

    #[test]
    fn the_search_never_clips_more_than_the_prior() {
        let prior = Basic { exposure: 1.1, ..Default::default() };
        let greedy = |bs: &[Basic]| -> Result<Vec<Eval>, ()> {
            Ok(bs.iter().map(|b| Eval { aesthetic: b.exposure, clipping: if b.exposure > 1.15 { 0.2 } else { 0.0 }, median: 0.5 }).collect())
        };
        assert!(search(prior, 1, ROUNDS, greedy).unwrap().exposure <= 1.15);
    }

    #[test]
    fn it_evaluates_about_fifty_candidates_in_batches() {
        let mut calls = vec![];
        search(Basic::default(), 3, ROUNDS, |bs| {
            calls.push(bs.len());
            judge(bs)
        })
        .unwrap();
        assert_eq!(calls, vec![1, 16, 16, 16]);
    }

    #[test]
    fn candidates_keep_local_contrast_and_stay_in_range() {
        let c = Basic { texture: 0.3, clarity: -0.2, dehaze: 0.1, exposure: 4.9, ..Default::default() };
        for b in candidates(&c, 0, 11) {
            assert_eq!((b.texture, b.clarity, b.dehaze), (0.3, -0.2, 0.1));
            assert!(b.exposure <= 5.0);
        }
        assert_ne!(candidates(&c, 0, 11), candidates(&c, 0, 12));
    }

    #[test]
    fn an_evaluator_error_stops_the_search() {
        assert!(search(Basic::default(), 1, ROUNDS, |_| Err::<Vec<Eval>, _>("model gone")).is_err());
    }

    #[test]
    fn wb_from_grey_inverts_wb_gains() {
        for (t, m) in [(0.0, 0.0), (40.0, -20.0), (-70.0, 35.0)] {
            let g = wb_gains(t, m);
            let cast = [0.2 / g[0], 0.2 / g[1], 0.2 / g[2]];
            let (t2, m2) = wb_from_grey(cast);
            assert!((t2 - t).abs() < 0.5 && (m2 - m).abs() < 0.5, "{t},{m} → {t2},{m2}");
        }
    }

    #[test]
    fn a_dark_photo_is_brightened_toward_middle_grey_and_bounded() {
        let p = prior(&measure(&ramp(10_000, 0.001, 0.05)), &Scene::default());
        assert!(p.exposure > 1.0 && p.exposure <= MAX_EV, "{}", p.exposure);
    }

    #[test]
    fn black_and_white_frames_give_finite_bounded_values() {
        for v in [0.0, 1e-9, 1.0, 50.0] {
            let p = prior(&measure(&flat([v, v, v], 4096)), &Scene::default());
            for x in [p.exposure, p.contrast, p.highlights, p.shadows, p.whites, p.blacks, p.temperature, p.tint, p.vibrance] {
                assert!(x.is_finite(), "{v}: {p:?}");
            }
            assert!(p.exposure.abs() <= MAX_EV);
        }
        let empty = prior(&measure(&[]), &Scene::default());
        assert_eq!(empty, Basic::default());
    }

    #[test]
    fn a_grey_photo_keeps_neutral_white_balance() {
        let p = prior(&measure(&ramp(10_000, 0.01, 0.8)), &Scene::default());
        assert!(p.temperature.abs() < 0.5 && p.tint.abs() < 0.5, "{p:?}");
    }

    #[test]
    fn a_blue_cast_is_warmed() {
        let px: Vec<_> = ramp(10_000, 0.02, 0.6).into_iter().map(|[v, _, _]| [v * 0.8, v, v * 1.25]).collect();
        assert!(prior(&measure(&px), &Scene::default()).temperature > 10.0);
    }

    #[test]
    fn clipped_and_saturated_pixels_do_not_vote_on_white_balance() {
        let mut px = ramp(9_000, 0.02, 0.6);
        px.extend(flat([1.0, 0.1, 0.05], 1_000)); // a red sign: saturated, excluded
        px.extend(flat([1.0, 1.0, 1.0], 500)); // clipped sky, excluded
        let p = prior(&measure(&px), &Scene::default());
        assert!(p.temperature.abs() < 2.0 && p.tint.abs() < 2.0, "{p:?}");
    }

    #[test]
    fn clipped_highlights_pull_whites_down() {
        let mut px = ramp(9_000, 0.02, 0.5);
        px.extend(flat([1.3, 1.3, 1.3], 1_000));
        let p = prior(&measure(&px), &Scene::default());
        assert!(p.whites < 0.0 && p.highlights < 0.0, "{p:?}");
    }

    #[test]
    fn scenes_bend_the_prior() {
        // A narrow range, so neither the ±EV clamp nor the highlight cap binds any scene.
        let m = measure(&ramp(10_000, 0.04, 0.12).into_iter().map(|[v, _, _]| [v * 1.3, v, v * 0.7]).collect::<Vec<_>>());
        let neutral = prior(&m, &Scene::default());
        let sunset = prior(&m, &Scene { sunset: 1.0, ..Default::default() });
        let night = prior(&m, &Scene { night: 1.0, ..Default::default() });
        let snow = prior(&m, &Scene { snow: 1.0, ..Default::default() });
        assert!(sunset.temperature > neutral.temperature + 10.0, "a sunset stays warm");
        assert!(night.exposure < neutral.exposure - 0.5, "a night stays dark");
        assert!(snow.exposure > neutral.exposure, "snow may be bright");
    }
}
