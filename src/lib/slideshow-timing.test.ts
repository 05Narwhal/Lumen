import { describe, expect, it } from "vitest";
import type { FrameKind } from "@/types/generated/FrameKind";
import type { TransitionKind } from "@/types/generated/TransitionKind";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import golden from "./slideshow-golden.json";
import { ease, frameAt, frameLayout, motionAt, SLICE_COUNT, timeline, TRANSITION_KINDS, videoTimeAt } from "./slideshow-timing";

/** The preview and the export must agree, so Rust writes the answers and we check them. */
describe("slideshow timing against the Rust golden file", () => {
  it("has cases to check", () => {
    expect(golden.cases.length).toBeGreaterThan(0);
  });

  for (const [i, c] of golden.cases.entries()) {
    it(`case ${i}: the timeline matches Rust`, () => {
      const doc = c.doc as unknown as SlideshowDoc;
      expect(timeline(doc)).toEqual(c.timeline);
    });

    it(`case ${i}: every motion sample matches Rust`, () => {
      const doc = c.doc as unknown as SlideshowDoc;
      c.samples.forEach((clipSamples, clip) => {
        for (const [u, x, y, scale] of clipSamples) {
          const got = motionAt(doc.clips[clip].motion, u);
          expect(got[0]).toBeCloseTo(x, 5);
          expect(got[1]).toBeCloseTo(y, 5);
          expect(got[2]).toBeCloseTo(scale, 5);
        }
      });
    });
  }
});

describe("frameLayout against the Rust golden file", () => {
  it("has frame cases to check", () => {
    expect(golden.frames.length).toBeGreaterThan(0);
  });

  for (const [i, c] of golden.frames.entries()) {
    it(`frame case ${i} (${c.kind}, ${c.aspects.length} photo(s)) matches Rust`, () => {
      const got = frameLayout(c.kind as FrameKind, c.aspects, c.frameAspect);
      expect(got).toHaveLength(c.placements.length);
      got.forEach((p, n) => {
        expect(p.photo).toBe(c.placements[n].photo);
        expect(p.blurred).toBe(c.placements[n].blurred);
        for (const k of ["x", "y", "w", "h"] as const) {
          expect(p.rect[k]).toBeCloseTo(c.placements[n].rect[k], 5);
        }
      });
    });
  }
});

describe("ease", () => {
  it("runs from 0 to 1 through 0.5 and is symmetric", () => {
    expect(ease(0)).toBe(0);
    expect(ease(0.5)).toBeCloseTo(0.5, 6);
    expect(ease(1)).toBe(1);
    expect(ease(0.25) + ease(0.75)).toBeCloseTo(1, 6);
  });

  it("clamps outside 0..1", () => {
    expect(ease(-2)).toBe(0);
    expect(ease(7)).toBe(1);
  });
});

const doc = (kind: SlideshowDoc["clips"][number]["transition"]["kind"], frame: FrameKind = "fill"): SlideshowDoc => ({
  version: 1,
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: [1, 2].map((asset_id) => ({
    asset_id,
    extra: [],
    frame,
    duration_ms: 4000,
    transition: { kind, duration_ms: 1000 },
    motion: { kind: "none", from: [0, 0, 1], to: [0, 0, 1] },
  })),
});

describe("frameAt", () => {
  it("shows one clip outside a transition", () => {
    const { layers } = frameAt(doc("crossfade"), 1000);
    expect(layers).toHaveLength(1);
    expect(layers[0].index).toBe(0);
    expect(layers[0].alpha).toBe(1);
    expect(layers[0].offsetX).toBe(0);
  });

  // The transition runs 3000..4000; its midpoint is 3500.
  it("crossfades two layers at half alpha", () => {
    const { layers } = frameAt(doc("crossfade"), 3500);
    expect(layers.map((l) => l.index)).toEqual([0, 1]);
    expect(layers[0].alpha).toBeCloseTo(0.5, 6);
    expect(layers[1].alpha).toBeCloseTo(0.5, 6);
  });

  it("slides the outgoing left and the incoming in from the right", () => {
    const { layers } = frameAt(doc("slide"), 3500);
    expect(layers[0].offsetX).toBeCloseTo(-0.5, 6);
    expect(layers[1].offsetX).toBeCloseTo(0.5, 6);
    expect(layers[0].alpha).toBe(1);
    expect(layers[1].alpha).toBe(1);
  });

  it("cuts with no transition", () => {
    const d = doc("none");
    expect(frameAt(d, 3999).layers.map((l) => l.index)).toEqual([0]);
    expect(frameAt(d, 4001).layers.map((l) => l.index)).toEqual([1]);
    expect(timeline(d).totalMs).toBe(8000);
  });

  it("holds the last frame at the end and the first before the start", () => {
    const d = doc("crossfade");
    expect(frameAt(d, timeline(d).totalMs + 500).layers.map((l) => l.index)).toEqual([1]);
    expect(frameAt(d, -100).layers.map((l) => l.index)).toEqual([0]);
  });

  it("has no layers for an empty show", () => {
    expect(frameAt({ ...doc("crossfade"), clips: [] }, 0).layers).toEqual([]);
  });
});

describe("the transitions added for frames", () => {
  it("fades through black at the midpoint", () => {
    const { layers, backdrop } = frameAt(doc("fadeblack"), 3500);
    expect(backdrop).toBe("black");
    expect(layers[0].alpha).toBeCloseTo(0, 6);
    expect(layers[1].alpha).toBeCloseTo(0, 6);
  });

  it("fades through white", () => {
    expect(frameAt(doc("fadewhite"), 3500).backdrop).toBe("white");
  });

  it("wipes the incoming picture in from the edge", () => {
    const { layers } = frameAt(doc("wipe"), 3500);
    expect(layers[1].clip).toEqual({ kind: "rect", x: 0.5, y: 0, w: 0.5, h: 1 });
  });

  // The wipe edge travels upward, so the new picture fills in from the bottom — the canvas
  // used to fill from the top, which is the wipe FFmpeg calls `wipedown`.
  it("wipes upward, the new picture growing from the bottom edge", () => {
    const { layers } = frameAt(doc("wipeup"), 3500);
    expect(layers[1].clip).toEqual({ kind: "rect", x: 0, y: 0.5, w: 1, h: 0.5 });
  });

  it("opens a circle over the outgoing picture", () => {
    const { layers } = frameAt(doc("circle"), 3500);
    const mask = layers[1].clip;
    expect(mask?.kind).toBe("circle");
    expect(mask?.kind === "circle" ? mask.r : 0).toBeCloseTo((0.5 * Math.hypot(1, 1)) / 2, 6);
  });

  it("slides upward", () => {
    const { layers } = frameAt(doc("slideup"), 3500);
    expect(layers[0].offsetY).toBeCloseTo(-0.5, 6);
    expect(layers[1].offsetY).toBeCloseTo(0.5, 6);
  });

  it("carries each clip's placements so the canvas can draw the frame", () => {
    const blurred = frameAt(doc("crossfade", "blur"), 1000, () => 0.75);
    expect(blurred.layers[0].placements).toHaveLength(2);
    expect(blurred.layers[0].placements[0].blurred).toBe(true);
  });
});

/** Halfway through the join between clip 1 and clip 2: 4000 ms − 1000 ms overlap → 3500 ms. */
describe("the transitions added in 6d", () => {
  const at = (kind: TransitionKind) => frameAt(doc(kind), 3500).layers;
  /**
   * The two pictures by the part they play rather than by where they sit in the array —
   * the array is paint order, and which of the two is on top is itself under test.
   */
  const pair = (kind: TransitionKind) => {
    const layers = at(kind);
    return { out: layers.find((l) => l.index === 0)!, into: layers.find((l) => l.index === 1)! };
  };

  it("covers with the incoming picture while the outgoing stays put", () => {
    const { out, into } = pair("cover");
    expect(out.offsetX).toBe(0);
    expect(into.offsetX).toBeCloseTo(0.5, 6);
  });

  it("reveals by moving the outgoing picture away", () => {
    const { out, into } = pair("reveal");
    expect(out.offsetX).toBeCloseTo(-0.5, 6);
    expect(into.offsetX).toBe(0);
  });

  it("slides both pictures down together", () => {
    const { out, into } = pair("slidedown");
    expect(out.offsetY).toBeCloseTo(0.5, 6);
    expect(into.offsetY).toBeCloseTo(-0.5, 6);
  });

  it("closes a circle over the outgoing picture", () => {
    const { out } = pair("circleclose");
    expect(out.clip?.kind).toBe("circle");
    expect(out.clip?.kind === "circle" ? out.clip.r : -1).toBeCloseTo((0.5 * Math.hypot(1, 1)) / 2, 6);
  });

  it("parts the frame from the middle", () => {
    expect(pair("vertopen").into.clip).toEqual({ kind: "rect", x: 0.25, y: 0, w: 0.5, h: 1 });
    expect(pair("horzopen").into.clip).toEqual({ kind: "rect", x: 0, y: 0.25, w: 1, h: 0.5 });
  });

  /**
   * `wipetl` retreats the *outgoing* picture into the top-left corner and lets the new one
   * take the rest of the frame. The canvas had it inverted — the new picture grew out of
   * the corner instead — which is the opposite wipe.
   */
  it("wipes the outgoing picture back into the corner", () => {
    expect(pair("corner").out.clip).toEqual({ kind: "rect", x: 0, y: 0, w: 0.5, h: 0.5 });
    expect(pair("corner").into.clip).toBeUndefined();
  });

  it("squeezes the outgoing picture flat", () => {
    expect(pair("squeeze").out.clip).toEqual({ kind: "scale", sx: 1, sy: 0.5 });
  });

  /**
   * FFmpeg's `zoomin`, measured frame by frame with a marker on a solid backdrop: the
   * outgoing picture is zoomed into over the first half — scale 2.1x by p = 0.25, past the
   * frame by p = 0.4 — and only then does the new one arrive, on `smoothstep(0.5, 1, p)`.
   * The canvas used to run a gentle 1.5x zoom with a linear crossfade across the whole
   * join, so the preview showed a blend at the midpoint where the film shows neither.
   */
  describe("zoom", () => {
    const layersAt = (ms: number) => frameAt(doc("zoom"), ms).layers;

    it("shows only the outgoing picture through the first half of the join", () => {
      const [out, into] = layersAt(3250);
      expect(out.alpha).toBe(1);
      expect(into.alpha).toBeCloseTo(0, 6);
    });

    it("zooms hard into the outgoing picture as it goes", () => {
      expect(layersAt(3250)[0].scale).toBeCloseTo(2.0, 1);
      expect(layersAt(3500)[0].scale).toBeGreaterThan(20);
    });

    it("hands over on a smoothstep through the second half", () => {
      expect(layersAt(3750)[1].alpha).toBeCloseTo(0.5, 6);
      expect(layersAt(3900)[1].alpha).toBeGreaterThan(0.8);
    });
  });

  it("brings a grid in as slices and as bands", () => {
    expect(pair("slices").into.clip).toEqual({ kind: "stripes", axis: "x", count: SLICE_COUNT, p: 0.5 });
    expect(pair("vslices").into.clip).toEqual({ kind: "stripes", axis: "y", count: SLICE_COUNT, p: 0.5 });
  });

  /**
   * A transition that moves, shrinks or clips the *outgoing* picture only shows anything
   * if that picture is painted last. Drawing the incoming one on top of it, as every
   * other transition wants, hides the whole effect and the join reads as a hard cut —
   * which is what "squeeze does nothing" turned out to be.
   */
  it("paints the outgoing picture last when the effect is on the outgoing picture", () => {
    for (const kind of ["reveal", "circleclose", "squeeze", "corner"] as TransitionKind[]) {
      const layers = at(kind);
      expect(layers, kind).toHaveLength(2);
      expect(layers[layers.length - 1].index, `${kind} paints the outgoing picture last`).toBe(0);
    }
  });

  it("paints the incoming picture last for every other transition", () => {
    const outgoingOnTop = new Set(["reveal", "circleclose", "squeeze", "corner"]);
    for (const kind of TRANSITION_KINDS) {
      if (kind === "none" || outgoingOnTop.has(kind)) continue;
      const layers = at(kind);
      expect(layers, kind).toHaveLength(2);
      expect(layers[layers.length - 1].index, `${kind} paints the incoming picture last`).toBe(1);
    }
  });

  it("every transition the editor offers is drawable throughout the join", () => {
    for (const kind of TRANSITION_KINDS) {
      for (const t of [3000, 3250, 3500, 3900, 4000]) {
        const { layers } = frameAt(doc(kind), t);
        expect(layers.length, `${kind} at ${t}`).toBeGreaterThan(0);
        for (const l of layers) {
          expect(Number.isFinite(l.alpha), `${kind} alpha`).toBe(true);
          expect(Number.isFinite(l.offsetX) && Number.isFinite(l.offsetY), `${kind} offset`).toBe(true);
          expect(Number.isFinite(l.scale) && l.scale > 0, `${kind} scale`).toBe(true);
        }
      }
    }
  });

  it("lists exactly the twenty transitions Rust knows", () => {
    expect(new Set(TRANSITION_KINDS).size).toBe(TRANSITION_KINDS.length);
    expect(TRANSITION_KINDS).toHaveLength(20);
  });
});

describe("videoTimeAt", () => {
  const clip = (duration_ms: number, video?: { in_ms: number; out_ms: number }) => ({
    asset_id: 1,
    extra: [],
    frame: "fill" as FrameKind,
    duration_ms,
    transition: { kind: "crossfade" as TransitionKind, duration_ms: 600, auto: false },
    motion: { kind: "none" as const, from: [0, 0, 1] as [number, number, number], to: [0, 0, 1] as [number, number, number] },
    ...(video ? { video } : {}),
  });
  const doc: SlideshowDoc = { version: 1, resolution: [1920, 1080], fps: 30, audio: null, clips: [clip(4000), clip(5000, { in_ms: 7000, out_ms: 12000 })] };

  it("follows the show through the video's best part", () => {
    // The second clip starts at 3400 (4000 less the 600 ms crossfade).
    expect(videoTimeAt(doc, 1, 3400)).toBe(7000);
    expect(videoTimeAt(doc, 1, 5400)).toBe(9000);
    expect(videoTimeAt(doc, 1, 99_000)).toBe(12_000);
  });

  it("holds the first frame during the incoming transition, and is null for a photo", () => {
    expect(videoTimeAt(doc, 1, 3000)).toBe(7000);
    expect(videoTimeAt(doc, 0, 1000)).toBeNull();
  });
});
