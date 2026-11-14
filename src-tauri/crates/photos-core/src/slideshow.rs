//! Slideshows (spec §14): a declarative document that the editor previews and the exporter
//! renders. The timing and motion maths live here once; the preview's TypeScript copy is
//! held to them by a golden file this module's tests write (`src/lib/slideshow-golden.json`).

use std::collections::HashMap;
use crate::beatgrid::BeatGrid;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CoreError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SlideshowDoc {
    pub version: u32,
    /// Width and height in pixels, both even.
    pub resolution: [u32; 2],
    pub fps: u32,
    pub audio: Option<AudioTrack>,
    pub clips: Vec<Clip>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AudioTrack {
    /// Absolute, or relative to the library root (`music/…`, Phase 6c).
    pub path: String,
    pub fade_in_ms: u32,
    pub fade_out_ms: u32,
    pub gain_db: f32,
    /// How much of the track's head to drop, so the show can start on a beat rather than
    /// wherever the recording happens to begin. Absent from documents saved before 6d.
    #[serde(default)]
    pub start_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Clip {
    #[ts(type = "number")]
    pub asset_id: i64,
    /// The other photos in a grid; empty for `fill` and `blur`.
    #[serde(default)]
    #[ts(type = "number[]")]
    pub extra: Vec<i64>,
    /// How this clip's photos are laid out in the frame.
    #[serde(default)]
    pub frame: FrameKind,
    pub duration_ms: u32,
    /// Into the next clip; the last clip's is unused.
    pub transition: Transition,
    pub motion: Motion,
    /// The part of a video this clip plays (Round 6); absent for a photograph. A video clip is
    /// `fill` or `blur`, alone, still (no Ken Burns) and muted under the music.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub video: Option<VideoSpan>,
}

/// Where in a video a clip plays from and to, in the video's own milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VideoSpan {
    pub in_ms: u32,
    pub out_ms: u32,
}

impl VideoSpan {
    pub fn len_ms(&self) -> u32 {
        self.out_ms.saturating_sub(self.in_ms)
    }
}

impl Clip {
    /// Every photo this clip shows, in painting order.
    pub fn assets(&self) -> Vec<i64> {
        std::iter::once(self.asset_id).chain(self.extra.iter().copied()).collect()
    }
}

/// A gutter between grid cells, as a fraction of the frame's width.
pub const GRID_GUTTER: f32 = 0.015;
/// Below this aspect a portrait is narrow enough that three fill a 16:9 frame.
const THREE_UP_ASPECT: f32 = 0.62;
/// How wide a `Feature`'s hero is, as a fraction of the frame.
const FEATURE_WIDTH: f32 = 0.62;
/// Wider than this and a photograph is a panorama: it wants a band, not a frame.
const PANORAMA_ASPECT: f32 = 2.2;
/// At or above this a photograph is not a portrait.
const PORTRAIT_ASPECT: f32 = 0.95;

/// How a clip's photos meet the frame. A landscape photo is happy filling it; a portrait
/// one is not, and is shown whole instead — over a blurred copy of itself, or beside its
/// neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum FrameKind {
    /// The photo covers the frame; anything outside it is cropped.
    #[default]
    Fill,
    /// The whole photo over a blurred, covering copy of itself.
    Blur,
    /// Two or three photos side by side over a blurred backdrop.
    Grid,
    /// Four photos, two by two.
    Quad,
    /// One photo given most of the frame, with two stacked beside it.
    Feature,
    /// Two photos one above the other, full width — the panorama layout.
    Stack,
}

impl FrameKind {
    /// How many photographs this layout shows.
    pub fn capacity(&self) -> usize {
        match self {
            Self::Fill | Self::Blur => 1,
            Self::Grid | Self::Feature => 3,
            Self::Quad => 4,
            Self::Stack => 2,
        }
    }

    /// Whether this layout shows more than one photograph.
    pub fn is_multi(&self) -> bool {
        !matches!(self, Self::Fill | Self::Blur)
    }
}

/// A rectangle in frame coordinates: 0..1 is the frame, and a rectangle may exceed it —
/// a covering photo is cropped by the frame edge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One photo drawn into one rectangle. `blurred` is the backdrop behind a fitted photo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Placement {
    /// Index into the clip's `assets()`.
    #[ts(type = "number")]
    pub photo: usize,
    pub rect: Rect,
    pub blurred: bool,
}

/// `aspect` is width / height, relative to the box. Centres a rectangle of that shape
/// inside `(w, h)`, either covering it or fitting within it.
fn placed(aspect: f32, w: f32, h: f32, x: f32, y: f32, cover: bool) -> Rect {
    let box_aspect = w / h;
    let (rw, rh) = if (aspect > box_aspect) == cover { (h * aspect, h) } else { (w, w / aspect) };
    Rect { x: x + (w - rw) / 2.0, y: y + (h - rh) / 2.0, w: rw, h: rh }
}

/// What a clip paints, in order. `aspects` is each photo's width / height and
/// `frame_aspect` is the video's; the rectangles returned are fractions of the frame.
pub fn frame_layout(kind: FrameKind, aspects: &[f32], frame_aspect: f32) -> Vec<Placement> {
    let sane = |a: f32| if a.is_finite() && a > 0.0 { a } else { 1.0 };
    let frame_aspect = sane(frame_aspect);
    let first = aspects.first().copied().map(sane).unwrap_or(1.0);
    // The frame is one unit wide and one tall, so a photo's shape is measured against it.
    let cover = |a: f32| placed(a / frame_aspect, 1.0, 1.0, 0.0, 0.0, true);
    let fit = |a: f32| placed(a / frame_aspect, 1.0, 1.0, 0.0, 0.0, false);
    // Every layout that shows more than one photograph is the same idea: a blurred copy of
    // the first covering the frame, then each photograph fitted whole inside its own box.
    let cells = |boxes: &[(f32, f32, f32, f32)]| -> Vec<Placement> {
        let mut out = vec![Placement { photo: 0, rect: cover(first), blurred: true }];
        for (i, (x, y, w, h)) in boxes.iter().enumerate().take(aspects.len()) {
            out.push(Placement {
                photo: i,
                rect: placed(sane(aspects[i]) / frame_aspect, *w, *h, *x, *y, false),
                blurred: false,
            });
        }
        out
    };
    let half = (1.0 - GRID_GUTTER) / 2.0;

    match kind {
        FrameKind::Fill => vec![Placement { photo: 0, rect: cover(first), blurred: false }],
        FrameKind::Blur => vec![
            Placement { photo: 0, rect: cover(first), blurred: true },
            Placement { photo: 0, rect: fit(first), blurred: false },
        ],
        FrameKind::Grid => {
            let n = aspects.len().clamp(1, 3);
            let column = (1.0 - GRID_GUTTER * (n as f32 - 1.0)) / n as f32;
            let boxes: Vec<(f32, f32, f32, f32)> =
                (0..n).map(|i| (i as f32 * (column + GRID_GUTTER), 0.0, column, 1.0)).collect();
            cells(&boxes)
        }
        FrameKind::Quad => {
            let (x1, y1) = (half + GRID_GUTTER, half + GRID_GUTTER);
            cells(&[(0.0, 0.0, half, half), (x1, 0.0, half, half), (0.0, y1, half, half), (x1, y1, half, half)])
        }
        FrameKind::Feature => {
            let side = 1.0 - FEATURE_WIDTH - GRID_GUTTER;
            let x = FEATURE_WIDTH + GRID_GUTTER;
            cells(&[(0.0, 0.0, FEATURE_WIDTH, 1.0), (x, 0.0, side, half), (x, half + GRID_GUTTER, side, half)])
        }
        FrameKind::Stack => cells(&[(0.0, 0.0, 1.0, half), (0.0, half + GRID_GUTTER, 1.0, half)]),
    }
}

/// What the builder knows about a photo: its id and its shape — and, for a video, the part
/// a show plays.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PhotoShape {
    pub id: i64,
    /// Width / height, as the photo is displayed.
    pub aspect: f32,
    pub video: Option<VideoSpan>,
}

impl PhotoShape {
    pub fn portrait(&self) -> bool {
        self.aspect < PORTRAIT_ASPECT
    }
}

/// How a show frames its photographs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum LayoutStyle {
    /// Landscapes fill the frame, portraits pair up, a lone portrait gets a backdrop.
    #[default]
    Smart,
    Fill,
    Blur,
    Grid,
    /// Smart, plus a transition picked per clip.
    Random,
}

/// And what it chooses from when a grid is involved. A grid is several photographs held
/// still, so it wants a join that arrives in pieces: slices and wipes read as the tiles
/// landing, where a zoom or a circle fights the layout it is trying to reveal.
const GRID_SHUFFLE: &[TransitionKind] = &[
    TransitionKind::Slices,
    TransitionKind::VSlices,
    TransitionKind::Corner,
    TransitionKind::VertOpen,
    TransitionKind::HorzOpen,
    TransitionKind::Crossfade,
];

/// A deterministic stream seeded by the photos themselves, so the same set always makes
/// the same show and a test can assert on it.
fn seeded(photos: &[PhotoShape]) -> impl FnMut() -> u32 {
    let mut state = photos.iter().fold(0x9e37_79b9u32, |acc, p| acc.wrapping_mul(31).wrapping_add(p.id as u32));
    move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state >> 16
    }
}


/// A gap longer than this starts a new moment. Twelve minutes: long enough that two frames
/// of the same scene stay in one moment, short enough that the order still reads as the day.
const MOMENT_GAP_S: i64 = 12 * 60;
/// How far apart two perceptual hashes must be to count as different pictures.
const NEAR_DUPLICATE_BITS: u32 = 6;

/// A photograph as the arranger sees it: when it was taken, what shape it is, how good it
/// is and what it looks like.
#[derive(Debug, Clone, PartialEq)]
pub struct Orderable {
    pub id: i64,
    pub aspect: f32,
    pub captured_at: i64,
    /// 0..1, higher is better. `quality_score` builds it from the catalogue.
    pub score: f32,
    pub phash: Option<u64>,
    /// Its CLIP embedding, when the model has seen it (Phase 7).
    pub vec: Option<crate::embedding::Vector>,
    /// Its dominant colour, from the thumbnail.
    pub color: Option<[u8; 3]>,
    /// The part a show plays, when it is a video (Round 6).
    pub video: Option<VideoSpan>,
}

impl From<&Orderable> for PhotoShape {
    fn from(o: &Orderable) -> Self {
        PhotoShape { id: o.id, aspect: o.aspect, video: o.video }
    }
}

/// A photograph's quality in 0..1, from what the catalogue knows without any model: the
/// user's own judgement first, sharpness only to break the ties.
pub fn quality_score(favorite: bool, rating: i64, sharpness: Option<f32>, sharpest: f32) -> f32 {
    let fav = if favorite { 3.0 } else { 0.0 };
    let stars = rating.clamp(0, 5) as f32;
    let sharp = match sharpness {
        Some(s) if sharpest > 0.0 => (s / sharpest).clamp(0.0, 1.0),
        _ => 0.0,
    };
    ((fav + stars + sharp) / 9.0).clamp(0.0, 1.0)
}

fn near_duplicate(a: Option<u64>, b: Option<u64>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => (a ^ b).count_ones() <= NEAR_DUPLICATE_BITS,
        _ => false,
    }
}

/// Time order at the large scale, taste within a moment.
///
/// A *moment* is a run of photographs with no gap longer than `MOMENT_GAP_S`. Between
/// moments the order is the truth of the day and is left alone; inside one it is a matter of
/// taste, so three things are improved there: the best photograph leads, two near-identical
/// frames are never adjacent, and photographs of the same shape are emitted together so the
/// layouts that hold several of them fill up.
pub fn arrange(photos: &[Orderable]) -> Vec<Orderable> {
    let mut sorted = photos.to_vec();
    sorted.sort_by(|a, b| a.captured_at.cmp(&b.captured_at).then(a.id.cmp(&b.id)));

    let mut out: Vec<Orderable> = Vec::with_capacity(sorted.len());
    let mut start = 0;
    while start < sorted.len() {
        let mut end = start + 1;
        while end < sorted.len() && sorted[end].captured_at - sorted[end - 1].captured_at <= MOMENT_GAP_S {
            end += 1;
        }
        out.extend(arrange_moment(&sorted[start..end]));
        start = end;
    }
    out
}

/// One moment, where the order is ours to choose. Greedy: the best photograph first, then
/// repeatedly the best of what is left that is neither a near-duplicate of the one just
/// placed nor a break in a run of portraits.
fn arrange_moment(moment: &[Orderable]) -> Vec<Orderable> {
    let mut left: Vec<Orderable> = moment.to_vec();
    // Best first, and by id where two are equally good, so the result never depends on the
    // order the caller happened to hand them over in.
    left.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));

    let mut out: Vec<Orderable> = Vec::with_capacity(left.len());
    while !left.is_empty() {
        let pick = match out.last() {
            None => 0,
            Some(previous) => {
                let want_portrait = previous.aspect < PORTRAIT_ASPECT;
                let usable = |c: &Orderable| !near_duplicate(c.phash, previous.phash);
                left.iter()
                    .position(|c| usable(c) && (c.aspect < PORTRAIT_ASPECT) == want_portrait)
                    // Nothing of the same shape: anything that is not a repeat will do.
                    .or_else(|| left.iter().position(usable))
                    // Everything left repeats it; take the best and move on.
                    .unwrap_or(0)
            }
        };
        out.push(left.remove(pick));
    }
    out
}

/// The same show, cut in time with music of `bpm` whose first beat is at `first_beat_ms`:
/// `snap_to_beat_grid` on an even grid. Kept for callers that know only a tempo.
///
/// A tempo outside what music does, or none at all, returns the document untouched.
pub fn snap_to_beats(doc: &SlideshowDoc, bpm: f32, first_beat_ms: u32) -> SlideshowDoc {
    if !bpm.is_finite() || !SNAPPABLE_BPM.contains(&bpm) {
        return doc.clone();
    }
    // Enough beats for every clip at its longest, and then some.
    let until = first_beat_ms as u64 + doc.clips.len() as u64 * MAX_CLIP_MS as u64 + 60_000;
    snap_to_beat_grid(doc, &BeatGrid::even(bpm, first_beat_ms, until.min(u32::MAX as u64) as u32))
}

/// The show cut to a list of beats (ML design §6).
///
/// What a viewer reads as the cut is the moment the next clip starts, which the timeline
/// puts at `start_i + dur_i − overlap_i`. So each cut is chosen as an *absolute* beat time —
/// the beat nearest where the clip wanted to end, at least `MIN_BEATS_PER_CLIP` on, and a
/// downbeat when one lies within a beat of that — and the durations are rebuilt around
/// those cuts, keeping each clip's overlap, so the transition straddles the beat. Because
/// the cuts are beats from the list rather than multiples of one tempo, a tempo that moves
/// is followed and rounding never accumulates. The last clip is cut too, so the show ends
/// on a beat. The track is trimmed so its first beat is the show's zero — the first beat at
/// or after where the show already starts in the track (Round 6), so a show that plays the
/// chorus still plays the chorus once it is cut to the beat.
///
/// A clip the user has trimmed to an extreme keeps its limits and may fall off the beat;
/// their edit wins over the arithmetic.
pub fn snap_to_beat_grid(doc: &SlideshowDoc, grid: &BeatGrid) -> SlideshowDoc {
    let from = doc.audio.as_ref().map_or(0, |a| a.start_ms);
    let beats = match grid.beats_ms.iter().position(|b| *b >= from) {
        Some(first) => &grid.beats_ms[first..],
        None => &grid.beats_ms[..0],
    };
    if beats.len() < 2 || doc.clips.is_empty() {
        return doc.clone();
    }
    let origin = beats[0];
    let rel: Vec<i64> = beats.iter().map(|b| (*b - origin) as i64).collect();
    let down: std::collections::HashSet<i64> =
        grid.downbeats_ms.iter().filter(|d| **d >= origin).map(|d| (*d - origin) as i64).collect();
    let beat = grid.beat_ms() as i64;
    let overlaps = timeline(doc).overlaps;
    let mut out = doc.clone();
    let (mut cut_index, mut cut_at) = (0usize, 0i64);
    for (i, clip) in out.clips.iter_mut().enumerate() {
        let overlap = overlaps.get(i).copied().unwrap_or(0) as i64;
        let target = cut_at + (clip.duration_ms as i64 - overlap).max(0);
        let first_allowed = cut_index + MIN_BEATS_PER_CLIP as usize;
        if first_allowed >= rel.len() {
            // The music ran out; the rest of the show keeps its own timing.
            break;
        }
        let distance = |k: &usize| (rel[*k] - target).abs();
        let nearest = (first_allowed..rel.len()).min_by_key(distance).unwrap_or(first_allowed);
        let chosen = (first_allowed..rel.len())
            .filter(|k| down.contains(&rel[*k]) && distance(k) <= beat)
            .min_by_key(distance)
            .unwrap_or(nearest);
        clip.duration_ms = (rel[chosen] - cut_at + overlap).clamp(MIN_CLIP_MS as i64, MAX_CLIP_MS as i64) as u32;
        cut_index = chosen;
        cut_at = rel[chosen];
    }
    if let Some(a) = out.audio.as_mut() {
        a.start_ms = origin;
    }
    out
}

/// A transition as long as half a beat, within what still reads as a transition.
pub fn beat_transition_ms(grid: &BeatGrid) -> u32 {
    ((grid.beat_ms() / 2.0).round() as u32).clamp(250, 900)
}

/// A show of `photos`, framed by `style`: four seconds a picture, 600 ms transitions and
/// alternating Ken Burns on anything that is not a grid. Photographs are grouped in the order
/// given, by shape alone; `build_arranged` groups them by what they show.
pub fn build(photos: &[PhotoShape], style: LayoutStyle) -> SlideshowDoc {
    let mut groups: Vec<(FrameKind, Vec<i64>)> = Vec::new();
    let mut i = 0;
    while i < photos.len() {
        let rest = &photos[i..];
        let (frame, take) = match style {
            LayoutStyle::Fill => (FrameKind::Fill, 1),
            LayoutStyle::Blur => (FrameKind::Blur, 1),
            // A video plays alone whatever the style asks (Round 6).
            _ if rest[0].video.is_some() => choose_frame(rest),
            LayoutStyle::Grid => {
                // This style always groups, even where Smart would show one photo whole — but
                // never across a video.
                let photos_here = rest.iter().position(|p| p.video.is_some()).unwrap_or(rest.len());
                let (frame, take) = choose_frame(rest);
                match group_frame(rest) {
                    Some(f) => f,
                    None if frame.is_multi() => (frame, take),
                    None => (FrameKind::Grid, 2.min(photos_here).max(1)),
                }
            }
            LayoutStyle::Smart | LayoutStyle::Random => choose_frame(rest),
        };
        let n = take.min(rest.len()).max(1);
        groups.push((frame, rest[..n].iter().map(|p| p.id).collect()));
        i += n;
    }
    assemble(&groups, style, photos, &HashMap::new())
}

/// Clips for groups already framed. `seed` seeds the transition stream, so the same
/// photographs always make the same show.
fn assemble(groups: &[(FrameKind, Vec<i64>)], style: LayoutStyle, seed: &[PhotoShape], photos: &HashMap<i64, Orderable>) -> SlideshowDoc {
    let pans = [(0.02, 0.01), (-0.02, 0.01), (0.015, -0.01), (-0.01, -0.015)];
    let mut next = seeded(seed);
    let spans: HashMap<i64, VideoSpan> = seed.iter().filter_map(|p| p.video.map(|v| (p.id, v))).collect();
    let mut clips: Vec<Clip> = Vec::new();
    for (frame, ids) in groups {
        let frame = *frame;
        let (dx, dy) = pans[clips.len() % pans.len()];
        let kind = TransitionKind::Crossfade;
        let video = spans.get(&ids[0]).copied();
        clips.push(Clip {
            asset_id: ids[0],
            extra: if video.is_some() { Vec::new() } else { ids[1..].to_vec() },
            frame,
            // A video plays for as long as its best part lasts.
            duration_ms: video.map_or(DEFAULT_CLIP_MS, |v| v.len_ms().clamp(MIN_CLIP_MS, MAX_CLIP_MS)),
            transition: Transition { kind, duration_ms: DEFAULT_TRANSITION_MS, auto: true },
            // A grid holds several photographs still, and a video moves by itself; only a
            // single photograph drifts.
            motion: match (frame, clips.len().is_multiple_of(2)) {
                _ if video.is_some() => Motion::none(),
                (FrameKind::Grid, _) => Motion::none(),
                (_, true) => Motion::zoom_in(dx, dy),
                (_, false) => Motion::zoom_out(dx, dy),
            },
            video,
        });
    }
    let doc = SlideshowDoc { version: 1, resolution: [1920, 1080], fps: 30, audio: None, clips };
    if style == LayoutStyle::Random {
        choose_joins(&doc, photos, None, 0.0, &mut next)
    } else {
        doc
    }
}

/// What a join between two clips looks like, for `choose_transition`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JoinContext {
    /// How far the two clips' leading photographs show the same thing, when both have embeddings.
    pub similar: Option<f32>,
    /// The time gap between them is longer than a moment.
    pub new_moment: bool,
    pub grid_either_side: bool,
    /// The cut lands on a downbeat.
    pub on_downbeat: bool,
    /// How busy the music is, 0..1.
    pub energy: f32,
    /// How bright each side is, 0..1, from the dominant colours.
    pub luma_out: Option<f32>,
    pub luma_in: Option<f32>,
}

/// The join between two clips, scored rather than drawn (ML design §9). Every kind starts at
/// zero; each rule that applies adds or takes one; the best wins, ties broken by `tie` — the
/// show's seeded stream — so the same show always gets the same transitions. A hard cut and
/// the plain fades are only ever chosen for a reason, never as a tie-break.
pub fn choose_transition(ctx: &JoinContext, recent: &[TransitionKind], tie: u32) -> TransitionKind {
    use TransitionKind as T;
    let mut score: Vec<(TransitionKind, f32)> = TransitionKind::ALL.iter().map(|k| (*k, 0.0)).collect();
    let mut bump = |kinds: &[TransitionKind], by: f32| {
        for (k, s) in score.iter_mut() {
            if kinds.contains(k) {
                *s += by;
            }
        }
    };
    if ctx.similar.is_some_and(|s| s >= 0.85) {
        bump(&[T::Crossfade], 1.0);
        bump(&[T::Slide, T::SlideUp, T::SlideDown, T::Wipe, T::WipeUp], -1.0);
    }
    if ctx.new_moment {
        bump(&[T::FadeBlack], 1.0);
        bump(&[T::Slices, T::VSlices], -1.0);
    }
    if ctx.grid_either_side {
        bump(GRID_SHUFFLE, 1.0);
        bump(&[T::Circle, T::Zoom], -1.0);
    }
    if ctx.on_downbeat && ctx.energy > 0.6 {
        bump(&[T::None, T::Zoom, T::Slide], 1.0);
        bump(&[T::Crossfade], -1.0);
    }
    if let (Some(out), Some(into)) = (ctx.luma_out, ctx.luma_in) {
        if into - out > 0.4 {
            bump(&[T::FadeWhite], 1.0);
        } else if out - into > 0.4 {
            bump(&[T::FadeBlack], 1.0);
        }
    }
    for r in recent.iter().rev().take(2) {
        bump(&[*r], -1.0);
    }
    let for_a_reason = |k: TransitionKind| !matches!(k, T::None | T::FadeBlack | T::FadeWhite);
    let best = score.iter().filter(|s| s.1 > 0.0 || for_a_reason(s.0)).map(|s| s.1).fold(f32::MIN, f32::max);
    let tied: Vec<TransitionKind> = score.iter().filter(|s| s.1 == best && (best > 0.0 || for_a_reason(s.0))).map(|s| s.0).collect();
    if tied.is_empty() {
        return T::Crossfade;
    }
    tied[tie as usize % tied.len()]
}

fn luma(c: [u8; 3]) -> f32 {
    (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0
}

/// Every automatic join of `doc` chosen again from what it joins: the photographs on either
/// side (from `photos`, when known) and, after a beat sync, the downbeats and the music's
/// energy. A join the user picked (`auto == false`) is left exactly as it is.
fn choose_joins(doc: &SlideshowDoc, photos: &HashMap<i64, Orderable>, grid: Option<&BeatGrid>, energy: f32, next: &mut impl FnMut() -> u32) -> SlideshowDoc {
    let starts = timeline(doc).starts;
    let start_ms = doc.audio.as_ref().map(|a| a.start_ms).unwrap_or(0);
    let mut out = doc.clone();
    let mut recent: Vec<TransitionKind> = Vec::new();
    for i in 0..out.clips.len() {
        let tie = next();
        if i + 1 < out.clips.len() && out.clips[i].transition.auto {
            let (a, b) = (&doc.clips[i], &doc.clips[i + 1]);
            let (pa, pb) = (photos.get(&a.asset_id), photos.get(&b.asset_id));
            let cut = starts.get(i + 1).map(|s| s + start_ms);
            let ctx = JoinContext {
                similar: match (pa, pb) {
                    (Some(x), Some(y)) if x.vec.is_some() && y.vec.is_some() => Some(similar(x, y)),
                    _ => None,
                },
                new_moment: matches!((pa, pb), (Some(x), Some(y)) if (y.captured_at - x.captured_at).abs() > MOMENT_GAP_S),
                grid_either_side: a.frame.is_multi() || b.frame.is_multi(),
                on_downbeat: match (grid, cut) {
                    (Some(g), Some(t)) => g.downbeats_ms.iter().any(|d| d.abs_diff(t) <= 30),
                    _ => false,
                },
                energy,
                luma_out: pa.and_then(|x| x.color).map(luma),
                luma_in: pb.and_then(|x| x.color).map(luma),
            };
            let t = &mut out.clips[i].transition;
            t.kind = choose_transition(&ctx, &recent, tie);
            if let Some(g) = grid {
                t.duration_ms = beat_transition_ms(g);
            }
        }
        recent.push(out.clips[i].transition.kind);
    }
    out
}

/// The joins of a show chosen again after it has been cut to the beat, when downbeats and
/// energy (the track's onsets per second over 6) are known. Only automatic joins change.
pub fn retransition(doc: &SlideshowDoc, photos: &HashMap<i64, Orderable>, grid: Option<&BeatGrid>, energy: f32) -> SlideshowDoc {
    let shapes: Vec<PhotoShape> = doc.clips.iter().map(|c| PhotoShape { id: c.asset_id, aspect: 1.0, video: None }).collect();
    let mut next = seeded(&shapes);
    choose_joins(doc, photos, grid, energy, &mut next)
}

/// A moment's photographs, framed together.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub frame: FrameKind,
    pub photos: Vec<Orderable>,
}

/// Embeddings this close are the same picture: two of them side by side look like a mistake.
const TWIN_COS: f32 = 0.96;
/// A photograph must beat the moment's median score by this much to be a hero.
const HERO_MARGIN: f32 = 0.1;

fn shape_class(o: &Orderable) -> u8 {
    if o.aspect >= PANORAMA_ASPECT {
        2
    } else if o.aspect < PORTRAIT_ASPECT {
        0
    } else {
        1
    }
}

/// How far two photographs show the same thing, 0..1; 0.5 when either has no embedding.
pub fn similar(a: &Orderable, b: &Orderable) -> f32 {
    match (&a.vec, &b.vec) {
        (Some(x), Some(y)) => ((crate::embedding::cosine(x, y) - 0.70) / 0.25).clamp(0.0, 1.0),
        _ => 0.5,
    }
}

fn srgb_to_lab(c: [u8; 3]) -> [f32; 3] {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// How well two photographs sit in one frame: `0.5·similar + 0.3·shape_fit + 0.2·harmony`.
/// Harmony is 1 − the CIE76 distance between their dominant colours over 100.
pub fn compat(a: &Orderable, b: &Orderable) -> f32 {
    let shape_fit = if shape_class(a) == shape_class(b) { 1.0 } else { 0.0 };
    let harmony = match (a.color, b.color) {
        (Some(x), Some(y)) => {
            let (p, q) = (srgb_to_lab(x), srgb_to_lab(y));
            let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
            1.0 - (d / 100.0).clamp(0.0, 1.0)
        }
        _ => 0.5,
    };
    0.5 * similar(a, b) + 0.3 * shape_fit + 0.2 * harmony
}

fn twins(a: &Orderable, b: &Orderable) -> bool {
    near_duplicate(a.phash, b.phash)
        || matches!((&a.vec, &b.vec), (Some(x), Some(y)) if crate::embedding::cosine(x, y) >= TWIN_COS)
}

/// The photographs that stand out enough to be shown alone: in Smart and Random, up to a
/// quarter of a moment of four or more, and only those clearly better than its median. A
/// library with no ratings has every score equal, and so no heroes.
fn heroes(moment: &[Orderable], style: LayoutStyle) -> std::collections::HashSet<i64> {
    if !matches!(style, LayoutStyle::Smart | LayoutStyle::Random) || moment.len() < 4 {
        return Default::default();
    }
    let mut scores: Vec<f32> = moment.iter().map(|o| o.score).collect();
    scores.sort_by(f32::total_cmp);
    let median = scores[scores.len() / 2];
    let mut by_score: Vec<&Orderable> = moment.iter().filter(|o| o.score > median + HERO_MARGIN).collect();
    by_score.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    by_score.iter().take((moment.len() / 4).max(1)).map(|o| o.id).collect()
}

/// One moment grouped by what its photographs show (ML design §9). Heroes stand alone; the
/// rest are seeded best-first and joined by the photographs most compatible with them —
/// same scene, same shape, colours that sit together — never a near-twin. Smart groups
/// only what one frame would crop badly (portraits, panoramas); Grid groups everything.
/// The frame then decides how many a group really takes. Groups come back in the order of
/// their earliest photograph, so the moment still reads in time.
pub fn group_moment(moment: &[Orderable], style: LayoutStyle) -> Vec<Group> {
    let single = |o: &Orderable| {
        let frame = match style {
            LayoutStyle::Fill => FrameKind::Fill,
            LayoutStyle::Blur => FrameKind::Blur,
            _ => choose_frame(&[PhotoShape::from(o)]).0,
        };
        Group { frame, photos: vec![o.clone()] }
    };
    if matches!(style, LayoutStyle::Fill | LayoutStyle::Blur) {
        return moment.iter().map(single).collect();
    }
    let grid_style = style == LayoutStyle::Grid;
    let mut heroes = heroes(moment, style);
    // A video is never grouped: it plays alone.
    heroes.extend(moment.iter().filter(|o| o.video.is_some()).map(|o| o.id));
    let mut groups: Vec<Group> = moment.iter().filter(|o| heroes.contains(&o.id)).map(single).collect();
    let mut left: Vec<Orderable> = moment.iter().filter(|o| !heroes.contains(&o.id)).cloned().collect();
    left.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
    while !left.is_empty() {
        let seed = left.remove(0);
        let class = shape_class(&seed);
        let groups_here = grid_style || class != 1;
        let groupable = |c: &Orderable| grid_style || shape_class(c) == class;
        let capacity = if grid_style { 4 } else if class == 2 { 2 } else { 3 };
        let mut group = vec![seed];
        while groups_here && group.len() < capacity {
            let best = left
                .iter()
                .enumerate()
                .filter(|(_, c)| groupable(c) && group.iter().all(|g| !twins(c, g)))
                .map(|(i, c)| (i, group.iter().map(|g| compat(c, g)).sum::<f32>() / group.len() as f32))
                .max_by(|a, b| a.1.total_cmp(&b.1).then(left[b.0].id.cmp(&left[a.0].id)));
            match best {
                Some((i, _)) => group.push(left.remove(i)),
                None => break,
            }
        }
        let shapes: Vec<PhotoShape> = group.iter().map(PhotoShape::from).collect();
        let (frame, take) = if grid_style {
            let (frame, take) = choose_frame(&shapes);
            match group_frame(&shapes) {
                Some(f) => f,
                None if frame.is_multi() => (frame, take),
                None if shapes.len() >= 2 => (FrameKind::Grid, 2),
                None => (frame, take),
            }
        } else {
            choose_frame(&shapes)
        };
        let take = take.clamp(1, group.len());
        // What the frame could not hold goes back to be grouped again.
        for back in group.drain(take..).rev() {
            left.insert(0, back);
        }
        let frame = if group.len() == 1 { single(&group[0]).frame } else { frame };
        groups.push(Group { frame, photos: group });
    }
    let pos = |id: i64| moment.iter().position(|o| o.id == id).unwrap_or(usize::MAX);
    groups.sort_by_key(|g| g.photos.iter().map(|o| pos(o.id)).min());
    groups
}

/// Which layout suits the photographs that start here, and how many of them it takes.
///
/// The checks run biggest-first, and a layout is only offered when there are photographs to
/// fill it: a half-empty quad looks like a mistake, where three-up looks deliberate.
pub fn choose_frame(rest: &[PhotoShape]) -> (FrameKind, usize) {
    let Some(first) = rest.first() else { return (FrameKind::Fill, 1) };
    // A video plays alone and whole: over its own blurred copy when it is upright (Round 6).
    if first.video.is_some() {
        return (if first.portrait() { FrameKind::Blur } else { FrameKind::Fill }, 1);
    }
    // Nothing groups across a video.
    let rest = &rest[..rest.iter().position(|p| p.video.is_some()).unwrap_or(rest.len())];
    let portraits = rest.iter().take_while(|p| p.portrait()).count();
    let panoramas = rest.iter().take_while(|p| p.aspect >= PANORAMA_ASPECT).count();
    let narrow = |n: usize, limit: f32| rest.len() >= n && rest[..n].iter().all(|p| p.aspect < limit);

    if panoramas >= 2 {
        (FrameKind::Stack, 2)
    } else if portraits >= 3 && narrow(3, THREE_UP_ASPECT) {
        (FrameKind::Grid, 3)
    } else if portraits >= 2 {
        (FrameKind::Grid, 2)
    } else if first.portrait() {
        (FrameKind::Blur, 1)
    } else {
        (FrameKind::Fill, 1)
    }
}

/// The `Grid` style's choice: fit as many photographs into each frame as will go.
///
/// `Quad` and `Feature` are here rather than in `choose_frame` because they buy *density*,
/// not fit. Three 9:16 photographs fill a 16:9 frame exactly, so a quad would show each of
/// them at a third the size for no reason; a landscape photograph already fits a landscape
/// frame, so a feature only shrinks it. Both are right when the user has asked to see more
/// photographs at once, and wrong as a default — which is what `Grid` means and `Smart`
/// does not.
fn group_frame(rest: &[PhotoShape]) -> Option<(FrameKind, usize)> {
    let rest = &rest[..rest.iter().position(|p| p.video.is_some()).unwrap_or(rest.len())];
    let first = rest.first()?;
    if rest.len() >= 4 && rest[..4].iter().all(|p| p.aspect < PANORAMA_ASPECT) {
        return Some((FrameKind::Quad, 4));
    }
    let hero = !first.portrait() && first.aspect < PANORAMA_ASPECT;
    (hero && rest.len() >= 3 && rest[1].portrait() && rest[2].portrait()).then_some((FrameKind::Feature, 3))
}

/// How many photos share a grid, given the run of portraits that starts it.
pub fn grid_size(aspects: &[f32]) -> usize {
    if aspects.len() >= 3 && aspects[..3].iter().all(|a| *a < THREE_UP_ASPECT) {
        3
    } else {
        2
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum TransitionKind {
    None,
    Crossfade,
    /// Through black, and back out of it.
    FadeBlack,
    FadeWhite,
    /// The outgoing picture travels left, the incoming follows it in.
    Slide,
    SlideUp,
    /// The incoming picture is revealed behind a moving edge.
    Wipe,
    WipeUp,
    /// The incoming picture opens out of the middle.
    Circle,
    Zoom,
    /// Both pictures travel down.
    SlideDown,
    /// The incoming picture slides in *over* the outgoing one, which stays put.
    Cover,
    /// The outgoing picture slides away; the incoming one was behind it all along.
    Reveal,
    /// The outgoing picture closes to a point.
    CircleClose,
    /// The frame parts down the middle.
    VertOpen,
    /// And across it.
    HorzOpen,
    /// A wipe out of the top-left corner.
    Corner,
    /// The outgoing picture squashes to a line.
    Squeeze,
    /// Columns of the incoming picture arrive in turn — a grid's transition.
    Slices,
    /// The same, in bands across.
    VSlices,
}

impl TransitionKind {
    /// Every transition, in the order the editor lists them.
    pub const ALL: [TransitionKind; 20] = [
        Self::None,
        Self::Crossfade,
        Self::FadeBlack,
        Self::FadeWhite,
        Self::Slide,
        Self::SlideUp,
        Self::SlideDown,
        Self::Cover,
        Self::Reveal,
        Self::Wipe,
        Self::WipeUp,
        Self::Corner,
        Self::VertOpen,
        Self::HorzOpen,
        Self::Circle,
        Self::CircleClose,
        Self::Squeeze,
        Self::Slices,
        Self::VSlices,
        Self::Zoom,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Transition {
    pub kind: TransitionKind,
    pub duration_ms: u32,
    /// Chosen by Lumen rather than by the user, so a beat sync may choose it again. Absent
    /// from documents saved before Phase 7a, which count as hand-picked and are left alone.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[ts(as = "Option<bool>", optional)]
    pub auto: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MotionKind {
    None,
    KenBurns,
}

/// `[x, y, scale]`: the frame's centre moved by `x` frame widths and `y` frame heights, and
/// zoomed by `scale` (1 is the photo covering the frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Motion {
    pub kind: MotionKind,
    pub from: [f32; 3],
    pub to: [f32; 3],
}

impl Motion {
    pub fn none() -> Self {
        Self { kind: MotionKind::None, from: [0.0, 0.0, 1.0], to: [0.0, 0.0, 1.0] }
    }

    /// Slowly in towards a point a little off centre.
    pub fn zoom_in(dx: f32, dy: f32) -> Self {
        Self { kind: MotionKind::KenBurns, from: [0.0, 0.0, 1.0], to: [dx, dy, 1.12] }
    }

    /// Slowly out from a point a little off centre.
    pub fn zoom_out(dx: f32, dy: f32) -> Self {
        Self { kind: MotionKind::KenBurns, from: [dx, dy, 1.12], to: [0.0, 0.0, 1.0] }
    }
}

/// The shortest and longest a clip may be, which the editor's slider also enforces.
pub const MIN_CLIP_MS: u32 = 500;
pub const MAX_CLIP_MS: u32 = 30_000;
/// A clip never gets fewer beats than this, whatever the tempo — at 180 BPM a single beat
/// is a third of a second, which is a flicker rather than a photograph.
pub const MIN_BEATS_PER_CLIP: u32 = 4;
/// Tempi a snap will act on. Outside this the estimate is not music.
const SNAPPABLE_BPM: std::ops::RangeInclusive<f32> = 20.0..=300.0;

/// How far into its track a show may start.
pub const MAX_AUDIO_START_MS: u32 = 30 * 60_000;
const DEFAULT_CLIP_MS: u32 = 4000;
const DEFAULT_TRANSITION_MS: u32 = 600;

impl SlideshowDoc {
    /// A show of `asset_ids`: 1080p30, four seconds a photo, crossfades, and Ken Burns that
    /// alternates in and out so consecutive photos never move the same way.
    /// A show of photos whose shapes are unknown, so every one fills the frame. Callers
    /// with a catalogue to hand should use [`build`] instead, which frames portraits.
    pub fn new_for(asset_ids: &[i64]) -> Self {
        let photos: Vec<PhotoShape> = asset_ids.iter().map(|id| PhotoShape { id: *id, aspect: 1.5, video: None }).collect();
        build(&photos, LayoutStyle::Fill)
    }

    /// Everything inside its range. Every document that crosses `invoke` passes through here.
    pub fn normalized(mut self) -> Self {
        self.version = 1;
        self.fps = self.fps.clamp(24, 60);
        let even = |v: u32| (v.clamp(320, 3840) / 2) * 2;
        self.resolution = [even(self.resolution[0]), even(self.resolution[1])];
        if let Some(a) = self.audio.as_mut() {
            // The best part of a song can be well into it (Round 6), but not past half an
            // hour: beyond that a bad value would only silence the show.
            a.start_ms = a.start_ms.min(MAX_AUDIO_START_MS);
            a.fade_in_ms = a.fade_in_ms.min(10_000);
            a.fade_out_ms = a.fade_out_ms.min(10_000);
            a.gain_db = if a.gain_db.is_finite() { a.gain_db.clamp(-30.0, 10.0) } else { 0.0 };
        }
        for c in &mut self.clips {
            // A video plays alone, filling the frame or over its blurred self, and still: its
            // own movement is the motion (Round 6).
            if let Some(v) = c.video.as_mut() {
                if v.out_ms <= v.in_ms {
                    v.out_ms = v.in_ms + MIN_CLIP_MS;
                }
                if !matches!(c.frame, FrameKind::Fill | FrameKind::Blur) {
                    c.frame = FrameKind::Fill;
                }
                c.extra.clear();
                c.motion = Motion::none();
            }
            // Only a grid shows more than one photo, and never more than three.
            c.extra.truncate(c.frame.capacity().saturating_sub(1));
            c.duration_ms = c.duration_ms.clamp(500, 30_000);
            c.transition.duration_ms = c.transition.duration_ms.min(3000);
            let fix = |p: [f32; 3]| {
                let scale = if p[2].is_finite() { p[2].clamp(1.0, 2.0) } else { 1.0 };
                let room = (1.0 - 1.0 / scale) / 2.0;
                let pan = |v: f32| if v.is_finite() { v.clamp(-room, room) } else { 0.0 };
                [pan(p[0]), pan(p[1]), scale]
            };
            c.motion.from = fix(c.motion.from);
            c.motion.to = fix(c.motion.to);
        }
        self
    }
}

/// When each clip starts, how much it overlaps the next, and how long the show is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Timeline {
    pub starts: Vec<u32>,
    pub overlaps: Vec<u32>,
    pub total_ms: u32,
}

/// A transition overlaps a clip with the next by its duration, but by no more than half of
/// either clip, so a short clip is never swallowed by the transitions on both sides of it.
pub fn timeline(doc: &SlideshowDoc) -> Timeline {
    let n = doc.clips.len();
    let overlaps: Vec<u32> = (0..n)
        .map(|i| {
            let c = &doc.clips[i];
            match doc.clips.get(i + 1) {
                Some(next) if c.transition.kind != TransitionKind::None => {
                    c.transition.duration_ms.min(c.duration_ms / 2).min(next.duration_ms / 2)
                }
                _ => 0,
            }
        })
        .collect();
    let mut starts = Vec::with_capacity(n);
    let mut t = 0u32;
    for (clip, overlap) in doc.clips.iter().zip(&overlaps) {
        starts.push(t);
        t += clip.duration_ms - overlap;
    }
    let total_ms = match n {
        0 => 0,
        _ => starts[n - 1] + doc.clips[n - 1].duration_ms,
    };
    Timeline { starts, overlaps, total_ms }
}

/// The frame a moment falls on (`ms` at `fps`), rounded as the export rounds a clip's length.
pub fn frame_at(ms: u32, fps: u32) -> u32 {
    ((ms as u64 * fps as u64 + 500) / 1000) as u32
}

/// A segment shorter than this is not worth its own FFmpeg process.
pub const MIN_SEGMENT_MS: u32 = 4000;

/// Where an export may be cut into `n` pieces rendered side by side (Round 4F): frame ranges
/// `[from, to)` covering the whole show, balanced by length. A boundary only ever falls where
/// one photo is alone on screen (inside a clip's *hold*, or exactly on a cut), at least one
/// frame clear of any transition, so every piece is the same pictures the whole run would
/// make. Fewer pieces when the show is short or has nowhere to cut.
pub fn segment_bounds(doc: &SlideshowDoc, n: usize) -> Vec<(u32, u32)> {
    let fps = doc.fps.max(1);
    let t = timeline(doc);
    let total = frame_at(t.total_ms, fps);
    let n = n.clamp(1, (t.total_ms / MIN_SEGMENT_MS).max(1) as usize);
    if n == 1 || total == 0 {
        return vec![(0, total)];
    }
    // Frames on which a piece may start: inside each clip's hold, a frame clear of its
    // transitions (a cut needs no clearance, so its join frame is allowed as well).
    let mut allowed: Vec<(u32, u32)> = Vec::new();
    for (i, clip) in doc.clips.iter().enumerate() {
        let start = t.starts[i];
        let into = if i > 0 { t.overlaps[i - 1] } else { 0 };
        let out = t.overlaps[i];
        let (a, b) = (start + into, start + clip.duration_ms - out);
        let lo = ((a as u64 * fps as u64).div_ceil(1000) as u32) + u32::from(into > 0);
        let hi = ((b as u64 * fps as u64) / 1000) as u32;
        let hi = if out > 0 { hi.saturating_sub(1) } else { hi };
        if lo <= hi {
            allowed.push((lo.max(1), hi.min(total.saturating_sub(1))));
        }
    }
    let min_frames = frame_at(MIN_SEGMENT_MS, fps);
    let mut cuts: Vec<u32> = Vec::new();
    for k in 1..n {
        let want = (total as u64 * k as u64 / n as u64) as u32;
        let near = allowed
            .iter()
            .filter(|(lo, hi)| lo <= hi)
            .map(|&(lo, hi)| want.clamp(lo, hi))
            .min_by_key(|f| f.abs_diff(want));
        if let Some(f) = near {
            let last = cuts.last().copied().unwrap_or(0);
            if f >= last + min_frames && f + min_frames <= total {
                cuts.push(f);
            }
        }
    }
    let mut out = Vec::with_capacity(cuts.len() + 1);
    let mut from = 0;
    for c in cuts {
        out.push((from, c));
        from = c;
    }
    out.push((from, total));
    out
}

/// Smoothstep: starts and ends at rest, so a Ken Burns move never jerks at a cut.
pub fn ease(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Where the motion is at clip-local progress `u` (0 at the clip's start, 1 at its end).
pub fn motion_at(m: &Motion, u: f32) -> [f32; 3] {
    if m.kind == MotionKind::None {
        return [0.0, 0.0, 1.0];
    }
    let e = ease(u);
    [0, 1, 2].map(|k| m.from[k] + (m.to[k] - m.from[k]) * e)
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SlideshowSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    pub clip_count: u32,
    pub duration_ms: u32,
    #[ts(type = "number | null")]
    pub cover_asset_id: Option<i64>,
    #[ts(type = "number")]
    pub updated_at: i64,
}

fn decode(json: &str) -> Result<SlideshowDoc> {
    Ok(serde_json::from_str::<SlideshowDoc>(json)?.normalized())
}

/// Newest edit first.
pub fn list(conn: &Connection) -> Result<Vec<SlideshowSummary>> {
    let mut stmt = conn.prepare("SELECT id, name, document_json, updated_at FROM slideshows ORDER BY updated_at DESC, id DESC")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(|(id, name, json, updated_at)| {
            let doc = decode(&json)?;
            Ok(SlideshowSummary {
                id,
                name,
                clip_count: doc.clips.len() as u32,
                duration_ms: timeline(&doc).total_ms,
                cover_asset_id: doc.clips.first().map(|c| c.asset_id),
                updated_at,
            })
        })
        .collect()
}

/// The show's name and document. Clips whose photo has gone (trashed, deleted) are left
/// out, so a show never refers to something it cannot render.
pub fn get(conn: &Connection, id: i64) -> Result<(String, SlideshowDoc)> {
    let (name, json): (String, String) = conn
        .query_row("SELECT name, document_json FROM slideshows WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?
        .ok_or_else(|| CoreError::NotFound(format!("slideshow {id}")))?;
    let mut doc = decode(&json)?;
    // A video is a clip only with the part it plays (Round 6); a video clip without one is
    // from before videos could be in a show, and is left out as it always was.
    let mut stmt = conn.prepare_cached("SELECT media_type FROM assets WHERE id = ?1 AND state = 0 AND concealed = 0")?;
    doc.clips.retain(|c| match stmt.query_row([c.asset_id], |r| r.get::<_, i64>(0)).optional() {
        Ok(Some(1)) => c.video.is_some(),
        Ok(Some(_)) => true,
        _ => false,
    });
    Ok((name, doc))
}

pub fn create(conn: &Connection, name: &str, doc: &SlideshowDoc, now: i64) -> Result<i64> {
    let json = serde_json::to_string(&doc.clone().normalized())?;
    conn.execute(
        "INSERT INTO slideshows (name, document_json, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        params![name.trim(), json, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn save(conn: &Connection, id: i64, name: &str, doc: &SlideshowDoc, now: i64) -> Result<()> {
    let json = serde_json::to_string(&doc.clone().normalized())?;
    let n = conn.execute("UPDATE slideshows SET name = ?2, document_json = ?3, updated_at = ?4 WHERE id = ?1", params![id, name.trim(), json, now])?;
    if n == 0 {
        return Err(CoreError::NotFound(format!("slideshow {id}")));
    }
    Ok(())
}

/// Removes the show and returns it, so the frontend can offer *Undo*.
pub fn delete(conn: &Connection, id: i64) -> Result<Option<(String, SlideshowDoc)>> {
    let old = get(conn, id).ok();
    conn.execute("DELETE FROM slideshows WHERE id = ?1", [id])?;
    Ok(old)
}

/// The file name of every track a slideshow or a memory film plays (Round 6), so automatic
/// music never gives two shows the same tune. `except_film` leaves out one memory's film —
/// the one being made again, which may keep its own music.
pub fn music_in_use(conn: &Connection, except_film: Option<i64>) -> Result<std::collections::HashSet<String>> {
    let mut docs: Vec<String> = conn.prepare("SELECT document_json FROM slideshows")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
    let films: Vec<String> = conn
        .prepare("SELECT doc_json FROM memory_films WHERE memory_id IS NOT ?1")?
        .query_map([except_film], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    docs.extend(films);
    Ok(docs
        .iter()
        .filter_map(|json| serde_json::from_str::<SlideshowDoc>(json).ok()?.audio)
        .filter_map(|a| a.path.rsplit(['/', '\\']).next().map(str::to_string))
        .filter(|name| !name.is_empty())
        .collect())
}

/// A show of a memory's highlights, in the order they were taken, named after the memory.
pub fn from_memory(
    conn: &Connection,
    memory_id: i64,
    now: i64,
    style: LayoutStyle,
    audio: Option<AudioTrack>,
) -> Result<i64> {
    let memory = crate::memories::get_memory(conn, memory_id)?;
    let ids: Vec<i64> = conn
        .prepare(
            "SELECT a.id FROM memory_items mi JOIN assets a ON a.id = mi.asset_id
             WHERE mi.memory_id = ?1 AND mi.curated = 1 AND a.state = 0 AND a.concealed = 0
             ORDER BY COALESCE(a.captured_at, a.imported_at), a.id",
        )?
        .query_map([memory_id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let ids = with_ready_videos(conn, &ids)?;
    let doc = SlideshowDoc { audio, ..build_arranged(&orderables_for(conn, &ids)?, style) };
    create(conn, &memory.title, &doc, now)
}

/// Everything the arranger needs about `ids`, in the order given.
pub fn orderables_for(conn: &Connection, ids: &[i64]) -> Result<Vec<Orderable>> {
    type Row = (Option<i64>, Option<i64>, i64, Option<i64>, bool, i64, Option<f64>, Option<i64>, Option<String>);
    let mut rows: Vec<(i64, Option<Row>)> = Vec::with_capacity(ids.len());
    for id in ids {
        let row: Option<Row> = conn
            .query_row(
                "SELECT width, height, orientation, captured_at, is_favorite, rating, sharpness, phash, dominant_color
                 FROM assets WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?)),
            )
            .optional()?;
        rows.push((*id, row));
    }
    // Sharpness only means anything next to the other photographs in the same show.
    let sharpest = rows.iter().filter_map(|(_, r)| r.as_ref().and_then(|x| x.6)).fold(0f64, f64::max) as f32;
    let mut features = crate::embedding::load(conn, ids)?;
    let spans = video_spans(conn, ids)?;
    Ok(rows
        .into_iter()
        .map(|(id, row)| match row {
            Some((w, h, orientation, captured_at, favorite, rating, sharpness, phash, color)) => Orderable {
                id,
                aspect: displayed_aspect(w, h, orientation),
                captured_at: captured_at.unwrap_or(0),
                score: match spans.get(&id) {
                    // A video is as good as its best part (Round 6), and never worse than an
                    // ordinary photograph: a moment that moves is worth showing.
                    Some((_, Some(h))) => 0.5 + 0.5 * h,
                    _ => quality_score(favorite, rating, sharpness.map(|s| s as f32), sharpest),
                },
                phash: phash.map(|p| p as u64),
                vec: features.remove(&id).map(|f| f.vec),
                color: color.as_deref().and_then(hex_color),
                video: spans.get(&id).map(|s| s.0),
            },
            // A photo that is no longer there still has to hold its place in the order.
            None => Orderable { id, aspect: 1.5, captured_at: 0, score: 0.0, phash: None, vec: None, color: None, video: None },
        })
        .collect())
}

/// The part each video among `ids` plays in a show (Round 6), with how good that part is:
/// its highlight, or — for a video not looked at yet — its opening seconds, unscored.
/// Photographs are absent.
pub fn video_spans(conn: &Connection, ids: &[i64]) -> Result<HashMap<i64, (VideoSpan, Option<f32>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT a.duration_ms, h.in_ms, h.out_ms, h.score FROM assets a
         LEFT JOIN video_highlights h ON h.asset_id = a.id WHERE a.id = ?1 AND a.media_type = 1",
    )?;
    let mut out = HashMap::new();
    for id in ids {
        type Row = (Option<i64>, Option<u32>, Option<u32>, Option<f64>);
        let row: Option<Row> =
            stmt.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        let Some((duration, from, to, score)) = row else { continue };
        let span = match (from, to) {
            (Some(a), Some(b)) if b > a => (VideoSpan { in_ms: a, out_ms: b }, score.map(|s| s as f32)),
            _ => {
                let len = duration.unwrap_or(0).clamp(0, crate::video_highlight::HIGHLIGHT_MAX_MS as i64) as u32;
                (VideoSpan { in_ms: 0, out_ms: if len == 0 { crate::video_highlight::HIGHLIGHT_MAX_MS } else { len } }, None)
            }
        };
        out.insert(*id, span);
    }
    Ok(out)
}

/// `ids` less the videos whose best part has not been found yet (Round 6): what an automatic
/// show uses, so a memory never waits on — or plays the wrong part of — a video.
pub fn with_ready_videos(conn: &Connection, ids: &[i64]) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare_cached(
        "SELECT 1 FROM assets a WHERE a.id = ?1 AND a.media_type = 1
           AND NOT EXISTS (SELECT 1 FROM video_highlights h WHERE h.asset_id = a.id)",
    )?;
    Ok(ids.iter().copied().filter(|id| !stmt.exists([id]).unwrap_or(false)).collect())
}

/// `#rrggbb` as bytes, as the thumbnailer writes `dominant_color`.
fn hex_color(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// A show of these photographs: arranged, then framed.
pub fn build_arranged(photos: &[Orderable], style: LayoutStyle) -> SlideshowDoc {
    let ordered = arrange(photos);
    let mut groups: Vec<(FrameKind, Vec<i64>)> = Vec::new();
    let mut start = 0;
    while start < ordered.len() {
        let mut end = start + 1;
        while end < ordered.len() && ordered[end].captured_at - ordered[end - 1].captured_at <= MOMENT_GAP_S {
            end += 1;
        }
        for g in group_moment(&ordered[start..end], style) {
            groups.push((g.frame, g.photos.iter().map(|p| p.id).collect()));
        }
        start = end;
    }
    let by_id: HashMap<i64, Orderable> = ordered.iter().map(|o| (o.id, o.clone())).collect();
    assemble(&groups, style, &ordered.iter().map(PhotoShape::from).collect::<Vec<_>>(), &by_id)
}

/// The shapes of `ids`, in the order given, for the layout builder. Orientation is applied,
/// so a photo stored sideways counts as the portrait it appears to be.
pub fn shapes_for(conn: &Connection, ids: &[i64]) -> Result<Vec<PhotoShape>> {
    let spans = video_spans(conn, ids)?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row: Option<(Option<i64>, Option<i64>, i64)> = conn
            .query_row("SELECT width, height, orientation FROM assets WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let aspect = match row {
            Some((w, h, orientation)) => displayed_aspect(w, h, orientation),
            None => 1.5,
        };
        out.push(PhotoShape { id: *id, aspect, video: spans.get(id).map(|s| s.0) });
    }
    Ok(out)
}

/// How wide a photograph looks, orientation applied — a photo stored sideways counts as the
/// portrait it appears to be. A photo with no stored size is assumed to be an ordinary
/// landscape, which is what `fill` would do with it anyway.
fn displayed_aspect(width: Option<i64>, height: Option<i64>, orientation: i64) -> f32 {
    match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => {
            let (w, h) = if (5..=8).contains(&orientation) { (h, w) } else { (w, h) };
            w as f32 / h as f32
        }
        _ => 1.5,
    }
}

#[cfg(test)]
mod tests {
    // Round 4F, Task 3: where an export may be cut.

    fn show(kinds: &[TransitionKind], clip_ms: u32, transition_ms: u32) -> SlideshowDoc {
        let mut d = SlideshowDoc::new_for(&(1..=kinds.len() as i64).collect::<Vec<_>>());
        for (c, k) in d.clips.iter_mut().zip(kinds) {
            c.duration_ms = clip_ms;
            c.transition = Transition { kind: *k, duration_ms: transition_ms, auto: false };
        }
        d
    }

    /// The frames on which two photos share the screen.
    fn shared(d: &SlideshowDoc) -> Vec<(u32, u32)> {
        let t = timeline(d);
        (0..d.clips.len().saturating_sub(1))
            .filter(|i| t.overlaps[*i] > 0)
            .map(|i| {
                let a = t.starts[i + 1];
                (a * d.fps / 1000, (a + t.overlaps[i]) * d.fps / 1000)
            })
            .collect()
    }

    #[test]
    fn pieces_cover_the_show_and_never_cut_through_a_transition() {
        let d = show(&[TransitionKind::Crossfade; 12], 4000, 800);
        let total = frame_at(timeline(&d).total_ms, d.fps);
        for n in 1..=8 {
            let b = segment_bounds(&d, n);
            assert_eq!(b.first().unwrap().0, 0);
            assert_eq!(b.last().unwrap().1, total);
            assert!(b.windows(2).all(|w| w[0].1 == w[1].0), "{b:?}");
            for &(from, _) in &b[1..] {
                for (lo, hi) in shared(&d) {
                    assert!(from < lo || from > hi, "piece starts at {from}, inside a transition {lo}–{hi}");
                }
            }
        }
    }

    #[test]
    fn two_pieces_of_a_twelve_clip_show_are_balanced() {
        let d = show(&[TransitionKind::Crossfade; 12], 4000, 600);
        let b = segment_bounds(&d, 2);
        assert_eq!(b.len(), 2);
        let (x, y) = (b[0].1 - b[0].0, b[1].1 - b[1].0);
        assert!((x as f32 / y as f32 - 1.0).abs() < 0.2, "{b:?}");
    }

    #[test]
    fn a_short_show_is_one_piece() {
        let d = show(&[TransitionKind::None; 2], 3000, 0);
        assert_eq!(segment_bounds(&d, 6).len(), 1);
    }

    #[test]
    fn a_cut_is_a_place_to_split() {
        let d = show(&[TransitionKind::None; 4], 5000, 0);
        let b = segment_bounds(&d, 2);
        assert_eq!(b, vec![(0, 300), (300, 600)], "exactly at the cut between clips 2 and 3");
    }

    use super::*;

    fn clip(asset_id: i64, duration_ms: u32, kind: TransitionKind, transition_ms: u32) -> Clip {
        Clip {
            asset_id,
            extra: Vec::new(),
            frame: FrameKind::Fill,
            duration_ms,
            transition: Transition { kind, duration_ms: transition_ms, auto: false },
            motion: Motion::none(),
            video: None,
        }
    }

    fn doc(clips: Vec<Clip>) -> SlideshowDoc {
        SlideshowDoc { clips, ..SlideshowDoc::new_for(&[]) }
    }

    const FRAME: f32 = 16.0 / 9.0;

    #[test]
    fn a_landscape_photo_covers_the_frame() {
        // Wider than 16:9: full height, overhanging sides.
        let wide = frame_layout(FrameKind::Fill, &[2.0], FRAME);
        assert_eq!(wide.len(), 1);
        assert!(!wide[0].blurred);
        assert!((wide[0].rect.h - 1.0).abs() < 1e-6, "{:?}", wide[0].rect);
        assert!(wide[0].rect.w > 1.0);

        // 3:2 is narrower than 16:9, so it covers by width and overhangs top and bottom.
        let classic = frame_layout(FrameKind::Fill, &[3.0 / 2.0], FRAME)[0].rect;
        assert!((classic.w - 1.0).abs() < 1e-6, "{classic:?}");
        assert!(classic.h > 1.0);

        // Either way the photo covers the frame and sits in the middle of it.
        for rect in [wide[0].rect, classic] {
            assert!(rect.w >= 1.0 - 1e-6 && rect.h >= 1.0 - 1e-6, "{rect:?}");
            assert!((rect.x + rect.w / 2.0 - 0.5).abs() < 1e-6 && (rect.y + rect.h / 2.0 - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn a_portrait_is_shown_whole_over_a_blurred_copy_of_itself() {
        let p = frame_layout(FrameKind::Blur, &[3.0 / 4.0], FRAME);
        assert_eq!(p.len(), 2);
        assert!(p[0].blurred && p[0].rect.w >= 1.0 && p[0].rect.h >= 1.0, "backdrop covers: {:?}", p[0].rect);
        assert!(!p[1].blurred);
        assert!((p[1].rect.h - 1.0).abs() < 1e-6);
        assert!(p[1].rect.w < 1.0);
        assert!((p[1].rect.x + p[1].rect.w / 2.0 - 0.5).abs() < 1e-6);
        assert!(p[1].rect.x >= 0.0);
    }

    #[test]
    fn two_portraits_share_the_frame_without_overlapping() {
        let p = frame_layout(FrameKind::Grid, &[3.0 / 4.0, 3.0 / 4.0], FRAME);
        assert_eq!(p.len(), 3, "a backdrop and two photos");
        assert!(p[0].blurred);
        let (left, right) = (p[1].rect, p[2].rect);
        assert_eq!((p[1].photo, p[2].photo), (0, 1));
        assert!(left.x + left.w <= right.x + 1e-6, "{left:?} then {right:?}");
        assert!(left.x >= 0.0 && right.x + right.w <= 1.0 + 1e-6);
    }

    #[test]
    fn three_tall_phone_photos_fit_three_columns() {
        let p = frame_layout(FrameKind::Grid, &[0.5625, 0.5625, 0.5625], FRAME);
        assert_eq!(p.len(), 4);
        for cell in &p[1..] {
            assert!((cell.rect.h - 1.0).abs() < 0.02, "{:?}", cell.rect);
        }
    }

    #[test]
    fn a_clip_lists_every_photo_it_shows() {
        let c = Clip {
            asset_id: 7,
            extra: vec![8, 9],
            frame: FrameKind::Grid,
            duration_ms: 4000,
            transition: Transition { kind: TransitionKind::Crossfade, duration_ms: 600, auto: false },
            motion: Motion::none(),
            video: None,
        };
        assert_eq!(c.assets(), vec![7, 8, 9]);
    }

    /// Documents saved before frames existed must still parse.
    #[test]
    fn a_version_one_clip_without_a_frame_is_a_fill() {
        let json = r#"{"asset_id":3,"duration_ms":4000,"transition":{"kind":"crossfade","duration_ms":600},
                       "motion":{"kind":"none","from":[0,0,1],"to":[0,0,1]}}"#;
        let c: Clip = serde_json::from_str(json).unwrap();
        assert_eq!(c.frame, FrameKind::Fill);
        assert_eq!(c.assets(), vec![3]);
    }

    fn shapes(aspects: &[f32]) -> Vec<PhotoShape> {
        aspects.iter().enumerate().map(|(i, a)| PhotoShape { id: i as i64 + 1, aspect: *a, video: None }).collect()
    }

    #[test]
    fn smart_pairs_portraits_and_leaves_landscapes_alone() {
        let d = build(&shapes(&[1.5, 0.75, 0.75, 1.5, 0.75]), LayoutStyle::Smart);
        let kinds: Vec<FrameKind> = d.clips.iter().map(|c| c.frame).collect();
        assert_eq!(kinds, vec![FrameKind::Fill, FrameKind::Grid, FrameKind::Fill, FrameKind::Blur]);
        assert_eq!(d.clips[1].assets(), vec![2, 3], "the two portraits share a frame");
        assert_eq!(d.clips[3].assets(), vec![5], "the lone portrait gets a backdrop");
    }

    #[test]
    fn three_phone_portraits_make_one_grid() {
        let d = build(&shapes(&[0.5625, 0.5625, 0.5625]), LayoutStyle::Smart);
        assert_eq!(d.clips.len(), 1);
        assert_eq!(d.clips[0].assets(), vec![1, 2, 3]);
    }

    #[test]
    fn every_photo_appears_exactly_once_whatever_the_style() {
        for style in [LayoutStyle::Smart, LayoutStyle::Fill, LayoutStyle::Blur, LayoutStyle::Grid, LayoutStyle::Random] {
            let d = build(&shapes(&[1.5, 0.75, 0.75, 0.5625, 0.5625, 0.5625, 1.0]), style);
            let mut seen: Vec<i64> = d.clips.iter().flat_map(|c| c.assets()).collect();
            seen.sort();
            assert_eq!(seen, vec![1, 2, 3, 4, 5, 6, 7], "{style:?} loses or repeats a photo");
        }
    }

    #[test]
    fn random_is_reproducible_from_the_photos() {
        let a = build(&shapes(&[1.5, 0.75, 0.75, 1.5]), LayoutStyle::Random);
        let b = build(&shapes(&[1.5, 0.75, 0.75, 1.5]), LayoutStyle::Random);
        assert_eq!(a, b, "the same photos give the same show");
        let kinds: Vec<TransitionKind> = a.clips.iter().map(|c| c.transition.kind).collect();
        assert!(kinds.iter().any(|k| *k != TransitionKind::Crossfade), "random varies the transitions: {kinds:?}");
    }

    #[test]
    fn a_document_with_no_shapes_is_all_fill() {
        let d = SlideshowDoc::new_for(&[4, 5]);
        assert!(d.clips.iter().all(|c| c.frame == FrameKind::Fill));
        assert_eq!(d.clips.len(), 2);
    }

    #[test]
    fn crossfades_overlap_neighbouring_clips() {
        let d = doc(vec![clip(1, 4000, TransitionKind::Crossfade, 600), clip(2, 4000, TransitionKind::Crossfade, 600), clip(3, 4000, TransitionKind::Crossfade, 600)]);
        let t = timeline(&d);
        assert_eq!(t.starts, [0, 3400, 6800]);
        assert_eq!(t.overlaps, [600, 600, 0], "the last clip has nothing to transition into");
        assert_eq!(t.total_ms, 10_800);
    }

    #[test]
    fn a_short_clip_limits_the_overlaps_around_it_and_none_has_none() {
        let d = doc(vec![clip(1, 4000, TransitionKind::Slide, 2000), clip(2, 1000, TransitionKind::None, 600), clip(3, 4000, TransitionKind::Zoom, 600)]);
        let t = timeline(&d);
        assert_eq!(t.overlaps, [500, 0, 0]);
        assert_eq!(t.starts, [0, 3500, 4500]);
        assert_eq!(t.total_ms, 8500);
        assert_eq!(timeline(&doc(vec![])).total_ms, 0);
    }

    #[test]
    fn easing_is_smooth_and_symmetric() {
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
        for u in [0.1f32, 0.3, 0.45] {
            assert!((ease(u) + ease(1.0 - u) - 1.0).abs() < 1e-6);
        }
        assert_eq!(ease(-1.0), 0.0);
        assert_eq!(ease(2.0), 1.0);
    }

    #[test]
    fn motion_runs_from_its_start_to_its_end() {
        let m = Motion { kind: MotionKind::KenBurns, from: [0.0, 0.0, 1.0], to: [0.05, -0.02, 1.2] };
        assert_eq!(motion_at(&m, 0.0), [0.0, 0.0, 1.0]);
        let end = motion_at(&m, 1.0);
        assert!((end[0] - 0.05).abs() < 1e-6 && (end[1] + 0.02).abs() < 1e-6 && (end[2] - 1.2).abs() < 1e-6);
        assert_eq!(motion_at(&Motion::none(), 0.7), [0.0, 0.0, 1.0], "no motion is the plain frame");
    }

    #[test]
    fn normalizing_keeps_every_edge_out_of_frame() {
        let d = SlideshowDoc {
            fps: 500,
            resolution: [99_999, 7],
            clips: vec![Clip { asset_id: 1, extra: Vec::new(), frame: FrameKind::Fill, duration_ms: 10, transition: Transition { kind: TransitionKind::Crossfade, duration_ms: 90_000, auto: false }, motion: Motion { kind: MotionKind::KenBurns, from: [0.4, -0.4, 1.0], to: [0.0, 0.0, 9.0] }, video: None }],
            ..SlideshowDoc::new_for(&[])
        }
        .normalized();
        assert_eq!(d.fps, 60);
        assert_eq!(d.resolution, [3840, 320]);
        let c = &d.clips[0];
        assert_eq!((c.duration_ms, c.transition.duration_ms), (500, 3000));
        assert_eq!(c.motion.from, [0.0, 0.0, 1.0], "at scale 1 there is no room to pan");
        assert_eq!(c.motion.to[2], 2.0);
    }

    #[test]
    fn the_spec_example_parses_and_a_new_show_is_sensible() {
        let d: SlideshowDoc = serde_json::from_str(r#"{"version":1,"resolution":[1920,1080],"fps":30,"audio":{"path":"music/1.mp3","fade_in_ms":800,"fade_out_ms":1200,"gain_db":-6},"clips":[{"asset_id":1234,"duration_ms":4000,"transition":{"kind":"crossfade","duration_ms":600},"motion":{"kind":"ken_burns","from":[0,0,1.0],"to":[0.05,0.02,1.12]}}]}"#).unwrap();
        assert_eq!(d.clips[0].motion.kind, MotionKind::KenBurns);
        let n = SlideshowDoc::new_for(&[5, 6, 7]);
        assert_eq!((n.resolution, n.fps, n.clips.len()), ([1920, 1080], 30, 3));
        assert!(n.clips.iter().all(|c| c.motion.kind == MotionKind::KenBurns && c.duration_ms == 4000));
        assert_eq!(n.clone().normalized(), n, "a new show is already normal");
    }

    /// Writes the maths the preview must reproduce. Like the ts-rs types, `cargo test`
    /// regenerates it and a changed file is a change to review.
    #[test]
    fn writes_the_golden_file_for_the_preview() {
        let cases: Vec<serde_json::Value> = [
            SlideshowDoc::new_for(&[1, 2, 3, 4]),
            doc(vec![clip(1, 4000, TransitionKind::Slide, 2000), clip(2, 1000, TransitionKind::None, 600), clip(3, 4000, TransitionKind::Zoom, 600)]),
            SlideshowDoc {
                clips: vec![Clip { asset_id: 9, extra: Vec::new(), frame: FrameKind::Fill, duration_ms: 2500, transition: Transition { kind: TransitionKind::Crossfade, duration_ms: 800, auto: false }, motion: Motion { kind: MotionKind::KenBurns, from: [0.03, 0.02, 1.25], to: [-0.02, 0.0, 1.05] }, video: None }],
                ..SlideshowDoc::new_for(&[])
            },
            // Cut to 128 BPM from a first beat at 120 ms: the durations beat sync produces.
            snap_to_beat_grid(&SlideshowDoc::new_for(&[1, 2, 3, 4]), &BeatGrid::even(128.0, 120, 60_000)),
            // Eight portraits of two scenes, grouped by content (Phase 7a).
            build_arranged(
                &(0..8)
                    .map(|i| o(i, 0.56, i * 30, 0.5, Some(if i % 2 == 0 { tilt(0, 1, 0.1 * i as f32) } else { tilt(1, 0, 0.1 * i as f32) })))
                    .collect::<Vec<_>>(),
                LayoutStyle::Smart,
            ),
        ]
        .iter()
        .map(|d| {
            let samples: Vec<serde_json::Value> = d
                .clips
                .iter()
                .map(|c| {
                    let points: Vec<[f32; 4]> = (0..=10).map(|k| {
                        let u = k as f32 / 10.0;
                        let m = motion_at(&c.motion, u);
                        [u, m[0], m[1], m[2]]
                    }).collect();
                    serde_json::json!(points)
                })
                .collect();
            serde_json::json!({ "doc": d, "timeline": timeline(d), "samples": samples })
        })
        .collect();
        // The frame geometry the canvas must reproduce, for every shape that matters.
        let frames: Vec<serde_json::Value> = [
            (FrameKind::Fill, vec![1.5]),
            (FrameKind::Fill, vec![2.0]),
            (FrameKind::Blur, vec![0.75]),
            (FrameKind::Blur, vec![0.5625]),
            (FrameKind::Grid, vec![0.75, 0.75]),
            (FrameKind::Grid, vec![0.5625, 0.5625, 0.5625]),
            (FrameKind::Grid, vec![0.75, 1.5]),
            (FrameKind::Quad, vec![0.75, 0.75, 0.75, 0.75]),
            (FrameKind::Quad, vec![0.5625, 1.5, 0.75, 1.0]),
            (FrameKind::Feature, vec![1.5, 0.75, 0.75]),
            (FrameKind::Stack, vec![3.0, 3.0]),
        ]
        .into_iter()
        .map(|(kind, aspects)| {
            let frame_aspect = 16.0 / 9.0;
            serde_json::json!({
                "kind": kind,
                "aspects": aspects,
                "frameAspect": frame_aspect,
                "placements": frame_layout(kind, &aspects, frame_aspect),
            })
        })
        .collect();
        let text = serde_json::to_string_pretty(&serde_json::json!({ "cases": cases, "frames": frames })).unwrap() + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../src/lib/slideshow-golden.json");
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text).unwrap();
        }
    }

    fn db_with(n: usize) -> crate::Db {
        let db = crate::Db::open_in_memory().unwrap();
        db.with_conn(|c| crate::synthetic::generate(c, n, 11)).unwrap();
        db
    }

    #[test]
    fn slideshows_are_stored_listed_and_deleted_with_their_document() {
        let db = db_with(6);
        db.with_conn(|c| {
            let photos: Vec<i64> = c.prepare("SELECT id FROM assets WHERE media_type = 0 ORDER BY id LIMIT 3")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            let id = create(c, "Lisbon", &SlideshowDoc::new_for(&photos), 10)?;
            let shows = list(c)?;
            assert_eq!(shows.len(), 1);
            assert_eq!((shows[0].name.as_str(), shows[0].clip_count, shows[0].duration_ms), ("Lisbon", 3, 10_800));
            assert_eq!(shows[0].cover_asset_id, Some(photos[0]));

            let (name, mut doc) = get(c, id)?;
            assert_eq!(name, "Lisbon");
            doc.clips.pop();
            save(c, id, "Lisbon, short", &doc, 20)?;
            assert_eq!(get(c, id)?.1.clips.len(), 2);
            assert_eq!(list(c)?[0].updated_at, 20);

            let removed = delete(c, id)?.expect("the deleted show, for Undo");
            assert_eq!(removed.0, "Lisbon, short");
            assert!(list(c)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn music_in_use_names_every_shows_track_but_the_films_own() {
        let db = db_with(3);
        db.with_conn(|c| {
            let with = |path: &str| SlideshowDoc {
                audio: Some(AudioTrack { path: path.into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 0 }),
                ..SlideshowDoc::new_for(&[1])
            };
            create(c, "A", &with("music/111.mp3"), 1)?;
            create(c, "B", &with("C:/Users/me/song.mp3"), 1)?;
            create(c, "Silent", &SlideshowDoc::new_for(&[1]), 1)?;
            c.execute_batch("INSERT INTO memories (id, kind, title, start_at, end_at, generated_at) VALUES (7, 1, 'Lisbon', 0, 10, 0);")?;
            crate::memory_films::put(c, 7, "k", &with("music/222.mp3"), None, 1)?;
            let all = music_in_use(c, None)?;
            assert_eq!(all, ["111.mp3", "song.mp3", "222.mp3"].map(String::from).into());
            let rebuilding = music_in_use(c, Some(7))?;
            assert!(!rebuilding.contains("222.mp3"), "a film being made again may keep its own tune");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clips_of_photos_that_are_gone_are_dropped() {
        let db = db_with(6);
        db.with_conn(|c| {
            let photos: Vec<i64> = c.prepare("SELECT id FROM assets WHERE media_type = 0 ORDER BY id LIMIT 3")?.query_map([], |r| r.get(0))?.collect::<std::result::Result<_, _>>()?;
            let id = create(c, "Show", &SlideshowDoc::new_for(&photos), 10)?;
            c.execute("UPDATE assets SET state = 1 WHERE id = ?1", [photos[1]])?;
            let (_, doc) = get(c, id)?;
            assert_eq!(doc.clips.iter().map(|c| c.asset_id).collect::<Vec<_>>(), [photos[0], photos[2]]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_show_from_a_memory_frames_portraits_smartly() {
        let db = db_with(0);
        db.with_conn(|c| {
            // A busy day of 12: the first landscape, the rest tall phone portraits.
            for i in 0..12 {
                let (w, h) = if i == 0 { (1200, 800) } else { (900, 1600) };
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, width, height)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1, ?3, ?4, ?5)",
                    rusqlite::params![i + 1, format!("h{i}"), 1_714_816_800 + i * 600, w, h],
                )?;
            }
            crate::memories::regenerate(c, chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(), 1)?;
            let memory = crate::memories::list_memories(c, false)?.remove(0);
            let id = from_memory(c, memory.id, 5, LayoutStyle::Smart, None)?;
            let (_, doc) = get(c, id)?;

            assert_eq!(doc.clips[0].frame, FrameKind::Fill, "the landscape fills the frame");
            // No portrait is ever cropped: each one is in a grid, or whole over a backdrop.
            assert!(
                doc.clips[1..].iter().all(|c| matches!(c.frame, FrameKind::Grid | FrameKind::Blur)),
                "{:?}",
                doc.clips.iter().map(|c| c.frame).collect::<Vec<_>>()
            );
            assert_eq!(doc.clips[1].extra.len(), 2, "three tall phone photos to a frame: {:?}", doc.clips[1]);
            // Seven portraits: two grids of three, and the odd one out whole on a backdrop.
            assert_eq!(doc.clips.last().unwrap().frame, FrameKind::Blur);
            // Every highlight is still in the show, exactly once.
            let mut shown: Vec<i64> = doc.clips.iter().flat_map(|c| c.assets()).collect();
            shown.sort();
            assert_eq!(shown.len(), memory.curated_count as usize);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_memory_becomes_a_show_of_its_highlights_in_time_order() {
        let db = db_with(0);
        db.with_conn(|c| {
            // A busy day: 12 photos, so one event memory.
            for i in 0..12 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, is_favorite)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1, ?3, ?4)",
                    rusqlite::params![i + 1, format!("h{i}"), 1_714_816_800 + i * 600, (i == 7) as i64],
                )?;
            }
            crate::memories::regenerate(c, chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(), 1)?;
            let memory = crate::memories::list_memories(c, false)?.remove(0);
            let id = from_memory(c, memory.id, 5, LayoutStyle::Smart, None)?;
            let (name, doc) = get(c, id)?;
            assert_eq!(name, memory.title);
            let ids: Vec<i64> = doc.clips.iter().map(|c| c.asset_id).collect();
            assert_eq!(ids.len(), memory.curated_count as usize);
            assert!(ids.contains(&8), "the favourite is a highlight: {ids:?}");
            // Curation spreads the highlights across the occasion, so these eight are far
            // enough apart to be moments of their own — and moments are never reordered.
            // (That the best photograph leads *within* a moment is `moments_stay_in_time_order`.)
            assert!(ids.windows(2).all(|w| w[0] < w[1]), "the day stays in order: {ids:?}");
            Ok(())
        })
        .unwrap();
    }


    #[test]
    fn a_memory_spanning_hours_keeps_its_hours_in_order() {
        let db = db_with(0);
        db.with_conn(|c| {
            // Two clusters four hours apart, the later one holding the favourite.
            for i in 0..14i64 {
                let at = if i < 7 { 1_714_816_800 + i * 300 } else { 1_714_816_800 + 14_400 + i * 300 };
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, is_favorite)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1, ?3, ?4)",
                    rusqlite::params![i + 1, format!("h{i}"), at, (i == 10) as i64],
                )?;
            }
            crate::memories::regenerate(c, chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(), 1)?;
            let memory = crate::memories::list_memories(c, false)?.remove(0);
            let id = from_memory(c, memory.id, 5, LayoutStyle::Smart, None)?;
            let ids: Vec<i64> = get(c, id)?.1.clips.iter().flat_map(|x| x.assets()).collect();
            // The favourite leads its own moment, never the whole show: morning first.
            let first_late = ids.iter().position(|i| *i > 7);
            let last_early = ids.iter().rposition(|i| *i <= 7);
            if let (Some(first_late), Some(last_early)) = (first_late, last_early) {
                assert!(last_early < first_late, "the afternoon must not jump the morning: {ids:?}");
            }
            Ok(())
        })
        .unwrap();
    }

    /// A memory's show carries whatever music it was handed, faded in and out so it does
    /// not start abruptly; handed none, it stays silent.
    #[test]
    fn a_memory_can_come_with_music() {
        let db = db_with(0);
        db.with_conn(|c| {
            for i in 0..12 {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at)
                     VALUES (?1, ?2, 'p', 'f.jpg', 'jpg', 0, 1, 1, 1, ?3)",
                    rusqlite::params![i + 1, format!("h{i}"), 1_714_816_800 + i * 600],
                )?;
            }
            crate::memories::regenerate(c, chrono::NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(), 1)?;
            let memory = crate::memories::list_memories(c, false)?.remove(0);

            let track = AudioTrack { path: "music/1886711.mp3".into(), fade_in_ms: 1000, fade_out_ms: 1500, gain_db: 0.0, start_ms: 0 };
            let with = from_memory(c, memory.id, 5, LayoutStyle::Smart, Some(track.clone()))?;
            assert_eq!(get(c, with)?.1.audio, Some(track));

            let without = from_memory(c, memory.id, 5, LayoutStyle::Smart, None)?;
            assert_eq!(get(c, without)?.1.audio, None, "no music in the library, no music in the show");
            Ok(())
        })
        .unwrap();
    }


    #[test]
    fn four_photos_share_a_square_grid() {
        let p = frame_layout(FrameKind::Quad, &[0.75, 0.75, 0.75, 0.75], FRAME);
        assert_eq!(p.len(), 5, "a backdrop and four photos");
        assert!(p[0].blurred);
        let cells: Vec<Rect> = p[1..].iter().map(|x| x.rect).collect();
        // Two rows of two, in reading order.
        assert!(cells[0].x < cells[1].x && cells[0].y < cells[2].y, "{cells:?}");
        assert!((cells[0].y - cells[1].y).abs() < 1e-6 && (cells[2].y - cells[3].y).abs() < 1e-6);
        for c in &cells {
            assert!(c.x >= -1e-6 && c.y >= -1e-6 && c.x + c.w <= 1.0 + 1e-6 && c.y + c.h <= 1.0 + 1e-6, "{c:?}");
        }
        assert_eq!(p[1..].iter().map(|x| x.photo).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    }

    #[test]
    fn a_feature_gives_the_first_photo_the_most_room() {
        let p = frame_layout(FrameKind::Feature, &[1.5, 0.75, 0.75], FRAME);
        assert_eq!(p.len(), 4);
        let (hero, a, b) = (p[1].rect, p[2].rect, p[3].rect);
        assert!(hero.w * hero.h > a.w * a.h * 2.0, "the hero dominates: {hero:?} vs {a:?}");
        assert!(hero.x + hero.w <= a.x + 1e-6, "the small ones are beside it");
        assert!(a.y + a.h <= b.y + 1e-6, "and stacked");
    }

    #[test]
    fn two_panoramas_stack() {
        let p = frame_layout(FrameKind::Stack, &[3.0, 3.0], FRAME);
        assert_eq!(p.len(), 3);
        let (top, bottom) = (p[1].rect, p[2].rect);
        assert!(top.y + top.h <= bottom.y + 1e-6, "one above the other");
        // Each band is half the frame high, and the photograph is fitted whole inside it:
        // a 3.0 panorama is narrower than the 3.6-shaped band, so it keeps its own shape.
        for r in [top, bottom] {
            assert!(r.h <= 0.5 + 1e-6 && r.w <= 1.0 + 1e-6, "{r:?}");
            assert!((r.x + r.w / 2.0 - 0.5).abs() < 1e-6, "centred: {r:?}");
        }
        // A wider photograph does fill the band's width.
        let wide = frame_layout(FrameKind::Stack, &[4.0, 4.0], FRAME);
        assert!((wide[1].rect.w - 1.0).abs() < 1e-6, "{:?}", wide[1].rect);
    }

    #[test]
    fn a_frame_never_shows_more_photos_than_it_holds() {
        for kind in [FrameKind::Fill, FrameKind::Blur, FrameKind::Grid, FrameKind::Quad, FrameKind::Feature, FrameKind::Stack] {
            let p = frame_layout(kind, &[1.0; 6], FRAME);
            let shown = p.iter().filter(|x| !x.blurred).count();
            assert_eq!(shown, kind.capacity(), "{kind:?} drew {shown}");
        }
    }

    #[test]
    fn the_builder_reaches_for_the_bigger_layouts() {
        // Four tall phone photos: Smart fills one frame with three of them, because three
        // 9:16 photographs fit a 16:9 frame exactly, and shows the fourth whole.
        let smart = build(&shapes(&[0.5625, 0.5625, 0.5625, 0.5625]), LayoutStyle::Smart);
        assert_eq!(smart.clips.iter().map(|c| c.frame).collect::<Vec<_>>(), vec![FrameKind::Grid, FrameKind::Blur]);
        assert_eq!(smart.clips[0].assets().len(), 3);

        // Asked to group, it puts all four in one frame instead.
        let quad = build(&shapes(&[0.5625, 0.5625, 0.5625, 0.5625]), LayoutStyle::Grid);
        assert_eq!(quad.clips.len(), 1, "{:?}", quad.clips.iter().map(|c| c.frame).collect::<Vec<_>>());
        assert_eq!(quad.clips[0].frame, FrameKind::Quad);
        assert_eq!(quad.clips[0].assets().len(), 4);

        let stack = build(&shapes(&[3.0, 3.0]), LayoutStyle::Smart);
        assert_eq!(stack.clips[0].frame, FrameKind::Stack, "two panoramas are stacked, not cropped");

        for style in [LayoutStyle::Smart, LayoutStyle::Random, LayoutStyle::Grid] {
            let d = build(&shapes(&[1.5, 0.75, 0.75, 0.5625, 0.5625, 0.5625, 0.5625, 3.0, 3.0, 1.0]), style);
            let mut seen: Vec<i64> = d.clips.iter().flat_map(|c| c.assets()).collect();
            seen.sort();
            assert_eq!(seen, (1..=10).collect::<Vec<_>>(), "{style:?} lost or repeated a photo");
        }
    }

    #[test]
    fn a_landscape_keeps_the_whole_frame_unless_grouping_was_asked_for() {
        let photos = shapes(&[1.5, 0.75, 0.75]);
        // Smart: the landscape fills the frame, the two portraits share the next one.
        let smart = build(&photos, LayoutStyle::Smart);
        assert_eq!(smart.clips.iter().map(|c| c.frame).collect::<Vec<_>>(), vec![FrameKind::Fill, FrameKind::Grid]);
        // Grid: all three together, the landscape as the hero.
        let grid = build(&photos, LayoutStyle::Grid);
        assert_eq!(grid.clips.len(), 1);
        assert_eq!(grid.clips[0].frame, FrameKind::Feature);
        assert_eq!(grid.clips[0].assets(), vec![1, 2, 3]);
    }

    #[test]
    fn normalizing_trims_a_clip_to_what_its_frame_holds() {
        let mut d = SlideshowDoc::new_for(&[1]);
        d.clips[0].frame = FrameKind::Stack;
        d.clips[0].extra = vec![2, 3, 4];
        assert_eq!(d.clone().normalized().clips[0].extra, vec![2], "a stack holds two");
        d.clips[0].frame = FrameKind::Quad;
        assert_eq!(d.normalized().clips[0].extra, vec![2, 3, 4], "a quad holds four");
    }


    fn orderable(id: i64, at: i64, aspect: f32, score: f32, phash: u64) -> Orderable {
        Orderable { id, aspect, captured_at: at, score, phash: Some(phash), vec: None, color: None, video: None }
    }

    /// A perceptual hash that is nothing like its neighbours. Small integers are all within
    /// a few bits of each other, which would make every photo a near-duplicate of every
    /// other one and is nothing like what dHash actually produces.
    fn spread(n: u64) -> u64 {
        n.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    #[test]
    fn moments_stay_in_time_order() {
        // Two moments an hour apart; the second must never precede the first.
        let photos = vec![
            orderable(1, 1000, 1.5, 0.1, spread(1)),
            orderable(2, 1020, 1.5, 0.9, spread(2)),
            orderable(3, 5000, 1.5, 0.5, spread(3)),
            orderable(4, 5020, 1.5, 0.2, spread(4)),
        ];
        let ids: Vec<i64> = arrange(&photos).iter().map(|p| p.id).collect();
        assert!(ids[..2].contains(&1) && ids[..2].contains(&2), "the first moment first: {ids:?}");
        assert_eq!(ids[0], 2, "and its best photo leads");
    }

    #[test]
    fn near_duplicates_are_not_left_side_by_side() {
        let same = 0xF0F0_F0F0_F0F0_F0F0u64;
        let photos = vec![
            orderable(1, 1000, 1.5, 0.9, same),
            orderable(2, 1005, 1.5, 0.8, same ^ 0b11),
            orderable(3, 1010, 1.5, 0.7, 0x0F0F_0F0F_0F0F_0F0F),
        ];
        let ids: Vec<i64> = arrange(&photos).iter().map(|p| p.id).collect();
        let (a, b) = (ids.iter().position(|i| *i == 1).unwrap(), ids.iter().position(|i| *i == 2).unwrap());
        assert!(a.abs_diff(b) > 1, "the two near-identical shots are separated: {ids:?}");
    }

    #[test]
    fn portraits_gather_so_the_grids_fill() {
        let photos = vec![
            orderable(1, 1000, 1.5, 0.5, spread(1)),
            orderable(2, 1005, 0.5625, 0.5, spread(2)),
            orderable(3, 1010, 1.5, 0.5, spread(3)),
            orderable(4, 1015, 0.5625, 0.5, spread(4)),
            orderable(5, 1020, 0.5625, 0.5, spread(5)),
            orderable(6, 1025, 1.5, 0.5, spread(6)),
        ];
        let out = arrange(&photos);
        let doc = build(&out.iter().map(PhotoShape::from).collect::<Vec<_>>(), LayoutStyle::Smart);
        assert!(
            doc.clips.iter().any(|c| c.frame == FrameKind::Grid && c.assets().len() == 3),
            "three portraits in one moment should fill one frame: {:?}",
            doc.clips.iter().map(|c| (c.frame, c.assets().len())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn arranging_keeps_every_photograph_exactly_once() {
        let mut photos: Vec<Orderable> = Vec::new();
        let mut seed = 12345u64;
        for i in 0..200i64 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let gap = if i % 17 == 0 { 9000 } else { (seed >> 40) as i64 % 30 };
            let at = photos.last().map(|p| p.captured_at).unwrap_or(0) + gap;
            photos.push(orderable(i + 1, at, if seed.is_multiple_of(3) { 0.75 } else { 1.5 }, (seed % 100) as f32 / 100.0, seed));
        }
        let out = arrange(&photos);
        assert_eq!(out.len(), photos.len());
        let mut ids: Vec<i64> = out.iter().map(|p| p.id).collect();
        ids.sort();
        assert_eq!(ids, (1..=200).collect::<Vec<_>>());
        // And the day is still in order. Work out the moments from the input, then check
        // that each one comes out as a contiguous block, and that the blocks are in time
        // order: that is exactly the promise "chronological at the large scale".
        let mut by_time = photos.clone();
        by_time.sort_by_key(|p| (p.captured_at, p.id));
        let mut moment_of: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
        let mut moment = 0usize;
        for (i, p) in by_time.iter().enumerate() {
            if i > 0 && p.captured_at - by_time[i - 1].captured_at > MOMENT_GAP_S {
                moment += 1;
            }
            moment_of.insert(p.id, moment);
        }
        let moments: Vec<usize> = out.iter().map(|p| moment_of[&p.id]).collect();
        assert!(moments.windows(2).all(|w| w[1] >= w[0]), "a later moment jumped an earlier one");
        let mut seen: Vec<usize> = moments.clone();
        seen.dedup();
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(seen.len(), unique.len(), "a moment was split across the show");
        assert!(moment > 0, "the fixture should contain more than one moment");
    }

    #[test]
    fn arranging_is_stable() {
        let photos = vec![
            orderable(1, 1000, 1.5, 0.5, spread(1)),
            orderable(2, 1005, 0.75, 0.5, spread(2)),
            orderable(3, 1010, 1.5, 0.5, spread(3)),
        ];
        assert_eq!(arrange(&photos), arrange(&photos), "the same photos give the same order");
    }

    #[test]
    fn quality_puts_a_favourite_above_an_unrated_sharp_photo() {
        let fav = quality_score(true, 0, Some(1.0), 100.0);
        let sharp = quality_score(false, 0, Some(100.0), 100.0);
        assert!(fav > sharp, "{fav} vs {sharp}");
        assert!((0.0..=1.0).contains(&quality_score(true, 5, Some(100.0), 100.0)));
        assert_eq!(quality_score(false, 0, None, 0.0), 0.0);
    }


    #[test]
    fn every_cut_lands_on_a_beat() {
        let doc = SlideshowDoc::new_for(&[1, 2, 3, 4]);
        let snapped = snap_to_beats(&doc, 120.0, 250);
        let t = timeline(&snapped);
        let beat = 500.0f64; // 120 BPM
        // The video always starts at zero; it is the audio that is trimmed by 250 ms so a
        // beat lands there. So the music's beats fall at multiples of one beat from zero.
        for (i, start) in t.starts.iter().enumerate().skip(1) {
            let since = *start as f64;
            let off = (since / beat).round() * beat - since;
            assert!(off.abs() < 1.0, "cut {i} at {start} ms is {off:.1} ms off the beat");
        }
        assert_eq!(snapped.audio, None, "this show has no music to trim");
    }

    #[test]
    fn a_clip_is_never_snapped_out_of_its_limits() {
        let mut doc = SlideshowDoc::new_for(&[1, 2]);
        doc.clips[0].duration_ms = MIN_CLIP_MS;
        doc.clips[1].duration_ms = MAX_CLIP_MS;
        let snapped = snap_to_beats(&doc, 40.0, 0).normalized();
        for c in &snapped.clips {
            assert!(c.duration_ms >= MIN_CLIP_MS && c.duration_ms <= MAX_CLIP_MS, "{}", c.duration_ms);
        }
    }

    #[test]
    fn a_clip_always_gets_at_least_a_bar() {
        // At 200 BPM a beat is 300 ms; a four-second clip must not become one beat long.
        let doc = SlideshowDoc::new_for(&[1, 2, 3]);
        let snapped = snap_to_beats(&doc, 200.0, 0);
        let overlaps = timeline(&snapped).overlaps;
        let beat = 60_000.0 / 200.0;
        for (i, c) in snapped.clips.iter().enumerate() {
            let advance = (c.duration_ms - overlaps[i]) as f32;
            assert!(advance / beat >= MIN_BEATS_PER_CLIP as f32 - 0.01, "clip {i} advanced {advance} ms");
        }
    }

    #[test]
    fn the_music_is_trimmed_so_the_run_starts_on_a_beat() {
        let mut doc = SlideshowDoc::new_for(&[1, 2]);
        doc.audio = Some(AudioTrack { path: "m.mp3".into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 0 });
        let snapped = snap_to_beats(&doc, 120.0, 250);
        assert_eq!(snapped.audio.unwrap().start_ms, 250, "the head before the first beat is dropped");
    }

    #[test]
    fn a_tempo_that_makes_no_sense_leaves_the_show_alone() {
        let doc = SlideshowDoc::new_for(&[1, 2, 3]);
        assert_eq!(snap_to_beats(&doc, 0.0, 0), doc);
        assert_eq!(snap_to_beats(&doc, f32::NAN, 0), doc);
        assert_eq!(snap_to_beats(&doc, 1000.0, 0), doc);
    }

    #[test]
    fn snapping_changes_the_timing_but_never_the_pictures() {
        let doc = build(&[PhotoShape { id: 1, aspect: 1.5, video: None }, PhotoShape { id: 2, aspect: 0.75, video: None }, PhotoShape { id: 3, aspect: 0.75, video: None }], LayoutStyle::Smart);
        let snapped = snap_to_beats(&doc, 128.0, 0);
        assert_eq!(
            doc.clips.iter().map(|c| (c.assets(), c.frame, c.transition.kind)).collect::<Vec<_>>(),
            snapped.clips.iter().map(|c| (c.assets(), c.frame, c.transition.kind)).collect::<Vec<_>>()
        );
    }

    fn grid_doc(durations: &[u32], transition: u32) -> SlideshowDoc {
        SlideshowDoc {
            version: 1,
            resolution: [1920, 1080],
            fps: 30,
            audio: Some(AudioTrack { path: "m.mp3".into(), fade_in_ms: 0, fade_out_ms: 0, gain_db: 0.0, start_ms: 0 }),
            clips: durations
                .iter()
                .enumerate()
                .map(|(i, d)| Clip {
                    asset_id: i as i64,
                    extra: vec![],
                    frame: FrameKind::Fill,
                    duration_ms: *d,
                    transition: Transition { kind: TransitionKind::Crossfade, duration_ms: transition, auto: false },
                    motion: Motion::none(),
                    video: None,
                })
                .collect(),
        }
    }

    /// Cuts are absolute beat times, so a tempo that moves is followed and no error accumulates.
    #[test]
    fn cuts_land_on_the_beats_of_a_moving_tempo() {
        let mut beats = vec![300u32];
        while *beats.last().unwrap() < 40_000 {
            let t = *beats.last().unwrap();
            beats.push(t + if t < 20_000 { 500 } else { 450 });
        }
        let grid = BeatGrid { beats_ms: beats.clone(), downbeats_ms: vec![], bpm: 125.0, confidence: 1.0 };
        let out = snap_to_beat_grid(&grid_doc(&[4000; 8], 600), &grid);
        let tl = timeline(&out);
        let start_ms = out.audio.as_ref().unwrap().start_ms;
        assert_eq!(start_ms, 300, "the track starts on its first beat");
        for s in &tl.starts[1..] {
            let t = s + start_ms;
            assert!(beats.iter().any(|b| b.abs_diff(t) <= 1), "cut at {t} ms is off the beat");
        }
    }

    #[test]
    fn prefers_a_downbeat_within_one_beat_of_the_target() {
        let beats: Vec<u32> = (0..100).map(|k| k * 500).collect();
        let downbeats: Vec<u32> = beats.iter().copied().step_by(4).collect();
        let grid = BeatGrid { beats_ms: beats, downbeats_ms: downbeats.clone(), bpm: 120.0, confidence: 1.0 };
        // Each clip wants to advance 3.8 s; the downbeat at 4 s is within a beat of that.
        let out = snap_to_beat_grid(&grid_doc(&[4400; 5], 600), &grid);
        for s in &timeline(&out).starts[1..] {
            assert!(downbeats.contains(s), "{s} is not a downbeat");
        }
    }

    /// Round 6: a show that starts at the chorus is cut to the chorus's beats, not the intro's.
    #[test]
    fn a_show_starting_mid_song_keeps_its_place() {
        let grid = BeatGrid::even(120.0, 250, 120_000);
        let mut doc = grid_doc(&[4000; 4], 600);
        doc.audio.as_mut().unwrap().start_ms = 30_100;
        let out = snap_to_beat_grid(&doc, &grid);
        assert_eq!(out.audio.as_ref().unwrap().start_ms, 30_250, "the first beat at or after the chosen start");
        let again = snap_to_beat_grid(&out, &grid);
        assert_eq!(again.audio.as_ref().unwrap().start_ms, 30_250, "syncing twice changes nothing");
    }

    #[test]
    fn an_empty_grid_leaves_the_show_alone() {
        let doc = grid_doc(&[4000; 3], 600);
        assert_eq!(snap_to_beat_grid(&doc, &BeatGrid::none()), doc);
    }

    #[test]
    fn transitions_last_half_a_beat() {
        assert_eq!(beat_transition_ms(&BeatGrid::even(120.0, 0, 10_000)), 250);
        assert_eq!(beat_transition_ms(&BeatGrid::even(60.0, 0, 10_000)), 500);
        assert_eq!(beat_transition_ms(&BeatGrid::even(30.0, 0, 30_000)), 900);
    }

    // ---- Phase 7a: grouping by content ----

    fn tilt(axis: usize, other: usize, t: f32) -> crate::embedding::Vector {
        let mut v = vec![0f32; crate::embedding::DIM];
        v[axis] = (1.0 - t * t).sqrt();
        v[other] = t;
        crate::embedding::quantize(&v)
    }

    fn o(id: i64, aspect: f32, t: i64, score: f32, vec: Option<crate::embedding::Vector>) -> Orderable {
        Orderable { id, aspect, captured_at: t, score, phash: None, vec, color: None, video: None }
    }

    fn ids(groups: &[Group]) -> Vec<Vec<i64>> {
        groups.iter().map(|g| g.photos.iter().map(|p| p.id).collect()).collect()
    }

    #[test]
    fn portraits_of_the_same_scene_share_a_frame() {
        // Four portraits in one moment: two of a beach (axis 0), two of a menu (axis 1). A tilt
        // of 0.4 is cosine 0.92 — the same scene, but not the near-twins (≥ 0.96) kept apart.
        let m = vec![
            o(1, 0.75, 0, 0.5, Some(tilt(0, 1, 0.0))),
            o(2, 0.75, 60, 0.5, Some(tilt(1, 0, 0.0))),
            o(3, 0.75, 120, 0.5, Some(tilt(0, 1, 0.4))),
            o(4, 0.75, 180, 0.5, Some(tilt(1, 0, 0.4))),
        ];
        let groups = group_moment(&m, LayoutStyle::Smart);
        let together = |a: i64, b: i64| groups.iter().any(|g| g.photos.iter().any(|x| x.id == a) && g.photos.iter().any(|x| x.id == b));
        assert!(together(1, 3) && together(2, 4), "{:?}", ids(&groups));
    }

    #[test]
    fn the_best_photograph_of_a_moment_is_shown_alone_in_smart() {
        let m: Vec<Orderable> = (0..8).map(|i| o(i, 0.75, i * 30, if i == 5 { 0.95 } else { 0.3 }, None)).collect();
        let groups = group_moment(&m, LayoutStyle::Smart);
        assert!(groups.iter().any(|g| g.photos.len() == 1 && g.photos[0].id == 5), "{:?}", ids(&groups));
    }

    /// With no ratings every score ties, and nobody is a hero: portraits still pair up.
    #[test]
    fn equal_photographs_have_no_hero() {
        let m: Vec<Orderable> = (0..4).map(|i| o(i, 0.75, i * 30, 0.0, None)).collect();
        assert!(group_moment(&m, LayoutStyle::Smart).iter().all(|g| g.photos.len() >= 2), "{:?}", ids(&group_moment(&m, LayoutStyle::Smart)));
    }

    #[test]
    fn near_twins_are_never_side_by_side() {
        let twin = Some(tilt(0, 1, 0.0));
        let m = vec![o(1, 0.75, 0, 0.5, twin.clone()), o(2, 0.75, 10, 0.5, twin), o(3, 0.75, 20, 0.5, Some(tilt(2, 0, 0.0)))];
        for g in group_moment(&m, LayoutStyle::Grid) {
            assert!(!(g.photos.iter().any(|x| x.id == 1) && g.photos.iter().any(|x| x.id == 2)));
        }
    }

    #[test]
    fn smart_shows_a_landscape_alone_and_grid_groups_it() {
        let m: Vec<Orderable> = (0..4).map(|i| o(i, 1.5, i * 30, 0.0, None)).collect();
        assert!(group_moment(&m, LayoutStyle::Smart).iter().all(|g| g.frame == FrameKind::Fill));
        let grid = group_moment(&m, LayoutStyle::Grid);
        assert_eq!(grid.len(), 1);
        assert_eq!(grid[0].frame, FrameKind::Quad);
    }

    #[test]
    fn every_photograph_appears_once_and_the_frames_hold_them() {
        let photos: Vec<Orderable> = (0..30).map(|i| o(i, if i % 3 == 0 { 1.5 } else { 0.75 }, i * 400, (i % 7) as f32 / 7.0, None)).collect();
        for style in [LayoutStyle::Smart, LayoutStyle::Grid, LayoutStyle::Random, LayoutStyle::Fill] {
            let doc = build_arranged(&photos, style);
            let mut seen: Vec<i64> = doc.clips.iter().flat_map(|c| c.assets()).collect();
            seen.sort();
            assert_eq!(seen, (0..30).collect::<Vec<_>>(), "{style:?}");
            for c in &doc.clips {
                assert!(c.assets().len() <= c.frame.capacity(), "{style:?}: {:?} holds {}", c.frame, c.assets().len());
                assert_eq!(c.frame.is_multi(), c.assets().len() > 1, "{style:?}: {:?}", c.frame);
            }
        }
    }

    #[test]
    fn colours_that_clash_are_less_compatible() {
        let mut a = o(1, 0.75, 0, 0.5, None);
        let mut near = o(2, 0.75, 0, 0.5, None);
        let mut far = o(3, 0.75, 0, 0.5, None);
        a.color = Some([200, 40, 40]);
        near.color = Some([190, 50, 45]);
        far.color = Some([30, 200, 60]);
        assert!(compat(&a, &near) > compat(&a, &far));
    }

    // ---- Phase 7a: transitions chosen for the join ----

    fn ctx() -> JoinContext {
        JoinContext { similar: None, new_moment: false, grid_either_side: false, on_downbeat: false, energy: 0.0, luma_out: None, luma_in: None }
    }

    #[test]
    fn a_match_cut_dissolves() {
        assert_eq!(choose_transition(&JoinContext { similar: Some(0.9), ..ctx() }, &[], 0), TransitionKind::Crossfade);
    }

    #[test]
    fn a_new_moment_fades_through_black() {
        assert_eq!(choose_transition(&JoinContext { new_moment: true, ..ctx() }, &[], 0), TransitionKind::FadeBlack);
    }

    #[test]
    fn a_grid_arrives_in_pieces() {
        for tie in 0..12 {
            let k = choose_transition(&JoinContext { grid_either_side: true, ..ctx() }, &[], tie);
            assert!(GRID_SHUFFLE.contains(&k), "{k:?}");
        }
    }

    #[test]
    fn a_loud_downbeat_cuts_hard_or_moves() {
        for tie in 0..12 {
            let k = choose_transition(&JoinContext { on_downbeat: true, energy: 0.8, ..ctx() }, &[], tie);
            assert!(matches!(k, TransitionKind::None | TransitionKind::Zoom | TransitionKind::Slide), "{k:?}");
        }
    }

    #[test]
    fn into_a_bright_picture_fades_through_white() {
        assert_eq!(choose_transition(&JoinContext { luma_out: Some(0.2), luma_in: Some(0.9), ..ctx() }, &[], 0), TransitionKind::FadeWhite);
        assert_eq!(choose_transition(&JoinContext { luma_out: Some(0.9), luma_in: Some(0.2), ..ctx() }, &[], 0), TransitionKind::FadeBlack);
    }

    #[test]
    fn never_the_same_join_three_times_running() {
        let k = choose_transition(&JoinContext { similar: Some(0.9), ..ctx() }, &[TransitionKind::Crossfade, TransitionKind::Crossfade], 0);
        assert_ne!(k, TransitionKind::Crossfade);
    }

    #[test]
    fn with_nothing_to_go_on_a_cut_or_a_fade_is_never_the_tie_break() {
        for tie in 0..40 {
            let k = choose_transition(&ctx(), &[], tie);
            assert!(!matches!(k, TransitionKind::None | TransitionKind::FadeBlack | TransitionKind::FadeWhite), "{k:?}");
        }
    }

    #[test]
    fn a_hand_picked_transition_is_never_replaced() {
        let mut doc = build(&[PhotoShape { id: 1, aspect: 1.5, video: None }, PhotoShape { id: 2, aspect: 1.5, video: None }], LayoutStyle::Random);
        assert!(doc.clips.iter().all(|c| c.transition.auto), "the builder marks its own choices");
        doc.clips[0].transition = Transition { kind: TransitionKind::Squeeze, duration_ms: 700, auto: false };
        let out = retransition(&doc, &HashMap::new(), Some(&BeatGrid::even(120.0, 0, 20_000)), 0.9);
        assert_eq!(out.clips[0].transition, doc.clips[0].transition);
    }

    #[test]
    fn a_beat_sync_retimes_and_rechooses_the_automatic_joins() {
        let doc = build(&[PhotoShape { id: 1, aspect: 1.5, video: None }, PhotoShape { id: 2, aspect: 1.5, video: None }, PhotoShape { id: 3, aspect: 1.5, video: None }], LayoutStyle::Random);
        let out = retransition(&doc, &HashMap::new(), Some(&BeatGrid::even(120.0, 0, 20_000)), 0.9);
        // The last clip joins nothing, so its transition is never used and never touched.
        assert!(out.clips[..2].iter().all(|c| c.transition.duration_ms == 250), "half a beat at 120 BPM");
    }

    #[test]
    fn documents_saved_before_this_phase_count_as_hand_picked() {
        let t: Transition = serde_json::from_str(r#"{"kind":"crossfade","duration_ms":600}"#).unwrap();
        assert!(!t.auto);
        assert_eq!(serde_json::to_string(&t).unwrap(), r#"{"kind":"crossfade","duration_ms":600}"#, "a hand-picked join is written as before");
    }

    #[test]
    fn an_audio_track_saved_before_beat_sync_still_parses() {
        let json = r#"{"path":"m.mp3","fade_in_ms":1000,"fade_out_ms":1500,"gain_db":0.0}"#;
        let a: AudioTrack = serde_json::from_str(json).unwrap();
        assert_eq!(a.start_ms, 0);
    }

    // Round 6: videos in slideshows.

    fn video_shape(id: i64, aspect: f32, from: u32, to: u32) -> PhotoShape {
        PhotoShape { id, aspect, video: Some(VideoSpan { in_ms: from, out_ms: to }) }
    }

    #[test]
    fn a_video_plays_alone_for_as_long_as_its_best_part() {
        let photos = [
            PhotoShape { id: 1, aspect: 0.75, video: None },
            video_shape(2, 0.56, 4_000, 9_000),
            PhotoShape { id: 3, aspect: 0.75, video: None },
            PhotoShape { id: 4, aspect: 0.75, video: None },
            video_shape(5, 1.78, 1_000, 4_500),
        ];
        for style in [LayoutStyle::Smart, LayoutStyle::Grid, LayoutStyle::Random] {
            let doc = build(&photos, style);
            let video = doc.clips.iter().find(|c| c.asset_id == 2).expect("the video has its own clip");
            assert!(video.extra.is_empty(), "{style:?}: a video is never in a grid");
            assert_eq!(video.frame, FrameKind::Blur, "an upright video plays over its blurred self");
            assert_eq!(video.video, Some(VideoSpan { in_ms: 4_000, out_ms: 9_000 }));
            assert_eq!(video.duration_ms, 5_000);
            assert_eq!(video.motion.kind, MotionKind::None, "a video moves by itself");
            let wide = doc.clips.iter().find(|c| c.asset_id == 5).unwrap();
            assert_eq!(wide.frame, FrameKind::Fill);
            assert!(doc.clips.iter().filter(|c| c.video.is_none()).all(|c| !c.extra.contains(&2) && !c.extra.contains(&5)));
        }
    }

    #[test]
    fn grouping_a_moment_never_takes_a_video_into_a_grid() {
        let mut moment: Vec<Orderable> = (1..=5).map(|i| o(i, 0.75, i * 10, 0.5, None)).collect();
        moment[2].video = Some(VideoSpan { in_ms: 0, out_ms: 4_000 });
        for style in [LayoutStyle::Smart, LayoutStyle::Grid] {
            let groups = group_moment(&moment, style);
            let with_video = groups.iter().find(|g| g.photos.iter().any(|p| p.id == 3)).unwrap();
            assert_eq!(with_video.photos.len(), 1, "{style:?}");
        }
    }

    #[test]
    fn a_video_clip_is_normalized_to_play_alone() {
        let mut doc = SlideshowDoc::new_for(&[1]);
        doc.clips[0] = Clip {
            extra: vec![2, 3],
            frame: FrameKind::Grid,
            motion: Motion::zoom_in(0.01, 0.01),
            video: Some(VideoSpan { in_ms: 5_000, out_ms: 2_000 }),
            ..doc.clips[0].clone()
        };
        let c = &doc.normalized().clips[0];
        assert!(c.extra.is_empty());
        assert_eq!(c.frame, FrameKind::Fill);
        assert_eq!(c.motion.kind, MotionKind::None);
        assert!(c.video.unwrap().out_ms > c.video.unwrap().in_ms);
    }

    #[test]
    fn a_photo_clip_is_saved_exactly_as_before() {
        let json = serde_json::to_string(&SlideshowDoc::new_for(&[1, 2])).unwrap();
        assert!(!json.contains("video"), "{json}");
    }

    #[test]
    fn a_show_may_start_well_into_its_track() {
        let mut doc = grid_doc(&[4000], 600);
        doc.audio.as_mut().unwrap().start_ms = 95_000;
        assert_eq!(doc.clone().normalized().audio.unwrap().start_ms, 95_000);
        doc.audio.as_mut().unwrap().start_ms = u32::MAX;
        assert_eq!(doc.normalized().audio.unwrap().start_ms, MAX_AUDIO_START_MS);
    }

    #[test]
    fn videos_bring_their_best_part_from_the_catalogue() {
        let db = db_with(0);
        db.with_conn(|c| {
            for (id, media_type, w, h) in [(1, 0, 4000, 3000), (2, 1, 1080, 1920), (3, 1, 1920, 1080)] {
                c.execute(
                    "INSERT INTO assets (id, content_hash, relative_path, filename, extension, media_type, byte_size, orientation, imported_at, captured_at, width, height, duration_ms)
                     VALUES (?1, ?2, ?3, 'f', 'x', ?4, 1, 1, ?1, ?1, ?5, ?6, 20000)",
                    params![id, format!("h{id}"), format!("p{id}"), media_type, w, h],
                )?;
            }
            let h = crate::video_highlight::Highlight { in_ms: 7_000, out_ms: 11_000, score: 0.8 };
            crate::video_highlight::put(c, 2, "frames@1", &h, 1)?;
            let spans = video_spans(c, &[1, 2, 3])?;
            assert!(!spans.contains_key(&1), "a photograph has no span");
            assert_eq!(spans[&2], (VideoSpan { in_ms: 7_000, out_ms: 11_000 }, Some(0.8)));
            assert_eq!(spans[&3].0, VideoSpan { in_ms: 0, out_ms: 6_000 }, "not looked at yet: its opening");
            assert_eq!(with_ready_videos(c, &[1, 2, 3])?, vec![1, 2], "an automatic show waits for no video");
            let o = orderables_for(c, &[2])?;
            assert_eq!(o[0].video, Some(VideoSpan { in_ms: 7_000, out_ms: 11_000 }));
            assert!((o[0].score - 0.9).abs() < 1e-6);
            // Saved and read back, the video clip stays; one with no span is dropped.
            let doc = build(&shapes_for(c, &[1, 2])?, LayoutStyle::Smart);
            let id = create(c, "With a video", &doc, 1)?;
            assert_eq!(get(c, id)?.1.clips.len(), 2);
            let mut old = doc.clone();
            old.clips[1].video = None;
            save(c, id, "Old", &old, 2)?;
            assert_eq!(get(c, id)?.1.clips.len(), 1);
            Ok(())
        })
        .unwrap();
    }
}
