//! Masks (recipe v3, develop spec §3 *mask stage*): weights from brush strokes and linear and
//! radial gradients, drawn in the unedited photo's coordinates and rasterised at the render's
//! resolution through the render's own geometry, then each mask's local adjustments mixed in.
//! An AI shape (Phase 8f) is a stored raster sampled the same way and refined against the frame.

use photos_core::ai_masks::{AiMaskRaster, AiRasters};
use photos_core::edits::{LocalAdjust, Mask, MaskShape, Recipe, MASK_GRID};
use rayon::prelude::*;

use super::local::{self, CLARITY_SIGMA};
use super::{basic, geometry, local_tools, Ctx, Linear, Region};

/// Maps the unedited, display-oriented photo (0–1, or its pixels) into a rendered frame's
/// pixels: the render's flips, quarter turns, straighten and crop, and for a part of the
/// frame its offset. Every step is a similarity, so a disc stays a disc and an ellipse an
/// ellipse; `m` is the affine map `x' = m0·x + m1·y + m2, y' = m3·x + m4·y + m5` on the
/// unedited photo's pixels (continuous: pixel `i` spans `[i, i + 1]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameMap {
    pub m: [f32; 6],
    /// The unedited photo's size in its own pixels.
    pub base: [f32; 2],
}

impl FrameMap {
    pub fn identity(w: u32, h: u32) -> Self {
        Self { m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], base: [w as f32, h as f32] }
    }

    /// The geometry `geometry::apply` gives a `w × h` base, step by step in its order.
    pub fn new(w: u32, h: u32, r: &Recipe) -> Self {
        let mut f = Self::identity(w, h);
        let (mut w, mut h) = (w, h);
        let wf = |v: u32| v as f32;
        if r.flip_h {
            f = f.then([-1.0, 0.0, wf(w), 0.0, 1.0, 0.0]);
        }
        if r.flip_v {
            f = f.then([1.0, 0.0, 0.0, 0.0, -1.0, wf(h)]);
        }
        match r.rotate % 360 {
            90 => {
                f = f.then([0.0, -1.0, wf(h), 1.0, 0.0, 0.0]);
                (w, h) = (h, w);
            }
            180 => f = f.then([-1.0, 0.0, wf(w), 0.0, -1.0, wf(h)]),
            270 => {
                f = f.then([0.0, 1.0, 0.0, -1.0, 0.0, wf(w)]);
                (w, h) = (h, w);
            }
            _ => {}
        }
        if r.straighten != 0.0 {
            // The inverse of `geometry::straighten_source`: O = C + z·Rᵀ(S − C).
            let (s, c) = r.straighten.to_radians().sin_cos();
            let ratio = (wf(w) / wf(h)).max(wf(h) / wf(w));
            let z = c.abs() + s.abs() * ratio;
            let (cx, cy) = (wf(w) / 2.0, wf(h) / 2.0);
            let (a, b, cc, d) = (z * c, -z * s, z * s, z * c);
            f = f.then([a, b, cx - a * cx - b * cy, cc, d, cy - cc * cx - d * cy]);
        }
        if let Some(k) = r.crop {
            let x0 = (k.x * wf(w)).round();
            let y0 = (k.y * wf(h)).round();
            f = f.offset(-x0, -y0);
        }
        f
    }

    /// `self`, then `a`.
    fn then(self, a: [f32; 6]) -> Self {
        let m = self.m;
        Self {
            m: [
                a[0] * m[0] + a[1] * m[3],
                a[0] * m[1] + a[1] * m[4],
                a[0] * m[2] + a[1] * m[5] + a[2],
                a[3] * m[0] + a[4] * m[3],
                a[3] * m[1] + a[4] * m[4],
                a[3] * m[2] + a[4] * m[5] + a[5],
            ],
            base: self.base,
        }
    }

    /// Moved by `(dx, dy)` pixels: for a part of the frame, minus the part's corner.
    pub fn offset(self, dx: f32, dy: f32) -> Self {
        self.then([1.0, 0.0, dx, 0.0, 1.0, dy])
    }

    /// For the same frame rendered `k` times as large.
    pub fn scaled(self, k: f32) -> Self {
        self.then([k, 0.0, 0.0, 0.0, k, 0.0])
    }

    /// A point of the unedited photo (0–1) in the frame's pixels.
    pub fn to_frame(&self, u: f32, v: f32) -> [f32; 2] {
        let (x, y) = (u * self.base[0], v * self.base[1]);
        let m = self.m;
        [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]]
    }

    /// A point of the frame (its pixels) in the unedited photo, 0–1: `to_frame` undone.
    pub fn from_frame(&self, x: f32, y: f32) -> [f32; 2] {
        let [dx, dy] = self.vector_back(x - self.m[2], y - self.m[5]);
        [dx / self.base[0], dy / self.base[1]]
    }

    /// A displacement in the frame's pixels, in the unedited photo's pixels.
    pub fn vector_back(&self, dx: f32, dy: f32) -> [f32; 2] {
        let m = self.m;
        let det = m[0] * m[4] - m[1] * m[3];
        [(m[4] * dx - m[1] * dy) / det, (m[0] * dy - m[3] * dx) / det]
    }

    /// The size `geometry::apply` gives a `w × h` base: turned, then cropped.
    pub fn frame_size(w: u32, h: u32, r: &Recipe) -> [f32; 2] {
        let (w, h) = if matches!(r.rotate % 360, 90 | 270) { (h, w) } else { (w, h) };
        match r.crop {
            Some(k) => [(k.w * w as f32).round().max(1.0), (k.h * h as f32).round().max(1.0)],
            None => [w as f32, h as f32],
        }
    }

    /// A displacement in the unedited photo's pixels, in the frame's pixels.
    fn vector(&self, dx: f32, dy: f32) -> [f32; 2] {
        let m = self.m;
        [m[0] * dx + m[1] * dy, m[3] * dx + m[4] * dy]
    }

    /// Frame pixels per unedited-photo pixel.
    pub fn scale(&self) -> f32 {
        let m = self.m;
        (m[0] * m[4] - m[1] * m[3]).abs().sqrt()
    }

    /// The unedited photo's long edge, in frame pixels.
    pub fn long_edge(&self) -> f32 {
        self.base[0].max(self.base[1]) * self.scale()
    }
}

/// What the mask stage leaves for the later ones.
#[derive(Default)]
pub struct Applied<'a> {
    /// The local sharpness to add to the photo's amount per pixel, when a mask without
    /// sharpening of its own has some.
    pub sharpness: Option<Vec<f32>>,
    /// Each mask with an area tool ([`local_tools::Tool`]) and its weights, so no raster is
    /// built twice in a render.
    pub areas: Vec<Area<'a>>,
}

impl Applied<'_> {
    /// Each mask's `tool`, where it has one, run on the box it covers and mixed in by its
    /// weights: the masks' half of a stage after the curve.
    pub fn run(&self, img: &mut Linear, ctx: &Ctx, tool: fn(&LocalAdjust) -> Option<local_tools::Tool>) {
        for area in &self.areas {
            if let Some(t) = tool(area.adjust) {
                let reach = t.reach(ctx, ctx.frame_long(img));
                if t.weighted() {
                    area.mix_weighted(img, ctx, reach, |part, pctx, w| t.run_weighted(part, pctx, w));
                } else {
                    area.mix(img, ctx, reach, |part, pctx| t.run(part, pctx));
                }
            }
        }
    }
}

/// A mask's weights at the render's resolution and the box `[x0, x1) × [y0, y1)` they cover.
pub struct Area<'a> {
    pub adjust: &'a LocalAdjust,
    weights: Vec<f32>,
    cover: (u32, u32, u32, u32),
}

impl Area<'_> {
    /// `effect` on a copy of the box the mask covers, `reach` render pixels wider so its blurs
    /// and streaks see what is round the box, then mixed back by the weights. Outside the
    /// mask no pixel is touched.
    pub fn mix(&self, img: &mut Linear, ctx: &Ctx, reach: f32, effect: impl FnOnce(&mut Linear, &Ctx)) {
        self.mix_with(img, ctx, reach, false, |part, pctx, _| effect(part, pctx));
    }

    /// [`Self::mix`] for a tool that spreads pixels: `effect` also gets the mask's weights over
    /// the same padded box, to weigh what it reads (see [`local_tools::Tool::weighted`]).
    pub fn mix_weighted(&self, img: &mut Linear, ctx: &Ctx, reach: f32, effect: impl FnOnce(&mut Linear, &Ctx, &[f32])) {
        self.mix_with(img, ctx, reach, true, effect);
    }

    fn mix_with(&self, img: &mut Linear, ctx: &Ctx, reach: f32, want_weights: bool, effect: impl FnOnce(&mut Linear, &Ctx, &[f32])) {
        let (x0, y0, x1, y1) = self.cover;
        let (w, h) = (img.width, img.height);
        let pad = reach.max(0.0).ceil() as u32 + 2;
        let (bx0, by0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
        let (bx1, by1) = ((x1 + pad).min(w), (y1 + pad).min(h));
        let mut part = geometry::crop(img, bx0, by0, bx1 - bx0, by1 - by0);
        let (ox, oy, fw, fh) = ctx.frame(img);
        let pctx = Ctx { region: Some(Region { x: ox + bx0, y: oy + by0, frame_width: fw, frame_height: fh }), ..*ctx };
        let weights = &self.weights;
        let pw = (bx1 - bx0) as usize;
        // The mask's weights over the box, only built for a tool that wants them.
        let box_weights: Vec<f32> = if want_weights {
            let mut v = Vec::with_capacity(pw * (by1 - by0) as usize);
            for y in by0 as usize..by1 as usize {
                v.extend_from_slice(&weights[y * w as usize + bx0 as usize..y * w as usize + bx1 as usize]);
            }
            v
        } else {
            Vec::new()
        };
        effect(&mut part, &pctx, &box_weights);
        img.data.par_chunks_mut(w as usize).enumerate().skip(y0 as usize).take((y1 - y0) as usize).for_each(|(y, row)| {
            let prow = &part.data[(y - by0 as usize) * pw..];
            for x in x0 as usize..x1 as usize {
                let m = weights[y * w as usize + x];
                if m > 0.0 {
                    let (p, q) = (&mut row[x], prow[x - bx0 as usize]);
                    *p = std::array::from_fn(|c| p[c] + m * (q[c] - p[c]));
                }
            }
        });
    }
}

/// The mask stage (after global Basic, before the curve): each effective mask's local Basic
/// runs on a copy of the box its weights cover, through the global stage's own code, then its
/// per-pixel tools, and is mixed back by the weights. Returns what the later stages need: the
/// local sharpness and the area tools' weights. `guide` is [`guide_for`], taken before global
/// Basic.
pub fn apply<'a>(img: &mut Linear, masks: &'a [Mask], ctx: &Ctx, guide: Option<&Guide>) -> Applied<'a> {
    let mut out = Applied::default();
    let Some(frame) = ctx.frame_map else { return out };
    let effective: Vec<&Mask> = masks.iter().filter(|m| m.is_effective()).collect();
    if effective.is_empty() || img.data.is_empty() {
        return out;
    }
    let (ox, oy, fw, fh) = ctx.frame(img);
    let frame = frame.offset(-(ox as f32), -(oy as f32));
    let (w, h) = (img.width, img.height);
    let long = ctx.frame_long(img);
    let mut airlight = ctx.airlight;
    for mask in effective {
        let weights = rasterize_with(mask, &frame, w, h, ctx.ai_rasters, guide);
        let Some((x0, y0, x1, y1)) = coverage(&weights, w, h) else { continue };
        let a = &mask.adjust;
        let b = a.basic();
        let tools = local_tools::Stack::new(a);
        let rows = (y0 as usize, (y1 - y0) as usize);
        if b.texture == 0.0 && b.clarity == 0.0 && b.dehaze == 0.0 {
            // Per-pixel sliders only: no copy, and only the pixels under the mask.
            let (gains, tones) = (basic::gains_of(&b), basic::Tones::new(&b));
            if gains.is_some() || tones.is_some() || tools.is_some() {
                img.data.par_chunks_mut(w as usize).enumerate().skip(rows.0).take(rows.1).for_each(|(y, row)| {
                    for x in x0 as usize..x1 as usize {
                        let m = weights[y * w as usize + x];
                        if m > 0.0 {
                            let p = row[x];
                            let mut q = p;
                            if let Some(g) = gains {
                                q = [q[0] * g[0], q[1] * g[1], q[2] * g[2]];
                            }
                            if let Some(t) = &tones {
                                q = t.pixel(q);
                            }
                            if let Some(t) = &tools {
                                q = t.pixel(q);
                            }
                            row[x] = std::array::from_fn(|c| p[c] + m * (q[c] - p[c]));
                        }
                    }
                });
            }
        } else {
            // Room for the local stages' blurs and cells, so the box's edge never shows.
            let pad = (3.0 * CLARITY_SIGMA * long).ceil() as u32 + 2;
            let (bx0, by0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
            let (bx1, by1) = ((x1 + pad).min(w), (y1 + pad).min(h));
            let mut part = geometry::crop(img, bx0, by0, bx1 - bx0, by1 - by0);
            if b.dehaze != 0.0 && airlight.is_none() {
                airlight = Some(local::airlight(img));
            }
            let pctx = Ctx { region: Some(Region { x: ox + bx0, y: oy + by0, frame_width: fw, frame_height: fh }), airlight, ..*ctx };
            basic::apply_gains(&mut part, &b);
            local::texture_clarity(&mut part, b.texture, b.clarity, &pctx);
            local::dehaze(&mut part, b.dehaze, &pctx);
            basic::apply_tones(&mut part, &b);
            let pw = (bx1 - bx0) as usize;
            img.data.par_chunks_mut(w as usize).enumerate().skip(rows.0).take(rows.1).for_each(|(y, row)| {
                let prow = &part.data[(y - by0 as usize) * pw..];
                for x in x0 as usize..x1 as usize {
                    let m = weights[y * w as usize + x];
                    if m > 0.0 {
                        let q = prow[x - bx0 as usize];
                        let (p, q) = (&mut row[x], tools.as_ref().map_or(q, |t| t.pixel(q)));
                        *p = std::array::from_fn(|c| p[c] + m * (q[c] - p[c]));
                    }
                }
            });
        }
        if a.sharpness != 0.0 && a.sharpen.is_none() {
            let s = out.sharpness.get_or_insert_with(|| vec![0.0; weights.len()]);
            s.par_iter_mut().zip(weights.par_iter()).for_each(|(s, m)| *s += m * a.sharpness);
        }
        if local_tools::Tool::any(a) {
            out.areas.push(Area { adjust: a, weights, cover: (x0, y0, x1, y1) });
        }
    }
    out
}

/// The box `[x0, x1) × [y0, y1)` holding every non-zero weight.
fn coverage(weights: &[f32], w: u32, _h: u32) -> Option<(u32, u32, u32, u32)> {
    let rows: Vec<Option<(u32, u32)>> = weights
        .par_chunks(w as usize)
        .map(|row| {
            let first = row.iter().position(|v| *v > 0.0)?;
            let last = row.iter().rposition(|v| *v > 0.0).unwrap_or(first);
            Some((first as u32, last as u32 + 1))
        })
        .collect();
    let y0 = rows.iter().position(Option::is_some)? as u32;
    let y1 = rows.iter().rposition(Option::is_some)? as u32 + 1;
    let (x0, x1) = rows.iter().flatten().fold((w, 0), |(a, b), (f, l)| (a.min(*f), b.max(*l)));
    Some((x0, y0, x1, y1))
}

/// 0 below `e0`, 1 above `e1`, smooth between.
#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-9)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A soft disc: 1 inside `inner`, 0 beyond `outer`, a smoothstep between.
#[inline]
fn falloff(r: f32, inner: f32, outer: f32) -> f32 {
    if r <= inner {
        1.0
    } else if r >= outer {
        0.0
    } else {
        1.0 - smoothstep(inner, outer, r)
    }
}

/// A mask's weights (0–1) over a `w × h` frame mapped by `frame`. Shapes combine in order —
/// an added shape is a union `1 − (1 − m)(1 − s)`, a subtracted one multiplies by `1 − s` —
/// then `invert`, then `opacity`. Rows are independent, so they run in parallel. An AI shape
/// weighs nothing here: see [`rasterize_with`].
pub fn rasterize(mask: &Mask, frame: &FrameMap, w: u32, h: u32) -> Vec<f32> {
    rasterize_with(mask, frame, w, h, None, None)
}

/// [`rasterize`] with the photo's AI rasters. A shape whose raster isn't among them weighs
/// nothing. With a `guide` its edge is refined against the frame; without one it is the raster's
/// own edge, which is what the overlay draws.
pub fn rasterize_with(mask: &Mask, frame: &FrameMap, w: u32, h: u32, rasters: Option<&AiRasters>, guide: Option<&Guide>) -> Vec<f32> {
    let shapes: Vec<Prepared> = mask.shapes.iter().filter_map(|s| Prepared::new(s, frame, (w, h), rasters, guide)).collect();
    let width = w as usize;
    let mut m = vec![0f32; width * h as usize];
    if width == 0 {
        return m;
    }
    let (invert, opacity) = (mask.invert, mask.opacity);
    m.par_chunks_mut(width).enumerate().for_each_init(
        || vec![0f32; width],
        |s, (y, row)| {
            let py = y as f32 + 0.5;
            for shape in &shapes {
                let Some((x0, x1)) = shape.row(py, s) else { continue };
                for (mv, sv) in row[x0..x1].iter_mut().zip(&s[x0..x1]) {
                    *mv = if shape.add { 1.0 - (1.0 - *mv) * (1.0 - sv) } else { *mv * (1.0 - sv) };
                }
            }
            if invert || opacity < 1.0 {
                for v in row.iter_mut() {
                    *v = if invert { 1.0 - *v } else { *v } * opacity;
                }
            }
        },
    );
    m
}

/// A shape mapped into the frame once, then evaluated a row at a time.
struct Prepared {
    add: bool,
    kind: Kind,
}

enum Kind {
    /// Full at `a`, none at `a + d`.
    Linear { a: [f32; 2], d: [f32; 2], len2: f32 },
    /// The centre and the two semi-axes as frame vectors (with their squared lengths).
    Radial { c: [f32; 2], e1: [f32; 2], e2: [f32; 2], l1: f32, l2: f32, inner: f32, inside: bool },
    /// Dab centres sorted by y; radius and inner radius in frame pixels.
    Brush { dabs: Vec<[f32; 2]>, r: f32, inner: f32, flow: f32 },
    /// An AI raster, sampled and refined over the whole frame once.
    Raster(Vec<f32>),
}

impl Prepared {
    fn new(shape: &MaskShape, f: &FrameMap, (w, h): (u32, u32), rasters: Option<&AiRasters>, guide: Option<&Guide>) -> Option<Self> {
        Some(match shape {
            MaskShape::Ai { add, raster, .. } => {
                let mut weights = sample_raster(rasters?.get(raster)?, f, w, h)?;
                if let Some(g) = guide.filter(|g| g.width == w as usize && g.rgb.len() == weights.len()) {
                    refine(&mut weights, g);
                }
                Prepared { add: *add, kind: Kind::Raster(weights) }
            }
            MaskShape::Linear { add, from, to } => {
                let a = f.to_frame(from[0], from[1]);
                let b = f.to_frame(to[0], to[1]);
                let d = [b[0] - a[0], b[1] - a[1]];
                Prepared { add: *add, kind: Kind::Linear { a, d, len2: d[0] * d[0] + d[1] * d[1] } }
            }
            MaskShape::Radial { add, center, radius, angle, feather, inside } => {
                let (sa, ca) = angle.to_radians().sin_cos();
                let (rx, ry) = (radius[0] * f.base[0], radius[1] * f.base[1]);
                let e1 = f.vector(ca * rx, sa * rx);
                let e2 = f.vector(-sa * ry, ca * ry);
                Prepared {
                    add: *add,
                    kind: Kind::Radial {
                        c: f.to_frame(center[0], center[1]),
                        e1,
                        e2,
                        l1: (e1[0] * e1[0] + e1[1] * e1[1]).max(1e-9),
                        l2: (e2[0] * e2[0] + e2[1] * e2[1]).max(1e-9),
                        inner: 1.0 - feather,
                        inside: *inside,
                    },
                }
            }
            MaskShape::Brush { add, radius, feather, flow, points } => {
                let r = radius * f.long_edge();
                if points.is_empty() || r <= 0.0 {
                    return None;
                }
                let mut dabs = dabs(f, r, points);
                dabs.sort_by(|a, b| a[1].total_cmp(&b[1]));
                let inner = r * (1.0 - feather);
                // A disc under a pixel wide only counts a pixel whose centre it covers, so a hair
                // line vanished or flickered with where it fell. Such a brush is the same disc seen
                // through a one-pixel box: its edge widens by half a pixel each side and its flow
                // falls with its width, so a line of 1 px marks its pixels at least half and one a
                // quarter as wide, in a smaller preview, a quarter as hard.
                let (r, inner, flow) = if r < 1.0 { (r + 0.5, (inner - 0.5).max(0.0), flow * (2.0 * r).min(1.0)) } else { (r, inner, *flow) };
                Prepared { add: *add, kind: Kind::Brush { dabs, r, inner, flow } }
            }
        })
    }

    /// Row `py`'s weights written into `s[x0..x1]`; outside that span they are 0.
    fn row(&self, py: f32, s: &mut [f32]) -> Option<(usize, usize)> {
        let w = s.len();
        match &self.kind {
            Kind::Raster(weights) => {
                let y = py as usize;
                s.copy_from_slice(&weights[y * w..(y + 1) * w]);
                Some((0, w))
            }
            Kind::Linear { a, d, len2 } => {
                for (x, v) in s.iter_mut().enumerate() {
                    *v = if *len2 < 1e-9 {
                        0.0
                    } else {
                        let t = ((x as f32 + 0.5 - a[0]) * d[0] + (py - a[1]) * d[1]) / len2;
                        1.0 - smoothstep(0.0, 1.0, t)
                    };
                }
                Some((0, w))
            }
            Kind::Radial { c, e1, e2, l1, l2, inner, inside } => {
                let dy = py - c[1];
                for (x, v) in s.iter_mut().enumerate() {
                    let dx = x as f32 + 0.5 - c[0];
                    let q1 = (dx * e1[0] + dy * e1[1]) / l1;
                    let q2 = (dx * e2[0] + dy * e2[1]) / l2;
                    let f = falloff((q1 * q1 + q2 * q2).sqrt(), *inner, 1.0);
                    *v = if *inside { f } else { 1.0 - f };
                }
                Some((0, w))
            }
            Kind::Brush { dabs, r, inner, flow } => {
                // `s` holds what is left, Π(1 − flow·w), over the span the dabs reach.
                let first = dabs.partition_point(|d| d[1] < py - r);
                let (r2, inner2) = (r * r, inner * inner);
                let mut span: Option<(usize, usize)> = None;
                for d in dabs[first..].iter().take_while(|d| d[1] <= py + r) {
                    let dy = py - d[1];
                    let half = (r2 - dy * dy).max(0.0).sqrt();
                    let lo = ((d[0] - half).floor().max(0.0) as usize).min(w);
                    let hi = ((d[0] + half).ceil().max(0.0) as usize).min(w);
                    if lo >= hi {
                        continue;
                    }
                    span = Some(match span {
                        None => {
                            s[lo..hi].fill(1.0);
                            (lo, hi)
                        }
                        Some((x0, x1)) => {
                            let (n0, n1) = (x0.min(lo), x1.max(hi));
                            s[n0..x0].fill(1.0);
                            s[x1..n1].fill(1.0);
                            (n0, n1)
                        }
                    });
                    for (x, keep) in s[lo..hi].iter_mut().enumerate() {
                        let dx = (lo + x) as f32 + 0.5 - d[0];
                        let q = dx * dx + dy * dy;
                        if q < r2 {
                            let v = if q <= inner2 { 1.0 } else { falloff(q.sqrt(), *inner, *r) };
                            *keep *= 1.0 - flow * v;
                        }
                    }
                }
                let (x0, x1) = span?;
                s[x0..x1].iter_mut().for_each(|v| *v = 1.0 - *v);
                Some((x0, x1))
            }
        }
    }
}

/// A raster over a `w × h` frame: bilinear between its cells' centres, in the unedited photo's
/// coordinates through `f`, held at its edge beyond them. `None` for an empty raster.
fn sample_raster(raster: &AiMaskRaster, f: &FrameMap, w: u32, h: u32) -> Option<Vec<f32>> {
    let (rw, rh) = (raster.width as usize, raster.height as usize);
    if rw == 0 || rh == 0 || raster.weights.len() != rw * rh {
        return None;
    }
    let width = w as usize;
    let mut out = vec![0f32; width * h as usize];
    if width == 0 {
        return Some(out);
    }
    out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        let py = y as f32 + 0.5;
        // The photo's coordinates are linear along a row of the frame.
        let [u0, v0] = f.from_frame(0.0, py);
        let [u1, v1] = f.from_frame(1.0, py);
        let (du, dv) = (u1 - u0, v1 - v0);
        for (x, o) in row.iter_mut().enumerate() {
            let t = x as f32 + 0.5;
            let gx = ((u0 + du * t) * rw as f32 - 0.5).clamp(0.0, rw as f32 - 1.0);
            let gy = ((v0 + dv * t) * rh as f32 - 0.5).clamp(0.0, rh as f32 - 1.0);
            let (x0, y0) = (gx as usize, gy as usize);
            let (x1, y1) = ((x0 + 1).min(rw - 1), (y0 + 1).min(rh - 1));
            let (tx, ty) = (gx - x0 as f32, gy - y0 as f32);
            let at = |x: usize, y: usize| raster.weights[y * rw + x] as f32;
            let top = at(x0, y0) + tx * (at(x1, y0) - at(x0, y0));
            let bottom = at(x0, y1) + tx * (at(x1, y1) - at(x0, y1));
            *o = (top + ty * (bottom - top)) / 255.0;
        }
    });
    Some(out)
}

/// How far an AI edge can move, as a share of the unedited photo's long edge: two of a raster's
/// cells (160 on its long side). Of that long edge rather than the crop's, so a crop doesn't
/// change the edge; the same share at every size, so preview, 1:1 and export agree.
const REFINE_RADIUS: f32 = 2.0 / 160.0;
/// The filter's ε on `sqrt` colour: contrast well under 1.5 % counts as flat, so the mask
/// stays soft there instead of following noise. A dark backdrop's edge is about that faint.
const REFINE_EPS: f32 = 2e-4;
/// Filtered weights under this are nothing, and as far from 1 are everything.
const REFINE_TIGHTEN: f32 = 0.2;
/// Each pass fits the last one's tightened weights again, so sky a model held back from a roof,
/// a pole or a sign is taken back by its colour instead of staying a light fringe.
const REFINE_PASSES: usize = 2;
/// The guide's `sqrt` channels are stored as `u16` over this range (linear up to 4).
const GUIDE_SCALE: f32 = 2.0;
/// The radius in the coarse grid's cells: about the same grid at every render size.
const REFINE_CELLS: f32 = 6.0;

/// The coarse grid of a fast guided filter (He & Sun, 2015): its cells are anchored to the
/// frame's corner, so a part of the frame has the same cells as the whole.
#[derive(Debug, Clone, Copy)]
struct Grid {
    /// A cell's side in pixels.
    cell: usize,
    /// The filter's radius in cells.
    radius: usize,
    /// Where the part sits in the frame, in pixels.
    origin: (usize, usize),
    /// The part's first cell, in the frame's cells.
    first: (usize, usize),
    w: usize,
    h: usize,
}

impl Grid {
    fn sizes(long_edge: f32) -> (usize, usize) {
        let radius = (long_edge * REFINE_RADIUS).max(1.0);
        let cell = ((radius / REFINE_CELLS).round() as usize).max(1);
        (cell, ((radius / cell as f32).round() as usize).max(1))
    }

    fn new(long_edge: f32, origin: (usize, usize), width: usize, height: usize) -> Self {
        let (cell, radius) = Self::sizes(long_edge);
        let first = (origin.0 / cell, origin.1 / cell);
        let w = (origin.0 + width.max(1) - 1) / cell + 1 - first.0;
        let h = (origin.1 + height.max(1) - 1) / cell + 1 - first.1;
        Self { cell, radius, origin, first, w, h }
    }

    /// `src` (a part's pixels) averaged into the grid's cells.
    fn down(&self, src: &[f32], width: usize) -> Vec<f32> {
        self.down_n(src, width, |v: &f32| [*v]).into_iter().map(|[v]| v).collect()
    }

    /// [`Grid::down`] of `K` channels read from each pixel by `at`.
    fn down_n<T: Sync, const K: usize>(&self, src: &[T], width: usize, at: impl Fn(&T) -> [f32; K] + Sync) -> Vec<[f32; K]> {
        let height = src.len() / width.max(1);
        let mut out = vec![[0f32; K]; self.w * self.h];
        out.par_chunks_mut(self.w).enumerate().for_each_init(
            || vec![0u32; self.w],
            |counts, (cy, row)| {
                counts.fill(0);
                let top = (self.first.1 + cy) * self.cell;
                let y0 = top.saturating_sub(self.origin.1);
                let y1 = (top + self.cell).saturating_sub(self.origin.1).min(height);
                for line in src.chunks_exact(width).take(y1).skip(y0) {
                    for (x, v) in line.iter().enumerate() {
                        let c = (x + self.origin.0) / self.cell - self.first.0;
                        let v = at(v);
                        for k in 0..K {
                            row[c][k] += v[k];
                        }
                        counts[c] += 1;
                    }
                }
                for (v, n) in row.iter_mut().zip(counts.iter()) {
                    v.iter_mut().for_each(|v| *v /= (*n).max(1) as f32);
                }
            },
        );
        out
    }

    /// The mean of a `2·radius + 1` window of cells around each cell, clipped at the grid's edge.
    fn mean(&self, src: &[f32]) -> Vec<f32> {
        let (w, h, r) = (self.w, self.h, self.radius);
        let stride = w + 1;
        let mut table = vec![0f64; stride * (h + 1)];
        for y in 0..h {
            let mut run = 0f64;
            for x in 0..w {
                run += src[y * w + x] as f64;
                table[(y + 1) * stride + x + 1] = table[y * stride + x + 1] + run;
            }
        }
        let mut out = vec![0f32; w * h];
        out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let (t, b) = (y.saturating_sub(r), (y + r + 1).min(h));
            for (x, v) in row.iter_mut().enumerate() {
                let (l, e) = (x.saturating_sub(r), (x + r + 1).min(w));
                let sum = table[b * stride + e] - table[t * stride + e] - table[b * stride + l] + table[t * stride + l];
                *v = (sum / ((b - t) * (e - l)) as f64) as f32;
            }
        });
        out
    }

    /// Where a pixel of the part falls between cell centres along one axis: the cell before it,
    /// the cell after it, and how far along.
    fn between(&self, pixel: usize, origin: usize, first: usize, cells: usize) -> (usize, usize, f32) {
        let t = (((pixel + origin) as f32 + 0.5) / self.cell as f32 - 0.5 - first as f32).clamp(0.0, cells as f32 - 1.0);
        let i = t as usize;
        (i, (i + 1).min(cells - 1), t - i as f32)
    }
}

/// How far past what is shown a part of a frame must reach for a refined AI edge to match the
/// whole's: the two windows a pixel's coefficients come from, its cell and the one it blends with,
/// for each pass.
pub fn refine_pad(map: &FrameMap) -> u32 {
    let (cell, radius) = Grid::sizes(map.long_edge());
    (REFINE_PASSES * (2 * radius + 3) * cell) as u32
}

/// What refining an AI raster needs from a frame: its colour before global Basic, so moving a
/// slider never moves a mask's edge. Colour, not luminance, so a blue sky and a warm wall just as
/// bright still part.
#[derive(Debug)]
pub struct Guide {
    width: usize,
    /// Each pixel's `sqrt` channels over `0..GUIDE_SCALE`, as `u16`: half the memory of `f32`.
    rgb: Vec<[u16; 3]>,
    grid: Grid,
    /// Per cell: its own mean colour, its window's, and the inverse of its window's covariance
    /// plus ε (symmetric: xx, xy, xz, yy, yz, zz).
    coarse: [Vec<f32>; 3],
    mean: [Vec<f32>; 3],
    inv: Vec<[f32; 6]>,
}

#[inline]
fn guide_level(v: u16) -> f32 {
    v as f32 * (GUIDE_SCALE / 65535.0)
}

/// The inverse of the symmetric `[xx, xy, xz, yy, yz, zz]`.
fn inverse3(m: [f32; 6]) -> [f32; 6] {
    let [a, b, c, d, e, f] = m.map(f64::from);
    let (i0, i1, i2) = (d * f - e * e, c * e - b * f, b * e - c * d);
    let det = a * i0 + b * i1 + c * i2;
    if det.abs() < 1e-30 {
        return [0.0; 6];
    }
    [i0, i1, i2, a * f - c * c, b * c - a * e, a * d - b * b].map(|v| (v / det) as f32)
}

impl Guide {
    /// `long_edge` is the unedited photo's in frame pixels ([`FrameMap::long_edge`]) and
    /// `origin` the corner of `img` in its frame.
    pub fn new(img: &Linear, long_edge: f32, origin: (u32, u32)) -> Self {
        let width = img.width as usize;
        let rgb: Vec<[u16; 3]> = img.data.par_iter().map(|p| p.map(|c| ((c.max(0.0).sqrt() / GUIDE_SCALE).min(1.0) * 65535.0).round() as u16)).collect();
        let grid = Grid::new(long_edge, (origin.0 as usize, origin.1 as usize), width, img.height as usize);
        let cells = grid.down_n(&rgb, width, |p| p.map(guide_level));
        let coarse: [Vec<f32>; 3] = std::array::from_fn(|c| cells.iter().map(|v| v[c]).collect());
        let mean: [Vec<f32>; 3] = std::array::from_fn(|c| grid.mean(&coarse[c]));
        let pairs = [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)];
        let products: Vec<Vec<f32>> = pairs.iter().map(|&(i, j)| grid.mean(&coarse[i].iter().zip(&coarse[j]).map(|(a, b)| a * b).collect::<Vec<_>>())).collect();
        let inv = (0..cells.len())
            .map(|n| {
                let cov: [f32; 6] = std::array::from_fn(|k| {
                    let (i, j) = pairs[k];
                    products[k][n] - mean[i][n] * mean[j][n] + if i == j { REFINE_EPS } else { 0.0 }
                });
                inverse3(cov)
            })
            .collect();
        Self { width, rgb, grid, coarse, mean, inv }
    }
}

/// The guide for a develop's masks, taken from the image as it is on entry: `None` unless a mask
/// that will render has a raster to refine.
pub fn guide_for(img: &Linear, masks: &[Mask], ctx: &Ctx) -> Option<Guide> {
    let (rasters, map) = (ctx.ai_rasters?, ctx.frame_map?);
    let needed = masks.iter().filter(|m| m.is_effective()).flat_map(|m| &m.shapes).any(|s| matches!(s, MaskShape::Ai { raster, .. } if rasters.contains_key(raster)));
    if !needed || img.data.is_empty() {
        return None;
    }
    let (ox, oy, _, _) = ctx.frame(img);
    Some(Guide::new(img, map.long_edge(), (ox, oy)))
}

/// A fast colour guided filter of `p` (He et al.; He & Sun, 2015): the linear fit `q = a·I + b`
/// of every window of the coarse grid against the guide's three channels, those coefficients
/// smoothed, then applied at the render's own resolution and tightened; [`REFINE_PASSES`] times.
/// Each pass is affine in `p` and the curve is symmetric, so a mask's inverse refines to the
/// inverse of its refinement.
fn refine(p: &mut [f32], g: &Guide) {
    let grid = &g.grid;
    let (cw, ch) = (grid.w, grid.h);
    let across: Vec<(usize, usize, f32)> = (0..g.width).map(|x| grid.between(x, grid.origin.0, grid.first.0, cw)).collect();
    for _ in 0..REFINE_PASSES {
        let coarse = grid.down(p, g.width);
        let mean_p = grid.mean(&coarse);
        let mean_ip: [Vec<f32>; 3] = std::array::from_fn(|c| grid.mean(&coarse.iter().zip(&g.coarse[c]).map(|(a, b)| a * b).collect::<Vec<_>>()));
        let n = coarse.len();
        let (mut a, mut b): ([Vec<f32>; 3], Vec<f32>) = (std::array::from_fn(|_| vec![0f32; n]), vec![0f32; n]);
        for i in 0..n {
            let cov: [f32; 3] = std::array::from_fn(|c| mean_ip[c][i] - g.mean[c][i] * mean_p[i]);
            let [xx, xy, xz, yy, yz, zz] = g.inv[i];
            let k = [xx * cov[0] + xy * cov[1] + xz * cov[2], xy * cov[0] + yy * cov[1] + yz * cov[2], xz * cov[0] + yz * cov[1] + zz * cov[2]];
            for c in 0..3 {
                a[c][i] = k[c];
            }
            b[i] = mean_p[i] - (0..3).map(|c| k[c] * g.mean[c][i]).sum::<f32>();
        }
        let a: [Vec<f32>; 3] = a.map(|f| grid.mean(&f));
        let b = grid.mean(&b);
        p.par_chunks_mut(g.width).enumerate().for_each(|(y, row)| {
            let (j0, j1, ty) = grid.between(y, grid.origin.1, grid.first.1, ch);
            let guide = &g.rgb[y * g.width..(y + 1) * g.width];
            for ((v, px), &(i0, i1, tx)) in row.iter_mut().zip(guide).zip(&across) {
                let at = |f: &[f32]| {
                    let top = f[j0 * cw + i0] + tx * (f[j0 * cw + i1] - f[j0 * cw + i0]);
                    let bottom = f[j1 * cw + i0] + tx * (f[j1 * cw + i1] - f[j1 * cw + i0]);
                    top + ty * (bottom - top)
                };
                let q = at(&b) + (0..3).map(|c| at(&a[c]) * guide_level(px[c])).sum::<f32>();
                // The curve takes off the glow a flat window's blur leaves, and is exactly 0 and 1
                // away from an edge, so a mask's box stays honest. Symmetric: `s(1 − q) = 1 − s(q)`.
                *v = smoothstep(REFINE_TIGHTEN, 1.0 - REFINE_TIGHTEN, q);
            }
        });
    }
}

/// Dab centres every `r/4` along a stroke, in frame pixels.
fn dabs(f: &FrameMap, r: f32, points: &[[u16; 2]]) -> Vec<[f32; 2]> {
    let g = MASK_GRID as f32;
    let spacing = (r / 4.0).max(0.5);
    let centres: Vec<[f32; 2]> = points.iter().map(|p| f.to_frame(p[0] as f32 / g, p[1] as f32 / g)).collect();
    let mut out = vec![centres[0]];
    for pair in centres.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let steps = ((b[0] - a[0]).hypot(b[1] - a[1]) / spacing).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use photos_core::ai_masks::AiMaskKind;
    use photos_core::edits::{Crop, LocalAdjust};

    fn mask(shapes: Vec<MaskShape>) -> Mask {
        Mask { id: 1, name: "m".into(), shapes, invert: false, opacity: 1.0, adjust: LocalAdjust { exposure: 1.0, ..Default::default() }, visible: true }
    }

    fn radial(center: [f32; 2], radius: [f32; 2], angle: f32, feather: f32) -> MaskShape {
        MaskShape::Radial { add: true, center, radius, angle, feather, inside: true }
    }

    #[test]
    fn from_frame_undoes_to_frame_through_turns_straighten_and_crop() {
        let r = Recipe { rotate: 90, flip_h: true, straighten: 7.0, crop: Some(Crop { x: 0.1, y: 0.2, w: 0.6, h: 0.5 }), ..Recipe::identity() };
        let map = FrameMap::new(600, 400, &r);
        for (u, v) in [(0.3, 0.4), (0.5, 0.5), (0.7, 0.2)] {
            let [x, y] = map.to_frame(u, v);
            let [u2, v2] = map.from_frame(x, y);
            assert!((u - u2).abs() < 1e-4 && (v - v2).abs() < 1e-4, "{u},{v} → {u2},{v2}");
        }
        assert_eq!(FrameMap::frame_size(600, 400, &r), [240.0, 300.0], "turned, then cropped");
    }

    fn brush(add: bool, feather: f32, points: Vec<[f32; 2]>) -> MaskShape {
        let g = MASK_GRID as f32;
        MaskShape::Brush { add, radius: 0.06, feather, flow: 1.0, points: points.into_iter().map(|p| [(p[0] * g).round() as u16, (p[1] * g).round() as u16]).collect() }
    }

    fn at(w: &[f32], width: u32, x: u32, y: u32) -> f32 {
        w[(y * width + x) as usize]
    }

    #[test]
    fn a_hard_radial_is_one_inside_and_zero_outside() {
        let (w, h) = (200, 100);
        let m = mask(vec![radial([0.5, 0.5], [0.2, 0.3], 0.0, 0.0)]);
        let wts = rasterize(&m, &FrameMap::identity(w, h), w, h);
        assert_eq!(at(&wts, w, 100, 50), 1.0);
        assert_eq!(at(&wts, w, 100 + 38, 50), 1.0, "inside the 40 px half-width");
        assert_eq!(at(&wts, w, 100 + 42, 50), 0.0);
        assert_eq!(at(&wts, w, 100, 50 + 32), 0.0, "the 30 px half-height");
        assert!(wts.iter().all(|v| *v == 0.0 || *v == 1.0));
        let outside = Mask { shapes: vec![MaskShape::Radial { add: true, center: [0.5, 0.5], radius: [0.2, 0.3], angle: 0.0, feather: 0.0, inside: false }], ..m };
        let o = rasterize(&outside, &FrameMap::identity(w, h), w, h);
        assert_eq!((at(&o, w, 100, 50), at(&o, w, 5, 5)), (0.0, 1.0));
    }

    #[test]
    fn a_turned_radial_turns() {
        let (w, h) = (200, 200);
        let m = mask(vec![radial([0.5, 0.5], [0.3, 0.05], 90.0, 0.0)]);
        let wts = rasterize(&m, &FrameMap::identity(w, h), w, h);
        assert_eq!(at(&wts, w, 100, 100 + 50), 1.0, "the long axis now runs down");
        assert_eq!(at(&wts, w, 100 + 50, 100), 0.0);
    }

    #[test]
    fn a_linear_gradient_falls_monotonically_from_full_to_none() {
        let (w, h) = (100, 80);
        let m = mask(vec![MaskShape::Linear { add: true, from: [0.5, 0.2], to: [0.5, 0.7] }]);
        let wts = rasterize(&m, &FrameMap::identity(w, h), w, h);
        let col: Vec<f32> = (0..h).map(|y| at(&wts, w, 30, y)).collect();
        assert!(col.windows(2).all(|p| p[1] <= p[0] + 1e-6), "{col:?}");
        assert_eq!(col[0], 1.0, "full before `from`");
        assert_eq!(col[79], 0.0, "none after `to`");
        assert!((at(&wts, w, 30, 36) - 0.5).abs() < 0.08, "half way is about half");
    }

    #[test]
    fn erasing_along_the_same_path_takes_the_stroke_away() {
        let (w, h) = (300, 200);
        let path = vec![[0.2, 0.3], [0.5, 0.6], [0.8, 0.4]];
        let painted = rasterize(&mask(vec![brush(true, 0.0, path.clone())]), &FrameMap::identity(w, h), w, h);
        assert!(painted.iter().sum::<f32>() > 1000.0, "the stroke paints");
        let hard = rasterize(&mask(vec![brush(true, 0.0, path.clone()), brush(false, 0.0, path.clone())]), &FrameMap::identity(w, h), w, h);
        assert!(hard.iter().all(|v| *v < 1e-6), "a hard eraser over a hard stroke leaves nothing");
        let soft_painted = rasterize(&mask(vec![brush(true, 0.6, path.clone())]), &FrameMap::identity(w, h), w, h);
        let soft = rasterize(&mask(vec![brush(true, 0.6, path.clone()), brush(false, 0.6, path)]), &FrameMap::identity(w, h), w, h);
        let (a, b) = (soft_painted.iter().sum::<f32>(), soft.iter().sum::<f32>());
        assert!(b < 0.1 * a, "a soft eraser leaves only the feathered rim: {b} of {a}");
    }

    #[test]
    fn invert_and_opacity_come_last() {
        let (w, h) = (100, 100);
        let m = Mask { invert: true, opacity: 0.5, ..mask(vec![radial([0.5, 0.5], [0.2, 0.2], 0.0, 0.0)]) };
        let wts = rasterize(&m, &FrameMap::identity(w, h), w, h);
        assert_eq!((at(&wts, w, 50, 50), at(&wts, w, 2, 2)), (0.0, 0.5));
        let empty = Mask { invert: true, ..mask(vec![]) };
        assert!(rasterize(&empty, &FrameMap::identity(w, h), w, h).iter().all(|v| *v == 1.0), "an inverted empty mask is everything");
    }

    /// What the frame shows at unedited point `p`, sampled from a raster of the frame.
    fn weight_at(wts: &[f32], w: u32, h: u32, f: &FrameMap, p: [f32; 2]) -> f32 {
        let q = f.to_frame(p[0], p[1]);
        let (x, y) = (q[0].floor() as i64, q[1].floor() as i64);
        assert!(x >= 0 && y >= 0 && x < w as i64 && y < h as i64, "{p:?} → {q:?} is outside {w}x{h}");
        wts[(y as u32 * w + x as u32) as usize]
    }

    #[test]
    fn a_mask_stays_put_when_the_crop_changes() {
        let (bw, bh) = (400, 300);
        let m = mask(vec![radial([0.4, 0.5], [0.15, 0.2], 25.0, 0.7), brush(true, 0.5, vec![[0.3, 0.3], [0.6, 0.45]])]);
        let a = Recipe { crop: Some(Crop { x: 0.1, y: 0.1, w: 0.7, h: 0.8 }), ..Recipe::identity() };
        let b = Recipe { crop: Some(Crop { x: 0.25, y: 0.3, w: 0.5, h: 0.6 }), ..Recipe::identity() };
        let (fa, fb) = (FrameMap::new(bw, bh, &a), FrameMap::new(bw, bh, &b));
        let (sa, sb) = (a.output_size(bw, bh), b.output_size(bw, bh));
        let (wa, wb) = (rasterize(&m, &fa, sa.0, sa.1), rasterize(&m, &fb, sb.0, sb.1));
        for p in [[0.4, 0.5], [0.45, 0.55], [0.35, 0.42], [0.5, 0.4], [0.3, 0.6]] {
            let (x, y) = (weight_at(&wa, sa.0, sa.1, &fa, p), weight_at(&wb, sb.0, sb.1, &fb, p));
            assert!((x - y).abs() < 1e-3, "{p:?}: {x} vs {y}");
        }
        assert!(weight_at(&wa, sa.0, sa.1, &fa, [0.4, 0.5]) > 0.9, "the centre is inside the mask");
        // Partly outside the second crop: clipped, not squeezed.
        assert_eq!(wb.len() as u32, sb.0 * sb.1);
    }

    #[test]
    fn masks_follow_turns_and_flips() {
        use super::super::{geometry, Linear};
        let (bw, bh) = (120, 80);
        let m = mask(vec![radial([0.3, 0.35], [0.2, 0.15], 30.0, 0.5), brush(true, 0.4, vec![[0.6, 0.2], [0.85, 0.7]])]);
        let flat = rasterize(&m, &FrameMap::identity(bw, bh), bw, bh);
        assert!(flat.iter().sum::<f32>() > 200.0, "the mask covers something");
        let as_image = Linear { width: bw, height: bh, data: flat.iter().map(|v| [*v; 3]).collect() };
        for r in [
            Recipe { rotate: 90, ..Recipe::identity() },
            Recipe { rotate: 180, flip_h: true, ..Recipe::identity() },
            Recipe { rotate: 270, flip_v: true, ..Recipe::identity() },
            Recipe { flip_h: true, flip_v: true, crop: Some(Crop { x: 0.1, y: 0.2, w: 0.6, h: 0.7 }), ..Recipe::identity() },
            Recipe { rotate: 90, straighten: 8.0, crop: Some(Crop { x: 0.05, y: 0.1, w: 0.8, h: 0.75 }), ..Recipe::identity() },
        ] {
            let want = geometry::apply(&as_image, &r);
            let got = rasterize(&m, &FrameMap::new(bw, bh, &r), want.width, want.height);
            let worst = want.data.iter().zip(&got).map(|(a, b)| (a[0] - b).abs()).fold(0.0f32, f32::max);
            let mean = want.data.iter().zip(&got).map(|(a, b)| (a[0] - b).abs()).sum::<f32>() / got.len() as f32;
            // Straighten resamples the raster bilinearly; the mask is evaluated exactly.
            let (wmax, wmean) = if r.straighten != 0.0 { (0.2, 0.01) } else { (1e-4, 1e-5) };
            assert!(worst <= wmax && mean <= wmean, "{r:?}: worst {worst}, mean {mean}");
        }
    }

    fn ai_shape(hash: &str) -> MaskShape {
        MaskShape::Ai { add: true, target: AiMaskKind::Sky, instance: 0, raster: hash.into() }
    }

    fn raster(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> AiMaskRaster {
        AiMaskRaster { kind: AiMaskKind::Sky, instance: 0, width: w, height: h, weights: (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect() }
    }

    fn rasters(r: AiMaskRaster) -> AiRasters {
        AiRasters::from([("aa".to_string(), r)])
    }

    /// The geometry every shape has to follow: flips, turns, straighten and crop.
    fn geometries() -> Vec<Recipe> {
        vec![
            Recipe::identity(),
            Recipe { flip_h: true, ..Recipe::identity() },
            Recipe { flip_v: true, ..Recipe::identity() },
            Recipe { rotate: 90, ..Recipe::identity() },
            Recipe { rotate: 180, flip_h: true, ..Recipe::identity() },
            Recipe { rotate: 270, flip_v: true, ..Recipe::identity() },
            Recipe { crop: Some(Crop { x: 0.2, y: 0.1, w: 0.6, h: 0.7 }), ..Recipe::identity() },
            Recipe { straighten: 8.0, ..Recipe::identity() },
            Recipe { rotate: 90, straighten: -5.0, flip_h: true, crop: Some(Crop { x: 0.05, y: 0.1, w: 0.8, h: 0.75 }), ..Recipe::identity() },
        ]
    }

    #[test]
    fn a_stored_edge_lands_where_it_should_through_every_geometry() {
        // A vertical edge at u = 0.5 and a horizontal one at v = 0.25: bilinear between cell
        // centres, so each crosses a half at exactly that line, and is known everywhere.
        let (rw, rh) = (160u32, 120u32);
        let vertical = rasters(raster(rw, rh, |x, _| if x < rw / 2 { 255 } else { 0 }));
        let horizontal = rasters(raster(rw, rh, |_, y| if y < rh / 4 { 255 } else { 0 }));
        let m = mask(vec![ai_shape("aa")]);
        let (bw, bh) = (960u32, 720u32);
        for r in geometries() {
            let f = FrameMap::new(bw, bh, &r);
            let (w, h) = r.output_size(bw, bh);
            for (rasters, axis, cells, line) in [(&vertical, 0, rw as f32, 0.5f32), (&horizontal, 1, rh as f32, 0.25)] {
                let wts = rasterize_with(&m, &f, w, h, Some(rasters), None);
                let (mut worst, mut crossed) = (0f32, false);
                for y in (0..h).step_by(7) {
                    for x in (0..w).step_by(5) {
                        let p = f.from_frame(x as f32 + 0.5, y as f32 + 0.5);
                        // The weight falls from 1 to 0 across one cell, centred on the line.
                        let want = (0.5 - (p[axis] - line) * cells).clamp(0.0, 1.0);
                        worst = worst.max((at(&wts, w, x, y) - want).abs());
                        crossed |= want == 1.0 || want == 0.0;
                    }
                }
                assert!(crossed && worst < 0.01, "{r:?} axis {axis}: worst {worst}");
            }
        }
    }

    #[test]
    fn an_ai_shape_follows_turns_and_flips_like_the_pixels_do() {
        let (bw, bh) = (240u32, 160u32);
        let store = rasters(raster(24, 16, |x, y| ((x * 7 + y * 13) % 2 * 200 + x * 2) as u8));
        let m = mask(vec![ai_shape("aa")]);
        let flat = rasterize_with(&m, &FrameMap::identity(bw, bh), bw, bh, Some(&store), None);
        let as_image = Linear { width: bw, height: bh, data: flat.iter().map(|v| [*v; 3]).collect() };
        for r in geometries().into_iter().filter(|r| r.straighten == 0.0) {
            let want = geometry::apply(&as_image, &r);
            let got = rasterize_with(&m, &FrameMap::new(bw, bh, &r), want.width, want.height, Some(&store), None);
            let worst = want.data.iter().zip(&got).map(|(a, b)| (a[0] - b).abs()).fold(0.0f32, f32::max);
            assert!(worst < 1e-3, "{r:?}: worst {worst}");
        }
    }

    #[test]
    fn a_missing_raster_weighs_nothing_and_combines_as_nothing() {
        let (w, h) = (100, 80);
        let f = FrameMap::identity(w, h);
        let empty = AiRasters::new();
        for store in [None, Some(&empty)] {
            assert!(rasterize_with(&mask(vec![ai_shape("aa")]), &f, w, h, store, None).iter().all(|v| *v == 0.0));
            let with_radial = mask(vec![radial([0.5, 0.5], [0.2, 0.2], 0.0, 0.0), ai_shape("aa")]);
            assert_eq!(rasterize_with(&with_radial, &f, w, h, store, None), rasterize(&mask(vec![radial([0.5, 0.5], [0.2, 0.2], 0.0, 0.0)]), &f, w, h));
        }
        let broken = AiRasters::from([("aa".to_string(), AiMaskRaster { width: 4, height: 4, weights: vec![255; 3], ..raster(1, 1, |_, _| 0) })]);
        assert!(rasterize_with(&mask(vec![ai_shape("aa")]), &f, w, h, Some(&broken), None).iter().all(|v| *v == 0.0), "a raster of the wrong size is no raster");
    }

    #[test]
    fn an_ai_shape_combines_like_any_shape() {
        let (w, h) = (100, 100);
        let f = FrameMap::identity(w, h);
        let store = rasters(raster(10, 10, |x, _| if x < 5 { 255 } else { 0 }));
        let sub = MaskShape::Ai { add: false, target: AiMaskKind::Sky, instance: 0, raster: "aa".into() };
        let m = mask(vec![radial([0.5, 0.5], [0.45, 0.45], 0.0, 0.0), sub]);
        let wts = rasterize_with(&m, &f, w, h, Some(&store), None);
        assert_eq!((at(&wts, w, 20, 50), at(&wts, w, 80, 50)), (0.0, 1.0), "subtracted from the disc");
        let inverted = Mask { invert: true, opacity: 0.5, ..mask(vec![ai_shape("aa")]) };
        let wts = rasterize_with(&inverted, &f, w, h, Some(&store), None);
        assert_eq!((at(&wts, w, 20, 50), at(&wts, w, 80, 50)), (0.0, 0.5));
    }

    /// A bright top and a dark textured bottom whose line wanders about `v = 0.5`.
    fn horizon(w: u32, h: u32) -> Linear {
        let mut img = Linear::new(w, h, [0.0; 3]);
        for (i, p) in img.data.iter_mut().enumerate() {
            let (u, v) = ((i as u32 % w) as f32 / w as f32, (i as u32 / w) as f32 / h as f32);
            let line = 0.5 + 0.03 * (u * 25.0).sin();
            let l = if v < line { 0.6 + 0.05 * u } else { 0.04 + 0.015 * ((u * 300.0).sin() * (v * 300.0).cos()) };
            *p = [l, l * 1.02, l * 0.95];
        }
        img
    }

    /// The horizon as a model might see it: coarse, and a cell off.
    fn horizon_raster(rw: u32, rh: u32) -> AiMaskRaster {
        raster(rw, rh, |x, y| {
            let (u, v) = ((x as f32 + 0.5) / rw as f32, (y as f32 + 0.5) / rh as f32);
            let line = 0.5 + 0.03 * (u * 25.0).sin() + 1.0 / rh as f32;
            if v < line { 255 } else { 0 }
        })
    }

    #[test]
    fn refining_pulls_a_coarse_edge_onto_the_photos_edge() {
        let (w, h) = (1600u32, 1200u32);
        let img = horizon(w, h);
        let map = FrameMap::identity(w, h);
        let store = rasters(horizon_raster(160, 120));
        let m = mask(vec![ai_shape("aa")]);
        let coarse = rasterize_with(&m, &map, w, h, Some(&store), None);
        let guide = Guide::new(&img, map.long_edge(), (0, 0));
        let fine = rasterize_with(&m, &map, w, h, Some(&store), Some(&guide));
        // Mean distance from the true line to where the weight crosses a half, over columns.
        let off = |wts: &[f32]| {
            let mut sum = 0f32;
            let cols: Vec<u32> = (10..w - 10).step_by(37).collect();
            for x in &cols {
                let line = (0.5 + 0.03 * (*x as f32 / w as f32 * 25.0).sin()) * h as f32;
                let cross = (0..h).find(|y| at(wts, w, *x, *y) < 0.5).unwrap_or(h);
                sum += (cross as f32 - line).abs();
            }
            sum / cols.len() as f32
        };
        let (before, after) = (off(&coarse), off(&fine));
        assert!(before > 8.0, "the raster is about a cell off: {before}");
        assert!(after < before / 3.0, "refined: {before} → {after} px");
        // And it stays clean away from the edge.
        assert_eq!((at(&fine, w, 800, 200), at(&fine, w, 800, 1000)), (1.0, 0.0));
    }

    /// A blue sky over rooflines: a warm block as bright as the sky, a dark block, a thin pole
    /// and a bright sign on it. Returns the image and where the sky truly is.
    fn skyline(w: u32, h: u32) -> (Linear, Vec<bool>) {
        let mut img = Linear::new(w, h, [0.0; 3]);
        let mut sky = vec![false; (w * h) as usize];
        let (wf, hf) = (w as f32, h as f32);
        for (i, p) in img.data.iter_mut().enumerate() {
            let (x, y) = ((i as u32 % w) as f32 + 0.5, (i as u32 / w) as f32 + 0.5);
            let (u, v) = (x / wf, y / hf);
            let roof = if u < 0.45 { 0.55 } else { 0.68 };
            let pole = (x - 0.62 * wf).abs() < 0.004 * wf && v > 0.3;
            let sign = (x - 0.62 * wf).abs() < 0.03 * wf && (0.26..0.31).contains(&v);
            let texture = 0.01 * ((x * 0.9).sin() * (y * 1.1).cos());
            *p = if sign {
                [0.85, 0.84, 0.8]
            } else if pole {
                [0.09, 0.09, 0.1]
            } else if v >= roof {
                // As bright as the sky (luminance ≈ 0.44), but warm; then a dark block.
                if u < 0.45 { [0.62 + texture, 0.42 + texture, 0.3] } else { [0.07 + texture, 0.06, 0.05] }
            } else {
                sky[i] = true;
                let g = 0.85 + 0.15 * (1.0 - v);
                [0.25 * g, 0.45 * g + texture, 0.85 * g]
            };
        }
        (img, sky)
    }

    /// The sky as a model sees it: each cell's share of sky, held back a cell from anything
    /// else (`erode`) and softened over `soften` cells around, so the pole, the sign and the
    /// cells along a roof come out well under 1.
    fn skyline_raster(sky: &[bool], w: u32, h: u32, rw: u32, rh: u32, erode: i32, soften: i32) -> AiMaskRaster {
        let (cw, ch) = (w / rw, h / rh);
        let share: Vec<f32> = (0..rw * rh)
            .map(|c| {
                let (cx, cy) = (c % rw, c / rw);
                let n = (0..ch).flat_map(|j| (0..cw).map(move |i| (i, j))).filter(|(i, j)| sky[((cy * ch + j) * w + cx * cw + i) as usize]).count();
                n as f32 / (cw * ch) as f32
            })
            .collect();
        let around = |src: &[f32], x: u32, y: u32, r: i32, f: fn(f32, f32) -> f32, start: f32| {
            (-r..=r).flat_map(|a| (-r..=r).map(move |b| (a, b))).fold(start, |acc, (dx, dy)| {
                let (nx, ny) = ((x as i32 + dx).clamp(0, rw as i32 - 1), (y as i32 + dy).clamp(0, rh as i32 - 1));
                f(acc, src[(ny as u32 * rw + nx as u32) as usize])
            })
        };
        let held: Vec<f32> = (0..rw * rh).map(|c| around(&share, c % rw, c / rw, erode, f32::min, 1.0)).collect();
        let n = ((2 * soften + 1) * (2 * soften + 1)) as f32;
        raster(rw, rh, |x, y| (around(&held, x, y, soften, |a, b| a + b, 0.0) / n * 255.0).round() as u8)
    }

    #[test]
    fn a_darkened_sky_leaves_no_fringe_along_roofs_poles_and_signs() {
        let (w, h) = (1600u32, 1200u32);
        let (img, sky) = skyline(w, h);
        let map = FrameMap::identity(w, h);
        let guide = Guide::new(&img, map.long_edge(), (0, 0));
        // Pixels within three cells of the other side, where a fringe would show.
        let near: Vec<usize> = (0..sky.len())
            .step_by(3)
            .filter(|&i| {
                let (x, y) = ((i as u32 % w) as i32, (i as u32 / w) as i32);
                (-30..=30).step_by(5).flat_map(|d| [(d, 0), (0, d)]).any(|(dx, dy)| {
                    let (nx, ny) = (x + dx, y + dy);
                    nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 && sky[(ny as u32 * w + nx as u32) as usize] != sky[i]
                })
            })
            .collect();
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let bad = |v: &[f32]| v.iter().filter(|e| **e > 0.25).count() as f32 / v.len() as f32;
        let mut failures = Vec::new();
        // The pole is about a raster cell and a third wide: only a model that holds the sky back
        // from it shows it at all, and what the raster doesn't show no filter can find.
        let pole = |i: usize| img.data[i] == [0.09, 0.09, 0.1];
        for (erode, soften) in [(0, 1), (0, 2), (1, 1), (1, 2)] {
            let store = rasters(skyline_raster(&sky, w, h, 160, 120, erode, soften));
            let wts = rasterize_with(&mask(vec![ai_shape("aa")]), &map, w, h, Some(&store), Some(&guide));
            let missed: Vec<f32> = near.iter().filter(|&&i| sky[i]).map(|&i| 1.0 - wts[i]).collect();
            let spilled: Vec<f32> = near.iter().filter(|&&i| !sky[i] && (!pole(i) || erode > 0)).map(|&i| wts[i]).collect();
            let report = format!("held back {erode}, soft {soften}: sky missed {:.3} (bad {:.3}), spilled {:.3} (bad {:.3})", mean(&missed), bad(&missed), mean(&spilled), bad(&spilled));
            println!("{report}");
            if mean(&missed) > 0.03 || bad(&missed) > 0.03 || mean(&spilled) > 0.03 || bad(&spilled) > 0.03 {
                failures.push(report);
            }
        }
        assert!(failures.is_empty(), "a fringe of sky left out or an edit spilling onto the roofs:\n{}", failures.join("\n"));
    }

    #[test]
    fn a_refined_mask_and_its_inverse_add_up() {
        let (w, h) = (800u32, 600u32);
        let img = horizon(w, h);
        let map = FrameMap::identity(w, h);
        let guide = Guide::new(&img, map.long_edge(), (0, 0));
        let one = horizon_raster(160, 120);
        let other = AiMaskRaster { weights: one.weights.iter().map(|v| 255 - v).collect(), ..one.clone() };
        let m = mask(vec![ai_shape("aa")]);
        let a = rasterize_with(&m, &map, w, h, Some(&rasters(one)), Some(&guide));
        let b = rasterize_with(&m, &map, w, h, Some(&rasters(other)), Some(&guide));
        let worst = a.iter().zip(&b).map(|(a, b)| (a + b - 1.0).abs()).fold(0.0f32, f32::max);
        assert!(worst < 0.01, "Subject and Background share no pixel twice: {worst}");
    }

    #[test]
    fn refining_nothing_or_everything_stays_exact() {
        let (w, h) = (300u32, 200u32);
        let img = horizon(w, h);
        let map = FrameMap::identity(w, h);
        let guide = Guide::new(&img, map.long_edge(), (0, 0));
        let m = mask(vec![ai_shape("aa")]);
        for (fill, want) in [(0u8, 0.0f32), (255, 1.0)] {
            let got = rasterize_with(&m, &map, w, h, Some(&rasters(raster(16, 12, |_, _| fill))), Some(&guide));
            assert!(got.iter().all(|v| *v == want), "fill {fill}");
        }
    }

    #[test]
    fn a_part_refines_as_the_same_pixels_of_the_whole() {
        let (w, h) = (1500u32, 1000u32);
        let img = horizon(w, h);
        let map = FrameMap::identity(w, h);
        let store = rasters(horizon_raster(160, 107));
        let m = mask(vec![ai_shape("aa")]);
        let guide = Guide::new(&img, map.long_edge(), (0, 0));
        let whole = rasterize_with(&m, &map, w, h, Some(&store), Some(&guide));
        let pad = refine_pad(&map);
        // Inside the frame, against its left edge, and into its far corner.
        for (x, y, vw, vh) in [(611u32, 397u32, 300u32, 200u32), (0, 420, 250, 150), (1200, 450, 300, 550)] {
            let (px, py) = (x.saturating_sub(pad), y.saturating_sub(pad));
            let (pw, ph) = ((x + vw + pad).min(w) - px, (y + vh + pad).min(h) - py);
            let part = geometry::crop(&img, px, py, pw, ph);
            let local = Guide::new(&part, map.long_edge(), (px, py));
            let got = rasterize_with(&m, &map.offset(-(px as f32), -(py as f32)), pw, ph, Some(&store), Some(&local));
            let mut worst = 0f32;
            for j in 0..vh {
                for i in 0..vw {
                    worst = worst.max((at(&got, pw, x - px + i, y - py + j) - at(&whole, w, x + i, y + j)).abs());
                }
            }
            assert!(worst <= 1.0 / 255.0, "at {x},{y}: worst {worst}");
        }
    }

    #[test]
    fn a_refined_edge_stays_put_when_the_crop_changes() {
        let (w, h) = (1500u32, 1000u32);
        let img = horizon(w, h);
        let store = rasters(horizon_raster(160, 107));
        let m = mask(vec![ai_shape("aa")]);
        let render = |r: &Recipe| {
            let frame = geometry::apply(&img, r);
            let map = FrameMap::new(w, h, r);
            let guide = Guide::new(&frame, map.long_edge(), (0, 0));
            (rasterize_with(&m, &map, frame.width, frame.height, Some(&store), Some(&guide)), map, frame.width, frame.height)
        };
        let (a, fa, wa, ha) = render(&Recipe { crop: Some(Crop { x: 0.1, y: 0.1, w: 0.8, h: 0.8 }), ..Recipe::identity() });
        let (b, fb, wb, hb) = render(&Recipe { crop: Some(Crop { x: 0.25, y: 0.3, w: 0.5, h: 0.4 }), ..Recipe::identity() });
        for p in [[0.4, 0.45], [0.5, 0.5], [0.6, 0.52], [0.45, 0.55], [0.55, 0.48]] {
            let (x, y) = (weight_at(&a, wa, ha, &fa, p), weight_at(&b, wb, hb, &fb, p));
            assert!((x - y).abs() < 0.05, "{p:?}: {x} vs {y}");
        }
    }

    #[test]
    fn no_guide_is_built_unless_a_mask_will_use_it() {
        let img = Linear::new(40, 30, [0.2; 3]);
        let store = rasters(raster(4, 3, |_, _| 255));
        let lit = LocalAdjust { exposure: 1.0, ..Default::default() };
        let ctx = Ctx { ai_rasters: Some(&store), frame_map: Some(FrameMap::identity(40, 30)), ..Ctx::default() };
        let needs = |shape: &str, adjust, ctx: &Ctx| guide_for(&img, &[Mask { adjust, ..mask(vec![ai_shape(shape)]) }], ctx).is_some();
        assert!(needs("aa", lit.clone(), &ctx));
        assert!(!needs("bb", lit.clone(), &ctx), "a raster the photo doesn't have");
        assert!(!needs("aa", LocalAdjust::default(), &ctx), "a mask with nothing to do");
        assert!(!needs("aa", lit.clone(), &Ctx { frame_map: None, ..ctx }), "no frame, no masks");
        assert!(!needs("aa", lit.clone(), &Ctx { ai_rasters: None, ..ctx }));
    }

    /// The overlay in Develop draws masks in TypeScript (`src/lib/mask-raster.ts`); it is held
    /// to these weights, as the slideshow canvas is held to `slideshow-golden.json`. An AI shape
    /// is the raster's own edge there: only Rust refines.
    #[test]
    fn writes_the_golden_cases_for_the_frontend() {
        let (bw, bh) = (48u32, 32u32);
        let lin = MaskShape::Linear { add: true, from: [0.2, 0.1], to: [0.7, 0.8] };
        let stroke = |add: bool| MaskShape::Brush { add, radius: 0.08, feather: 0.5, flow: 0.7, points: vec![[400, 800], [1800, 2600], [3600, 1400]] };
        let soft = MaskShape::Radial { add: true, center: [0.55, 0.45], radius: [0.3, 0.2], angle: 35.0, feather: 0.6, inside: false };
        let cases: Vec<(&str, Mask, Recipe)> = vec![
            ("radial-hard", mask(vec![radial([0.5, 0.5], [0.25, 0.3], 0.0, 0.0)]), Recipe::identity()),
            ("radial-soft-outside", mask(vec![soft.clone()]), Recipe::identity()),
            ("linear", mask(vec![lin.clone()]), Recipe::identity()),
            ("brush", mask(vec![stroke(true)]), Recipe::identity()),
            (
                "combined",
                Mask { invert: true, opacity: 0.6, ..mask(vec![stroke(true), MaskShape::Linear { add: false, from: [0.0, 0.0], to: [0.6, 0.0] }, radial([0.7, 0.7], [0.15, 0.2], 10.0, 0.3)]) },
                Recipe::identity(),
            ),
            (
                "turned-cropped",
                mask(vec![soft, stroke(true), stroke(false)]),
                Recipe { rotate: 90, flip_h: true, straighten: 6.0, crop: Some(Crop { x: 0.1, y: 0.05, w: 0.8, h: 0.85 }), ..Recipe::identity() },
            ),
            ("ai", mask(vec![ai_shape("aa")]), Recipe::identity()),
            (
                "ai-combined",
                Mask { invert: true, opacity: 0.7, ..mask(vec![lin.clone(), MaskShape::Ai { add: false, target: AiMaskKind::Person, instance: 1, raster: "aa".into() }, ai_shape("missing")]) },
                Recipe::identity(),
            ),
            ("ai-turned-cropped", mask(vec![ai_shape("aa")]), Recipe { rotate: 270, flip_v: true, straighten: -4.0, crop: Some(Crop { x: 0.1, y: 0.05, w: 0.8, h: 0.85 }), ..Recipe::identity() }),
        ];
        // A small raster with soft and hard edges, in the photo's aspect.
        let fixture = raster(12, 8, |x, y| [0u8, 40, 255, 255, 120, 0, 0, 200, 255, 90, 10, 0][((x + 2 * y) % 12) as usize]);
        let store = rasters(fixture.clone());
        let out: Vec<serde_json::Value> = cases
            .into_iter()
            .map(|(name, m, r)| {
                let (w, h) = r.output_size(bw, bh);
                let weights: Vec<f64> = rasterize_with(&m, &FrameMap::new(bw, bh, &r), w, h, Some(&store), None).into_iter().map(|v| (v as f64 * 10_000.0).round() / 10_000.0).collect();
                serde_json::json!({ "name": name, "base": [bw, bh], "recipe": r, "mask": m, "width": w, "height": h, "weights": weights })
            })
            .collect();
        let out = serde_json::json!({ "rasters": { "aa": { "width": fixture.width, "height": fixture.height, "weights": fixture.weights } }, "cases": out });
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/mask-golden.json");
        std::fs::write(path, serde_json::to_string(&out).unwrap() + "\n").unwrap();
    }

    #[test]
    fn a_feathered_stroke_along_the_edge_has_no_seam() {
        let (w, h) = (300, 200);
        let m = Mask { shapes: vec![MaskShape::Brush { add: true, radius: 0.1, feather: 1.0, flow: 1.0, points: vec![[0, 400], [0, 3600]] }], ..mask(vec![]) };
        let wts = rasterize(&m, &FrameMap::identity(w, h), w, h);
        for y in [40, 100, 160] {
            let (edge, inside) = (at(&wts, w, 0, y), at(&wts, w, 1, y));
            assert!(edge >= inside - 1e-6, "no darkening at the border: {edge} < {inside}");
            assert!((edge - inside).abs() < 0.02, "no seam: {edge} vs {inside}");
            assert!(edge > 0.5, "the stroke is there: {edge}");
        }
    }

    /// The heaviest weight in the column at `x`, and the column's sum (the line's thickness in rows).
    fn column(wts: &[f32], w: u32, h: u32, x: u32) -> (f32, f32) {
        let col: Vec<f32> = (0..h).map(|y| at(wts, w, x, y)).collect();
        (col.iter().cloned().fold(0.0, f32::max), col.iter().sum())
    }

    /// A one-pixel brush (§5): `radius` is half a pixel of a 6000 px long edge. However the line
    /// falls between pixel centres or onto the 4096 grid, it marks the render-size mask at least
    /// half, and a smaller preview of it fades rather than vanishes.
    #[test]
    fn a_one_pixel_stroke_marks_at_render_size_and_fades_in_a_preview() {
        for (width, at_least) in [(6000u32, 0.5f32), (1500, 1e-3)] {
            let h = width * 2 / 3;
            for dy in 0..8u16 {
                let m = Mask { shapes: vec![MaskShape::Brush { add: true, radius: 0.5 / 6000.0, feather: 0.0, flow: 1.0, points: vec![[800, 2040 + dy], [3200, 2040 + dy]] }], ..mask(vec![]) };
                let wts = rasterize(&m, &FrameMap::identity(width, h), width, h);
                let (peak, sum) = column(&wts, width, h, width / 2);
                assert!(peak >= at_least, "{width} px wide, line {dy}: the line's best row is {peak}");
                if width == 6000 {
                    assert!(sum < 3.0, "{width} px wide, line {dy}: still a thin line, {sum} rows thick");
                } else {
                    // A quarter of the line's width: a fraction of a row, thinnest when the line
                    // falls between two pixel centres.
                    assert!(sum > 0.02 && sum < 1.0, "{width} px wide, line {dy}: {sum} rows thick");
                }
            }
        }
    }
}
