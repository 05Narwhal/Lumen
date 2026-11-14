import { useEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { isMac } from "@/components/window-controls";
import { developOf, outputSize } from "@/lib/develop";
import {
  appendStroke,
  type BrushSettings,
  brushDrag,
  brushMinRadius,
  brushPixels,
  showMaskOverlay,
  linearRotate,
  maskUi,
  moveBy,
  OVERLAY_COLOURS,
  radialFeather,
  radialResize,
  radialRotate,
  replaceShape,
  type LinearShape,
  type RadialShape,
  useMaskOverlayRule,
} from "@/lib/develop-masks";
import { type AiRasters, type FrameMap, frameMap, fromFrame, longEdge, quantise, rasterize, scaledMap, StrokeTint, toFrame } from "@/lib/mask-raster";
import type { Mask } from "@/types/generated/Mask";

const NO_RASTERS: AiRasters = {};

type Pt = [number, number];

/** The overlay's tint is drawn at most this many pixels along its long edge, then scaled. */
const TINT_EDGE = 480;
/** While painting, the session (and so the preview) hears of the stroke this often. */
const SEND_MS = 100;
/** How far beyond a gradient's end its rotate handle sits, in screen pixels. */
const ROTATE_GAP = 28;

/** Alt + right button (Photoshop's), or Ctrl + Option on a Mac, where Ctrl + click is a right click. */
const isBrushDrag = (e: { button: number; altKey: boolean; ctrlKey: boolean }) => e.altKey && (e.button === 2 || (isMac() && e.ctrlKey && e.button === 0));

/** A point (0–1) of the unedited photo moved `q1`, `q2` radii along a radial's axes. */
function onRadial(s: RadialShape, f: FrameMap, q1: number, q2: number): Pt {
  const t = (s.angle * Math.PI) / 180;
  const [c, sn] = [Math.cos(t), Math.sin(t)];
  const [rx, ry] = [s.radius[0] * f.base[0], s.radius[1] * f.base[1]];
  return [s.center[0] + (c * q1 * rx - sn * q2 * ry) / f.base[0], s.center[1] + (sn * q1 * rx + c * q2 * ry) / f.base[1]];
}

const add = (a: Pt, b: Pt): Pt => [a[0] + b[0], a[1] + b[1]];
const sub = (a: Pt, b: Pt): Pt => [a[0] - b[0], a[1] - b[1]];
const len = (a: Pt) => Math.hypot(a[0], a[1]);
const scale = (a: Pt, k: number): Pt => [a[0] * k, a[1] * k];

/** The tint colour of a token, as RGB, read through a canvas so any CSS colour works. */
function tokenRgb(el: Element, token: string): [number, number, number] {
  const css = getComputedStyle(el).getPropertyValue(token).trim() || "rgb(255, 40, 40)";
  const c = document.createElement("canvas").getContext?.("2d");
  if (!c) return [255, 40, 40];
  c.fillStyle = css;
  c.fillRect(0, 0, 1, 1);
  const d = c.getImageData(0, 0, 1, 1).data;
  return [d[0], d[1], d[2]];
}

/**
 * Develop's mask tools over the photo (Phase 8e), in a box the size of the photo as shown:
 * the selected mask's weights as a tint (`O`, on by default, hidden while dragging), handles for its
 * gradients, and the brush. Everything is drawn through the same maths Rust renders with
 * (`mask-raster.ts`), and every pointer position is mapped back into the unedited photo, so
 * turns, flips, straighten and crop all hold. A drag or a stroke is one history step.
 */
export function MaskOverlay({
  session,
  base,
  box,
  rasters = NO_RASTERS,
}: {
  session: DevelopSession;
  base: { w: number; h: number };
  box: { width: number; height: number };
  /** Phase 8f: AI shapes' rasters by hash, tinted like any other shape. */
  rasters?: AiRasters;
}) {
  const ui = maskUi.use();
  const r = session.draft;
  const masks = developOf(r).masks ?? [];
  const mask = masks.find((m) => m.id === ui.selected) ?? null;
  useMaskOverlayRule(mask ?? undefined);
  const [fw] = outputSize(r, base.w, base.h);
  const map = useMemo(() => scaledMap(frameMap(base.w, base.h, r), box.width / Math.max(1, fw)), [base.w, base.h, r, box.width, fw]);
  const root = useRef<HTMLDivElement>(null);
  const tint = useRef<HTMLCanvasElement>(null);
  const [cursor, setCursor] = useState<Pt | null>(null);
  /** The size-and-feather drag: the brush when it began, the pointer then, and where the ring stays. */
  const [sizing, setSizing] = useState<{ from: BrushSettings; at: Pt; ring: Pt } | null>(null);

  const local = (e: { clientX: number; clientY: number }): Pt => {
    const b = root.current?.getBoundingClientRect();
    return [e.clientX - (b?.left ?? 0), e.clientY - (b?.top ?? 0)];
  };

  /** Masks as they were when a drag began: each move replaces, it never accumulates. */
  const start = useRef<{ masks: Mask[]; at: Pt } | null>(null);
  const setMasks = (next: Mask[]) => session.update((d) => ({ ...d, masks: next }));

  // The tint is drawn at a reduced size; its colour is read from the token once per colour.
  // On unless the user turned it off (`O`), and out of the way while anything is dragged.
  const showTint = !!mask && ui.overlay && !ui.active;
  const k = Math.min(1, TINT_EDGE / Math.max(box.width, box.height, 1));
  const tw = Math.max(1, Math.round(box.width * k));
  const th = Math.max(1, Math.round(box.height * k));
  const colours = useRef(new Map<number, [number, number, number]>());
  const rgb = (canvas: HTMLCanvasElement) => {
    const i = ui.colour % OVERLAY_COLOURS.length;
    let c = colours.current.get(i);
    if (!c) {
      c = tokenRgb(canvas, OVERLAY_COLOURS[i]);
      colours.current.set(i, c);
    }
    return c;
  };
  /** Writes `weights` into the canvas, only inside `dirty` when given. */
  const paint = (weights: Float32Array, dirty?: { x0: number; y0: number; x1: number; y1: number } | null) => {
    const canvas = tint.current;
    const ctx = canvas?.getContext?.("2d");
    if (!canvas || !ctx) return;
    if (canvas.width !== tw || canvas.height !== th) {
      canvas.width = tw;
      canvas.height = th;
      dirty = null;
    }
    const b = dirty ?? { x0: 0, y0: 0, x1: tw, y1: th };
    if (b.x0 >= b.x1 || b.y0 >= b.y1) return;
    const [cr, cg, cb] = rgb(canvas);
    const img = ctx.createImageData(b.x1 - b.x0, b.y1 - b.y0);
    let o = 0;
    for (let y = b.y0; y < b.y1; y++)
      for (let x = b.x0; x < b.x1; x++, o += 4) {
        img.data[o] = cr;
        img.data[o + 1] = cg;
        img.data[o + 2] = cb;
        img.data[o + 3] = Math.round(weights[y * tw + x] * 150);
      }
    ctx.putImageData(img, b.x0, b.y0);
  };

  /** The stroke being painted: drawn here dab by dab, sent to the session a few times a second. */
  const stroke = useRef<{ masks: Mask[]; points: Pt[]; add: boolean; tint: StrokeTint | null; timer: ReturnType<typeof setTimeout> | null; settling: boolean; unsent: boolean } | null>(null);
  // A stroke cut short by unmounting (another photo, another tool) sends nothing late.
  useEffect(
    () => () => {
      const t = stroke.current?.timer;
      if (t) clearTimeout(t);
    },
    [],
  );

  // The whole tint, when the mask or the view changes — not mid-stroke, which draws itself.
  // biome-ignore lint/correctness/useExhaustiveDependencies: paint reads the same values
  useEffect(() => {
    if (!mask || !showTint || stroke.current) return;
    paint(rasterize(mask, scaledMap(map, k), tw, th, rasters));
  }, [mask, map, tw, th, showTint, ui.colour, rasters]);

  if (!mask) return null;

  // The brush: a stroke from pointer down to up is one shape and one history step.
  const brushRadius = ui.brush.radius * longEdge(map);
  const brushOf = (add: boolean) => ({ ...ui.brush, add });
  const ringAt = sizing?.ring ?? cursor;
  /** The stroke so far into the session (and so the preview). */
  const send = (s: NonNullable<typeof stroke.current>) => {
    if (s.timer) clearTimeout(s.timer);
    s.timer = null;
    s.unsent = false;
    setMasks(appendStroke(s.masks, mask.id, brushOf(s.add), s.points));
    // The next send waits for this one to be drawn, then `SEND_MS` more.
    s.settling = true;
    requestAnimationFrame(() => {
      s.settling = false;
      if (s.unsent && stroke.current === s) sendSoon(s);
    });
  };
  /**
   * Each send re-renders all of Develop, which can take longer than `SEND_MS` (60–80 ms in
   * dev, then again when the preview lands). Paced from the last frame drawn, never from the
   * last send, so moves only draw the tint and never queue behind renders.
   */
  const sendSoon = (s: NonNullable<typeof stroke.current>) => {
    s.unsent = true;
    if (s.timer || s.settling) return;
    s.timer = setTimeout(() => {
      s.timer = null;
      if (stroke.current === s) send(s);
    }, SEND_MS);
  };
  const drawStroke = (s: NonNullable<typeof stroke.current>) => {
    if (!s.tint) return;
    s.tint.extend(s.points.map(quantise));
    paint(s.tint.weights, s.tint.dirty);
  };
  const paintDown = (e: PointerEvent<HTMLDivElement>) => {
    if (!ui.painting) return;
    if (isBrushDrag(e)) {
      // Nothing is painted: the pointer only turns into the brush's size and feather.
      e.stopPropagation();
      e.currentTarget.setPointerCapture?.(e.pointerId);
      setSizing({ from: ui.brush, at: [e.clientX, e.clientY], ring: local(e) });
      return;
    }
    if (e.button !== 0) return;
    e.stopPropagation();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    // Painting shows where the mask is, even if the overlay was off (Round 8B).
    showMaskOverlay();
    const add = e.altKey ? !ui.painting.add : ui.painting.add;
    const s = { masks, points: [fromFrame(map, local(e))], add, tint: tint.current ? new StrokeTint(mask, scaledMap(map, k), tw, th, brushOf(add), rasters) : null, timer: null, settling: false, unsent: false };
    stroke.current = s;
    drawStroke(s);
    send(s);
  };
  const paintMove = (e: PointerEvent<HTMLDivElement>) => {
    if (sizing) {
      const brush = brushDrag(sizing.from, e.clientX - sizing.at[0], e.clientY - sizing.at[1], longEdge(map), brushMinRadius(Math.max(base.w, base.h)));
      maskUi.set((u) => ({ ...u, brush }));
      return;
    }
    const at = local(e);
    setCursor(at);
    const s = stroke.current;
    if (!s) return;
    const last = toFrame(map, ...s.points[s.points.length - 1]);
    if (e.shiftKey) s.points = [s.points[0], fromFrame(map, at)];
    else if (len(sub(at, last)) >= Math.max(1, brushRadius / 8)) s.points.push(fromFrame(map, at));
    else return;
    drawStroke(s);
    sendSoon(s);
  };
  const paintUp = () => {
    if (sizing) {
      setSizing(null);
      return;
    }
    const s = stroke.current;
    if (!s) return;
    send(s);
    stroke.current = null;
    session.commit("Brush");
  };

  /**
   * A handle's drag: `change` turns the pointer into the new shape, from the shape as it was
   * when the drag began — never the one on screen, which has already moved (each move then
   * added the whole distance again and the mask flew off).
   */
  const drag = <S extends Mask["shapes"][number]>(index: number, label: string, change: (p: Pt, from: Pt, shape: S) => Mask["shapes"][number]) => ({
    onPointerDown: (e: PointerEvent<Element>) => {
      if (e.button !== 0) return;
      e.stopPropagation();
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
      // Moving or resizing shows the mask, even if the overlay was off (Round 8B).
      showMaskOverlay();
      start.current = { masks, at: local(e) };
    },
    onPointerMove: (e: PointerEvent<Element>) => {
      const s = start.current;
      if (!s) return;
      e.stopPropagation();
      const shape = s.masks.find((m) => m.id === mask.id)?.shapes[index] as S | undefined;
      if (shape) setMasks(replaceShape(s.masks, mask.id, index, change(local(e), s.at, shape)));
    },
    onPointerUp: (e: PointerEvent<Element>) => {
      if (!start.current) return;
      e.stopPropagation();
      start.current = null;
      session.commit(label);
    },
  });

  const handles = ui.painting
    ? null
    : mask.shapes.map((shape, i) => {
        if (shape.kind === "linear") {
          const s = shape as LinearShape;
          const a = toFrame(map, s.from[0], s.from[1]);
          const b = toFrame(map, s.to[0], s.to[1]);
          const mid = scale(add(a, b), 0.5);
          const d = sub(b, a);
          const unit = scale(d, 1 / (len(d) || 1));
          const n: Pt = [-unit[1], unit[0]];
          const far = Math.max(box.width, box.height) * 2;
          const line = (p: Pt, k: string) => <line key={k} className={`mask-overlay__line mask-overlay__line--${k}`} x1={p[0] - n[0] * far} y1={p[1] - n[1] * far} x2={p[0] + n[0] * far} y2={p[1] + n[1] * far} />;
          const rot = add(b, scale(unit, ROTATE_GAP));
          return (
            <g key={i} data-shape="linear">
              {line(a, "from")}
              {line(mid, "mid")}
              {line(b, "to")}
              <circle className="mask-overlay__handle" role="button" aria-label="Linear start" cx={a[0]} cy={a[1]} r={6} {...drag<LinearShape>(i, "Linear gradient", (p, _, b) => ({ ...b, from: fromFrame(map, p) }))} />
              <circle className="mask-overlay__handle" role="button" aria-label="Linear end" cx={b[0]} cy={b[1]} r={6} {...drag<LinearShape>(i, "Linear gradient", (p, _, b) => ({ ...b, to: fromFrame(map, p) }))} />
              <circle
                className="mask-overlay__handle mask-overlay__handle--move"
                role="button"
                aria-label="Linear centre"
                cx={mid[0]}
                cy={mid[1]}
                r={7}
                {...drag<LinearShape>(i, "Linear gradient", (p, from, b) => ({ ...b, from: moveBy(map, b.from, from, p), to: moveBy(map, b.to, from, p) }))}
              />
              <circle className="mask-overlay__handle mask-overlay__handle--rotate" role="button" aria-label="Linear rotate" cx={rot[0]} cy={rot[1]} r={5} {...drag<LinearShape>(i, "Linear gradient", (p, _, b) => linearRotate(b, map, p, mid))} />
            </g>
          );
        }
        if (shape.kind === "radial") {
          const s = shape as RadialShape;
          const at = (q1: number, q2: number) => toFrame(map, ...onRadial(s, map, q1, q2));
          const c = at(0, 0);
          const e1 = sub(at(1, 0), c);
          const e2 = sub(at(0, 1), c);
          const deg = (Math.atan2(e1[1], e1[0]) * 180) / Math.PI;
          const inner = 1 - s.feather;
          const rot = add(c, scale(e2, -(1 + ROTATE_GAP / (len(e2) || 1))));
          const fea = at(inner, 0);
          return (
            <g key={i} data-shape="radial">
              <ellipse className="mask-overlay__ellipse" cx={c[0]} cy={c[1]} rx={len(e1)} ry={len(e2)} transform={`rotate(${deg} ${c[0]} ${c[1]})`} />
              <ellipse className="mask-overlay__ellipse mask-overlay__ellipse--feather" cx={c[0]} cy={c[1]} rx={len(e1) * inner} ry={len(e2) * inner} transform={`rotate(${deg} ${c[0]} ${c[1]})`} />
              <circle
                className="mask-overlay__handle mask-overlay__handle--move"
                role="button"
                aria-label="Radial centre"
                cx={c[0]}
                cy={c[1]}
                r={7}
                {...drag<RadialShape>(i, "Radial gradient", (p, from, b) => ({ ...b, center: moveBy(map, b.center, from, p) }))}
              />
              {(
                [
                  [1, 0, "x"],
                  [-1, 0, "x"],
                  [0, 1, "y"],
                  [0, -1, "y"],
                ] as const
              ).map(([q1, q2, axis]) => {
                const p = at(q1, q2);
                return (
                  <circle
                    key={`${q1}${q2}`}
                    className="mask-overlay__handle"
                    role="button"
                    aria-label={axis === "x" ? "Radial width" : "Radial height"}
                    cx={p[0]}
                    cy={p[1]}
                    r={5}
                    {...drag<RadialShape>(i, "Radial gradient", (pt, _, b) => radialResize(b, axis, map, pt))}
                  />
                );
              })}
              <circle className="mask-overlay__handle mask-overlay__handle--rotate" role="button" aria-label="Radial rotate" cx={rot[0]} cy={rot[1]} r={5} {...drag<RadialShape>(i, "Radial gradient", (p, _, b) => radialRotate(b, map, p))} />
              <circle className="mask-overlay__handle mask-overlay__handle--feather" role="button" aria-label="Radial feather" cx={fea[0]} cy={fea[1]} r={4} {...drag<RadialShape>(i, "Radial gradient", (p, _, b) => radialFeather(b, map, p))} />
            </g>
          );
        }
        return null;
      });

  return (
    <div className="mask-overlay" ref={root} style={{ width: box.width, height: box.height }}>
      {showTint && <canvas ref={tint} className="mask-overlay__tint" aria-hidden />}
      {ui.painting ? (
        <div
          className="mask-overlay__paint"
          role="application"
          aria-label="Paint the mask"
          onPointerDown={paintDown}
          onPointerMove={paintMove}
          onPointerUp={paintUp}
          onPointerCancel={paintUp}
          onPointerLeave={() => setCursor(null)}
          // The browser's menu stays off the photo while painting; Alt + right-drag is the brush's.
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
        >
          {ringAt && (
            <svg className="mask-overlay__svg" aria-hidden>
              <circle className="mask-overlay__ring" cx={ringAt[0]} cy={ringAt[1]} r={brushRadius} />
              <circle className="mask-overlay__ring mask-overlay__ring--inner" cx={ringAt[0]} cy={ringAt[1]} r={brushRadius * (1 - ui.brush.feather)} />
            </svg>
          )}
          {sizing && (
            <span className="mask-overlay__brush-label" style={{ left: sizing.ring[0] + brushRadius + 8, top: sizing.ring[1] }}>
              {`${Math.round(brushPixels(ui.brush.radius, Math.max(base.w, base.h)))} px · feather ${Math.round(ui.brush.feather * 100)}`}
            </span>
          )}
        </div>
      ) : (
        <svg className="mask-overlay__svg">{handles}</svg>
      )}
    </div>
  );
}
