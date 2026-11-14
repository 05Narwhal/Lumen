//! A mask's per-pixel tools past its Basic (Round 19): the curve, then HSL and grading, run
//! on what the mask's local Basic made, through the global stages' own per-pixel code, before
//! the mask's weight mixes the result in. Vibrance belongs to the local Basic
//! ([`LocalAdjust::basic`]), where the photo's runs too, so the order is the spec's: vibrance
//! → curve → HSL → grading. (`local.rs` is texture, clarity and dehaze: the stages that look
//! at a pixel's neighbours.)
//!
//! [`Tool`] is the rest: a mask's own sharpening, motion blur, glow, halation and grain, each
//! run at the photo's stage for it on the box the mask covers and mixed in by its weights.

use photos_core::edits::{Glow, Grain, Halation, LocalAdjust, MotionBlur, Sharpen};

use super::color::Colour;
use super::curve::Lut;
use super::{detail, effects, Ctx, Linear};

/// What a mask does to a pixel after its local Basic. `None` from [`Stack::new`] when it does
/// nothing, so a mask without these tools costs what it cost before them.
pub struct Stack {
    curve: Option<Lut>,
    colour: Option<Colour>,
}

impl Stack {
    pub fn new(a: &LocalAdjust) -> Option<Self> {
        let (curve, colour) = (Lut::new(&a.curve), Colour::new(&a.hsl, &a.grading));
        (curve.is_some() || colour.is_some()).then_some(Self { curve, colour })
    }

    #[inline]
    pub fn pixel(&self, mut p: [f32; 3]) -> [f32; 3] {
        if let Some(c) = &self.curve {
            p = c.pixel(p);
        }
        if let Some(c) = &self.colour {
            p = c.pixel(p);
        }
        p
    }
}

/// A mask's area tool: one that reads a pixel's neighbours or the frame, so it runs on a part of
/// the frame at its own stage rather than per pixel at the mask stage.
#[derive(Debug, Clone, Copy)]
pub enum Tool {
    Sharpen(Sharpen),
    MotionBlur(MotionBlur),
    Glow(Glow),
    Halation(Halation),
    Grain(Grain),
}

impl Tool {
    /// The mask's sharpening when it has radius, detail and masking of its own; without them
    /// its amount is added to the photo's (see `masks::apply`).
    pub fn sharpen(a: &LocalAdjust) -> Option<Self> {
        let s = a.sharpen.as_ref().filter(|_| a.sharpness != 0.0)?;
        Some(Self::Sharpen(Sharpen { amount: a.sharpness, radius: s.radius, detail: s.detail, masking: s.masking }))
    }

    pub fn motion_blur(a: &LocalAdjust) -> Option<Self> {
        (a.motion_blur.amount > 0.0).then_some(Self::MotionBlur(a.motion_blur))
    }

    pub fn glow(a: &LocalAdjust) -> Option<Self> {
        (a.glow.amount > 0.0).then_some(Self::Glow(a.glow))
    }

    pub fn halation(a: &LocalAdjust) -> Option<Self> {
        (a.halation.amount > 0.0).then_some(Self::Halation(a.halation))
    }

    pub fn grain(a: &LocalAdjust) -> Option<Self> {
        (a.grain.amount != 0.0).then_some(Self::Grain(a.grain))
    }

    /// Whether the mask has any area tool, so the mask stage keeps its weights.
    pub fn any(a: &LocalAdjust) -> bool {
        [Self::sharpen, Self::motion_blur, Self::glow, Self::halation, Self::grain].iter().any(|t| t(a).is_some())
    }

    /// How far round a pixel the tool reads, in render pixels, for a frame whose long edge is
    /// `long`: the border a part needs to render its inside as the whole frame would.
    pub fn reach(&self, ctx: &Ctx, long: f32) -> f32 {
        match self {
            Self::Sharpen(s) => 3.0 * s.radius * ctx.scale + 1.0,
            Self::MotionBlur(m) => 0.5 * effects::motion_blur_length(m, long) + 1.0,
            Self::Glow(g) => 3.0 * effects::glow_sigma(g, long),
            Self::Halation(h) => 3.0 * effects::halation_sigma(h, long),
            Self::Grain(_) => 1.0,
        }
    }

    /// Whether the tool spreads pixels (a streak, a bloom), so inside a mask it must take its
    /// source only from the mask's own pixels: it runs through [`Self::run_weighted`].
    pub fn weighted(&self) -> bool {
        matches!(self, Self::MotionBlur(_) | Self::Glow(_) | Self::Halation(_))
    }

    /// [`Self::run`] with the mask's `weights` (one per pixel of `img`) as the source weight.
    /// A tool that is not [`Self::weighted`] ignores them.
    pub fn run_weighted(&self, img: &mut Linear, ctx: &Ctx, weights: &[f32]) {
        match self {
            Self::MotionBlur(m) => effects::motion_blur_weighted(img, m, ctx, weights),
            Self::Glow(g) => effects::glow_weighted(img, g, ctx, Some(weights)),
            Self::Halation(h) => effects::halation_weighted(img, h, ctx, Some(weights)),
            _ => self.run(img, ctx),
        }
    }

    pub fn run(&self, img: &mut Linear, ctx: &Ctx) {
        match self {
            Self::Sharpen(s) => detail::sharpen(img, s, ctx),
            Self::MotionBlur(m) => effects::motion_blur(img, m, ctx),
            Self::Glow(g) => effects::glow(img, g, ctx),
            Self::Halation(h) => effects::halation(img, h, ctx),
            Self::Grain(g) => effects::grain(img, g, ctx),
        }
    }
}
