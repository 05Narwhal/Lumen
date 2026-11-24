import { IconFlagFilled, IconX } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";
import { useContextMenu } from "@/components/context-menu";
import { Spinner } from "@/components/spinner";
import { beforeRecipe, decoded, type DevelopSession, type Rendered } from "@/hooks/use-develop-session";
import { useElementSize } from "@/hooks/use-element-size";
import { useLoadedRasters } from "@/lib/ai-masks";
import { developOf, outputSize } from "@/lib/develop";
import { maskUi, OVERLAY_COLOURS, stepBrushRadius } from "@/lib/develop-masks";
import { invoke } from "@/lib/ipc";
import { matches } from "@/lib/keybinds";
import { developCanvasMenu, type DevelopCanvasActions, type DevelopCanvasContext } from "@/lib/menus";
import { FLAG_PICK, FLAG_REJECT, LABELS } from "@/lib/marks";
import type { DevelopBaseInfo } from "@/types/generated/DevelopBaseInfo";
import type { DevelopPreview } from "@/types/generated/DevelopPreview";
import type { StripItem } from "@/types/generated/StripItem";
import { mediaUrl } from "@/utils/media";
import { nextCompare, PANE_GAP, paneSize, sideBySideAxis } from "./compare";
import { MaskOverlay } from "./mask-overlay";
import { ZoomBar } from "./zoom-bar";
import { clampPan, levelOf, nextLevel, panAroundCentre, percent, scaleOf, sourceFor, visibleRect, wheelFactor, zoomAround, type Pan, type Size, type ZoomState } from "./zoom";

/** The outgoing photo is dropped after its slide even if `animationend` never comes. */
const LEAVE_MS = 400;

/** Panning settles for this long before the full-resolution part is asked for. */
const REGION_DEBOUNCE_MS = 120;
/** A spinner only when a render takes longer than this. */
const SPINNER_DELAY_MS = 300;

/** The stage's padding around a fitted photo (`--space-4`). */
const PAD = 16;
/** The navigator's long edge. */
const NAV = 120;
const FIT: ZoomState = { mode: "fit" };

// From the clock, as the preview's counter: Rust keeps the newest request for the life of the
// process, so a reloaded page counting from 1 again would have every region called stale.
let regionCounter = Date.now();

/** A rendered part of the photo, in photo pixels, and the settings it was rendered with. */
type Region = { url: string; x: number; y: number; w: number; h: number; tag: string };

const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);

/** The open photo's flag and label, in the canvas's corner (culling lives in Develop). */
function Marks({ marks }: { marks: Pick<StripItem, "flag" | "colorLabel"> }) {
  const label = LABELS.find((l) => l.value === marks.colorLabel);
  if (!label && marks.flag === 0) return null;
  return (
    <div className="develop-canvas__marks">
      {marks.flag === FLAG_PICK && (
        <span className="develop-canvas__flag" title="Picked">
          <IconFlagFilled size={13} aria-label="Picked" />
        </span>
      )}
      {marks.flag === FLAG_REJECT && (
        <span className="develop-canvas__flag develop-canvas__flag--reject" title="Rejected">
          <IconX size={13} aria-label="Rejected" />
        </span>
      )}
      {label && <span className="develop-canvas__label" style={{ background: label.color }} title={`${label.name} label`} aria-label={`${label.name} label`} />}
    </div>
  );
}

/**
 * The photo as developed (spec §1). Fitted to the stage by default; the zoom bar, `Z`, a
 * double-click, `Ctrl+=` / `Ctrl+−` and Ctrl+wheel (around the cursor) show it filled or at
 * 5–400 % (100 % = one image pixel per screen pixel), dragged to pan, with the visible part
 * rendered from the full-resolution frame once the preview runs out of pixels (Round 4J).
 * `\` shows the photo before its develop settings, `J` paints clipping.
 */
export function DevelopCanvas({
  session,
  assetId,
  marks,
  order = [],
  actions,
  openWith,
}: {
  session: DevelopSession;
  assetId: number;
  marks: Pick<StripItem, "flag" | "colorLabel"> | null;
  /** The workspace's actions on the open photo, for the canvas's right-click menu (Round 4G). */
  actions?: Pick<DevelopCanvasActions, "copySettings" | "pasteSettings" | "reset" | "flag" | "label" | "export" | "openWith">;
  /** The apps that fit the open photo, for *Edit in* and *Open with ▸* (Round 22 §9). */
  openWith?: DevelopCanvasContext["openWith"];
  /** The filmstrip's photos in order: a photo further along slides in from the right. */
  order?: number[];
}) {
  // The newest render when it is of this photo; until then this photo's cached 1024 px
  // rendition (upright, its saved edit applied), so the canvas never shows another photo
  // than the panels do (Round 4A review) — the render then swaps in place.
  const [standIn, setStandIn] = useState<(Rendered & { assetId: number }) | null>(null);
  useEffect(() => {
    if (session.preview?.assetId === assetId) return;
    let live = true;
    const url = mediaUrl("thumb", assetId, { s: 1024 });
    void decoded(url).then(() => {
      if (live) setStandIn({ assetId, url, width: 0, height: 0 });
    });
    return () => {
      live = false;
    };
    // Once per photo: a render of this photo makes the stand-in moot.
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyed on the photo alone
  }, [assetId]);
  const p = session.preview?.assetId === assetId ? session.preview : standIn?.assetId === assetId ? standIn : session.preview;
  // Which way the photo shown last moved to the one shown now (Round 4A: a carousel, not
  // a reload). Decided when the photo on screen changes, not when the URL does, because the
  // last photo stays up until the next one's render is in.
  // The slides are CSS keyframes (a Motion element here stalled Mantine's menus in tests,
  // and the duration tokens already honour reduced motion).
  const direction = useRef(1);
  const lastShown = useRef<Rendered & { assetId: number } | null>(null);
  const [leaving, setLeaving] = useState<(Rendered & { assetId: number; direction: number }) | null>(null);
  if (p && p.assetId !== lastShown.current?.assetId) {
    const prev = lastShown.current;
    if (prev) {
      const from = order.indexOf(prev.assetId);
      const to = order.indexOf(p.assetId);
      direction.current = from < 0 || to < 0 ? 1 : Math.sign(to - from) || 1;
    }
    lastShown.current = p;
    // Set during render on purpose: the outgoing photo must be in the same paint as the
    // incoming one, or the stage flashes empty between them.
    if (prev && (leaving?.assetId !== prev.assetId)) setLeaving({ ...prev, direction: direction.current });
  } else if (p) {
    lastShown.current = p;
  }
  useEffect(() => {
    if (!leaving) return;
    const t = setTimeout(() => setLeaving(null), LEAVE_MS);
    return () => clearTimeout(t);
  }, [leaving]);
  const [ref, size] = useElementSize<HTMLDivElement>(50);
  // The photo as fitted, measured, so the mask tools can lie exactly over it (Phase 8e).
  const [imgBox, setImgBox] = useState<{ width: number; height: number } | null>(null);
  const imgObserver = useRef<ResizeObserver | null>(null);
  const measureImg = useCallback((el: HTMLImageElement | null) => {
    imgObserver.current?.disconnect();
    imgObserver.current = null;
    if (!el) return;
    const apply = () => setImgBox(el.clientWidth > 0 && el.clientHeight > 0 ? { width: el.clientWidth, height: el.clientHeight } : null);
    apply();
    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(apply);
      ro.observe(el);
      imgObserver.current = ro;
    }
  }, []);
  const masks = maskUi.use();
  // Phase 8f: AI shapes' rasters for the overlay's tint, fetched over `photos://ai-mask/`.
  const { rasters: aiRasters } = useLoadedRasters(session.assetId, developOf(session.draft).masks ?? []);
  const [zoom, setZoom] = useState<ZoomState>(FIT);
  const { open: openMenu } = useContextMenu();
  const [pan, setPan] = useState<Pan>({ x: 0, y: 0 });
  /** Where `Z` and a double-click go from fit: the level last used, 100 % at first. */
  const lastLevel = useRef<ZoomState>({ mode: "level", level: 1 });
  const [region, setRegion] = useState<Region | null>(null);
  const [beforeRegion, setBeforeRegion] = useState<Region | null>(null);
  /** Split's divider, 0–1 of the stage's width. */
  const [split, setSplit] = useState(0.5);
  const [slow, setSlow] = useState(false);
  const [space, setSpace] = useState(false);
  const info = useQuery({ queryKey: ["develop-base-info", assetId], queryFn: () => invoke<DevelopBaseInfo>("develop_base_info", { assetId }) });
  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  const frame = info.data ? outputSize(session.draft, info.data.width, info.data.height) : null;
  const photo: Size | null = frame ? { w: frame[0], h: frame[1] } : null;
  const stage: Size = { w: size.width, h: size.height };
  // Side by side halves the stage; every pane shows the same zoom and pan (Round 4J).
  const compare = session.compare;
  const axis = compare === "side" && photo ? sideBySideAxis(photo, stage) : null;
  const view: Size = axis ? paneSize(stage, axis) : stage;
  /** A point of the stage in its pane's own coordinates. */
  const toPane = (x: number, y: number): Pan => {
    if (axis === "row" && x > view.w + PANE_GAP / 2) return { x: x - view.w - PANE_GAP, y };
    if (axis === "column" && y > view.h + PANE_GAP / 2) return { x, y: y - view.h - PANE_GAP };
    return { x, y };
  };
  // Fit is drawn by CSS inside the stage's padding; as a level it is the same scale.
  const fitView: Size = { w: Math.max(1, view.w - 2 * PAD), h: Math.max(1, view.h - 2 * PAD) };
  const measured = !!photo && view.w > 0 && view.h > 0;
  /** Any state as a level, so the maths works in the whole stage. */
  const asLevel = (z: ZoomState): ZoomState => (z.mode === "level" || !photo ? z : { mode: "level", level: levelOf(z, photo, z.mode === "fit" ? fitView : view, dpr) });
  const level = photo ? levelOf(asLevel(zoom), photo, view, dpr) : 1;
  const scale = level / dpr;
  const zoomed = zoom.mode !== "fit";
  const masking = session.tool === "masks" && masks.selected !== null && !!info.data && compare === "off" && !session.before;
  const overlay = (box: { width: number; height: number }) =>
    info.data ? <MaskOverlay session={session} base={{ w: info.data.width, h: info.data.height }} box={box} rasters={aiRasters} /> : null;

  // A spinner only when the preview is slow to come.
  useEffect(() => {
    if (!session.rendering) {
      setSlow(false);
      return;
    }
    const t = setTimeout(() => setSlow(true), SPINNER_DELAY_MS);
    return () => clearTimeout(t);
  }, [session.rendering]);

  /** Shows `next` with `nextPan` (photo pixels); back at fit, the full-size frame is freed. */
  const show = useCallback((next: ZoomState, nextPan: Pan) => {
    setZoom(next);
    setPan(nextPan);
    if (next.mode === "level") lastLevel.current = next;
    if (next.mode === "fit") {
      setRegion(null);
      setBeforeRegion(null);
      void invoke("develop_release_full").catch(() => {});
    }
  }, []);

  // Everything the event handlers need, current, without re-subscribing on every pan.
  // `edge`: the full photo's long edge in pixels, which `[` and `]` hold the brush to (1 px and up).
  const edge = Math.max(info.data?.width ?? 0, info.data?.height ?? 0) || 4000;
  const latest = useRef({ zoom, pan, photo, view, fitView, dpr, measured, asLevel, show, toPane, edge });
  latest.current = { zoom, pan, photo, view, fitView, dpr, measured, asLevel, show, toPane, edge };

  /** Another zoom, keeping the view's centre on the same photo point. */
  const zoomTo = useCallback((next: ZoomState) => {
    const l = latest.current;
    if (!l.measured || !l.photo) return;
    const from = l.asLevel(l.zoom);
    const to = l.asLevel(next);
    l.show(next, next.mode === "fit" ? l.pan : panAroundCentre(from, to, l.pan, l.photo, l.view, l.dpr));
  }, []);
  const step = useCallback(
    (dir: 1 | -1) => {
      const l = latest.current;
      if (l.photo) zoomTo(nextLevel(l.zoom, dir, l.photo, l.fitView, l.dpr));
    },
    [zoomTo],
  );
  /** Zooms by `factor` around a point of the stage (CSS pixels from its top-left). */
  const zoomAt = useCallback((factor: number, anchor: Pan) => {
    const l = latest.current;
    if (!l.measured || !l.photo) return;
    const from = l.asLevel(l.zoom);
    const r = zoomAround(from, factor, anchor, l.zoom.mode === "fit" ? clampPan(l.pan, from, l.photo, l.view, l.dpr) : l.pan, l.photo, l.view, l.dpr);
    l.show(r.zoom, r.pan);
  }, []);

  // Leaving the photo fits the next one; leaving the screen frees the full-size frame.
  useEffect(() => {
    setZoom(FIT);
    setRegion(null);
    setBeforeRegion(null);
  }, [assetId]);
  useEffect(() => () => void invoke("develop_release_full").catch(() => {}), []);

  // Past what the preview's base holds, the visible part from the full-resolution frame, once
  // panning and zooming pause. A region belongs to the settings it was rendered with: one
  // for older settings is never drawn over a newer preview.
  const regionTag = JSON.stringify([assetId, session.draft, session.before]);
  const baseLong = p && p.assetId === assetId ? Math.max(p.width, p.height) : 0;
  const needsRegion = zoomed && measured && photo && sourceFor(level, baseLong, Math.max(photo.w, photo.h)) === "region";
  const bv = session.beforeView;
  const comparing = (compare === "split" || compare === "side") && !!bv;
  const beforeGeometry = beforeRecipe(session.draft);
  const beforeTag = JSON.stringify([assetId, beforeGeometry]);
  const regionKey = needsRegion ? JSON.stringify([regionTag, pan, level, view, comparing]) : null;
  // biome-ignore lint/correctness/useExhaustiveDependencies: regionKey carries what is asked
  useEffect(() => {
    if (!regionKey || !photo) return;
    const rect = visibleRect(pan, { mode: "level", level }, photo, view, dpr);
    const tag = regionTag;
    const t = setTimeout(() => {
      const request = ++regionCounter;
      const at = { x: rect.x * photo.w, y: rect.y * photo.h, w: rect.w * photo.w, h: rect.h * photo.h };
      const ask = (recipe: typeof session.draft, before: boolean, put: (r: Region) => void, t: string) =>
        invoke<DevelopPreview>("develop_region", { assetId, recipe, rect, scale: Math.min(1, level), request, before })
          .then((r) => {
            if (r.superseded || request !== regionCounter) return;
            put({ url: mediaUrl("edit-preview", r.token), ...at, tag: t });
          })
          .catch(() => {});
      void ask(session.draft, session.before, setRegion, tag);
      // Rust keeps the before's region apart from the after's, so neither overtakes the other.
      if (comparing) void ask(beforeGeometry, true, setBeforeRegion, beforeTag);
    }, REGION_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [regionKey]);

  // Before/after, clipping, fit ↔ the last level, zoom in / out / fit, Space held pans, and the
  // mask tools' keys: all from the registry's `develop.*` rows.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.defaultPrevented) return;
      // The keys come from the registry (`lib/keybinds.ts`), so the person's own choices apply.
      if (matches(e, "develop.zoom-in")) step(1);
      else if (matches(e, "develop.zoom-out")) step(-1);
      else if (matches(e, "develop.fit")) zoomTo(FIT);
      else if (e.ctrlKey || e.metaKey || e.altKey) return;
      else if (e.key === " ") {
        // Held over the stage, not on a control Space would press.
        if (e.target instanceof HTMLElement && e.target.closest("button, [role='slider'], a")) return;
        e.preventDefault();
        setSpace(true);
        return;
      } else if (matches(e, "develop.before-after")) session.setBefore(!session.before);
      else if (matches(e, "develop.clipping")) session.setClipping(!session.clipping);
      else if (matches(e, "develop.compare-mode")) session.setCompare(nextCompare(session.compare));
      else if (matches(e, "develop.zoom-toggle")) zoomTo(latest.current.zoom.mode === "fit" ? lastLevel.current : FIT);
      // The mask tools (Phase 8e): the overlay, its colour, the brush's size and with Shift its feather.
      else if (session.tool === "masks" && matches(e, "develop.mask-overlay-colour")) maskUi.set((u) => ({ ...u, overlay: true, colour: (u.colour + 1) % OVERLAY_COLOURS.length }));
      else if (session.tool === "masks" && matches(e, "develop.mask-overlay")) maskUi.set((u) => ({ ...u, overlay: !u.overlay }));
      else if (session.tool === "masks" && (matches(e, "develop.feather-more") || matches(e, "develop.feather-less"))) {
        const dir = matches(e, "develop.feather-more") ? 1 : -1;
        maskUi.set((u) => ({ ...u, brush: { ...u.brush, feather: Math.min(1, Math.max(0, u.brush.feather + dir * 0.1)) } }));
      } else if (session.tool === "masks" && (matches(e, "develop.brush-larger") || matches(e, "develop.brush-smaller"))) {
        const dir = matches(e, "develop.brush-larger") ? 1 : -1;
        maskUi.set((u) => ({ ...u, brush: { ...u.brush, radius: stepBrushRadius(u.brush.radius, dir, latest.current.edge) } }));
      } else return;
      e.preventDefault();
    };
    const onUp = (e: KeyboardEvent) => {
      if (e.key === " ") setSpace(false);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onUp);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onUp);
    };
  }, [session, zoomTo, step]);

  // Ctrl+wheel (and a touchpad's pinch, which arrives as one) zooms around the cursor; a
  // plain wheel pans once zoomed. Not React's `onWheel`, which is passive and can't stop the
  // webview scrolling.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      const l = latest.current;
      if (!l.measured || !l.photo) return;
      const px = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? l.view.h : 1;
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        const r = el.getBoundingClientRect();
        zoomAt(wheelFactor(e.deltaY * px), l.toPane(e.clientX - r.left, e.clientY - r.top));
      } else if (l.zoom.mode !== "fit") {
        e.preventDefault();
        const s = scaleOf(l.asLevel(l.zoom), l.photo, l.view, l.dpr);
        setPan(clampPan({ x: l.pan.x + (e.deltaX * px) / s, y: l.pan.y + (e.deltaY * px) / s }, l.zoom, l.photo, l.view, l.dpr));
      }
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [ref, zoomAt]);

  // Dragging pans once zoomed.
  const drag = useRef<{ x: number; y: number; pan: Pan } | null>(null);
  const down = (e: PointerEvent<HTMLDivElement>) => {
    if (!zoomed || e.button !== 0) return;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    drag.current = { x: e.clientX, y: e.clientY, pan };
  };
  const move = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d || !photo) return;
    setPan(clampPan({ x: d.pan.x - (e.clientX - d.x) / scale, y: d.pan.y - (e.clientY - d.y) / scale }, zoom, photo, view, dpr));
  };
  const up = () => {
    drag.current = null;
  };
  const dbl = (e: React.MouseEvent<HTMLDivElement>) => {
    // Two quick clicks while masking are two dabs or two points, never a zoom.
    if (masking) return;
    if (zoomed) return zoomTo(FIT);
    if (!photo) return;
    const r = e.currentTarget.getBoundingClientRect();
    const target = lastLevel.current.mode === "level" ? lastLevel.current.level : 1;
    zoomAt(target / levelOf(asLevel(FIT), photo, view, dpr), toPane(e.clientX - r.left, e.clientY - r.top));
  };

  // The navigator: the whole photo small, the visible part outlined; a press or a drag in it
  // moves the view there.
  const navDrag = useRef(false);
  const navTo = (e: PointerEvent<HTMLDivElement>) => {
    if (!photo) return;
    const r = e.currentTarget.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) return;
    const fx = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    const fy = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
    setPan(clampPan({ x: fx * photo.w - view.w / 2 / scale, y: fy * photo.h - view.h / 2 / scale }, zoom, photo, view, dpr));
  };

  // Split's divider: dragged (never panning the photo under it) or moved with the arrows.
  const splitDrag = useRef(false);
  const splitTo = (e: PointerEvent<HTMLDivElement>) => {
    const r = ref.current?.getBoundingClientRect();
    const w = r && r.width > 0 ? r.width : stage.w;
    if (w > 0) setSplit(Math.min(1, Math.max(0, (e.clientX - (r?.left ?? 0)) / w)));
  };

  const shown = zoomed && measured && photo && p;
  /** One photo layer: the after (with its slide between photos) or the before. */
  const layer = (which: "after" | "before") => {
    if (which === "after")
      return (
        <div className="develop-canvas__layer">
  {!p && (
    <div className="develop-canvas__wait">
      <Spinner size="md" label="Developing" />
    </div>
  )}
  {leaving && !shown && (
    <img
      key={`leaving-${leaving.assetId}`}
      className="develop-canvas__img develop-canvas__img--leaving"
      style={{ "--dir": leaving.direction } as React.CSSProperties}
      src={leaving.url}
      width={leaving.width || undefined}
      height={leaving.height || undefined}
      alt=""
      aria-hidden
      draggable={false}
      onAnimationEnd={() => setLeaving(null)}
    />
  )}
  {p && !shown && (
    <img
      key={p.assetId}
      className="develop-canvas__img"
      data-photo={p.assetId}
      data-direction={direction.current}
      data-arriving={leaving ? "true" : undefined}
      style={{ "--dir": direction.current } as React.CSSProperties}
      src={p.url}
      width={p.width || undefined}
      height={p.height || undefined}
      alt="The photo as developed"
      draggable={false}
      ref={measureImg}
      onLoad={(e) => measureImg(e.currentTarget)}
    />
  )}
  {masking && p && !shown && imgBox && (
    <div className="develop-canvas__masks" style={imgBox}>
      {overlay(imgBox)}
    </div>
  )}
  {shown && (
    <div
      className="develop-canvas__frame"
      style={{ width: photo.w * scale, height: photo.h * scale, transform: `translate(${-pan.x * scale}px, ${-pan.y * scale}px)` }}
    >
      <img className="develop-canvas__under" src={p.url} alt="The photo as developed" draggable={false} />
      {region && region.tag === regionTag && (
        <img
          className="develop-canvas__region"
          src={region.url}
          alt=""
          draggable={false}
          style={{ left: region.x * scale, top: region.y * scale, width: region.w * scale, height: region.h * scale }}
        />
      )}
      {masking && overlay({ width: photo.w * scale, height: photo.h * scale })}
    </div>
  )}
        </div>
      );
    const clip = compare === "split" ? { clipPath: `inset(0 ${(1 - split) * 100}% 0 0)` } : undefined;
    return (
      <div className="develop-canvas__layer develop-canvas__layer--before" style={clip}>
        {!bv && (
          <div className="develop-canvas__wait">
            <Spinner size="sm" label="Rendering the before" />
          </div>
        )}
        {bv && !shown && <img className="develop-canvas__img" src={bv.url} width={bv.width || undefined} height={bv.height || undefined} alt="" draggable={false} />}
        {bv && shown && (
          <div
            className="develop-canvas__frame"
            style={{ width: photo.w * scale, height: photo.h * scale, transform: `translate(${-pan.x * scale}px, ${-pan.y * scale}px)` }}
          >
            <img className="develop-canvas__under" src={bv.url} alt="" draggable={false} />
            {beforeRegion && beforeRegion.tag === beforeTag && (
              <img
                className="develop-canvas__region"
                src={beforeRegion.url}
                alt=""
                draggable={false}
                style={{ left: beforeRegion.x * scale, top: beforeRegion.y * scale, width: beforeRegion.w * scale, height: beforeRegion.h * scale }}
              />
            )}
          </div>
        )}
      </div>
    );
  };
  const panes =
    compare === "side" ? (
      <>
        <div className="develop-canvas__pane">
          {layer("before")}
          <span className="develop-canvas__tag">Before</span>
        </div>
        <div className="develop-canvas__pane">
          {layer("after")}
          <span className="develop-canvas__tag">After</span>
        </div>
      </>
    ) : (
      <div className="develop-canvas__pane">
        {layer("after")}
        {compare === "split" && layer("before")}
        {compare === "split" && (
          <div
            className="develop-canvas__divider"
            role="separator"
            aria-orientation="vertical"
            aria-label="Divider between before and after"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(split * 100)}
            tabIndex={0}
            style={{ left: `${split * 100}%` }}
            onPointerDown={(e) => {
              e.stopPropagation();
              e.currentTarget.setPointerCapture?.(e.pointerId);
              splitDrag.current = true;
            }}
            onPointerMove={(e) => {
              if (!splitDrag.current) return;
              e.stopPropagation();
              splitTo(e);
            }}
            onPointerUp={(e) => {
              e.stopPropagation();
              splitDrag.current = false;
            }}
            onDoubleClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => {
              if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
              e.preventDefault();
              e.stopPropagation();
              setSplit((v) => Math.min(1, Math.max(0, v + (e.key === "ArrowRight" ? 0.05 : -0.05))));
            }}
          >
            <span className="develop-canvas__tag develop-canvas__tag--before">Before</span>
            <span className="develop-canvas__tag develop-canvas__tag--after">After</span>
          </div>
        )}
      </div>
    );
  const seen = shown ? visibleRect(pan, { mode: "level", level }, photo, view, dpr) : null;
  const navSize = photo ? (photo.w >= photo.h ? { w: NAV, h: (NAV * photo.h) / photo.w } : { w: (NAV * photo.w) / photo.h, h: NAV }) : null;
  return (
    <div
      ref={ref}
      className="develop-canvas"
      data-zoomed={zoomed}
      data-space={space || undefined}
      data-compare={compare}
      data-axis={axis ?? undefined}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onDoubleClick={dbl}
      onContextMenu={
        actions
          ? (e) =>
              openMenu(
                e,
                developCanvasMenu(
                  { compare, clipping: session.clipping, labels: LABELS, openWith },
                  {
                    ...actions,
                    setCompare: session.setCompare,
                    setClipping: session.setClipping,
                    zoom: (to) => zoomTo(to === "fit" ? FIT : to === "fill" ? { mode: "fill" } : { mode: "level", level: to }),
                  },
                ),
              )
          : undefined
      }
      aria-label={zoomed ? `Photo at ${percent(level)}, drag to move around` : "Photo, double-click to zoom in"}
    >
      {panes}
      {shown && seen && navSize && (
        <div
          className="develop-canvas__nav"
          role="img"
          aria-label="Navigator"
          style={{ width: navSize.w, height: navSize.h }}
          onPointerDown={(e) => {
            e.stopPropagation();
            e.currentTarget.setPointerCapture?.(e.pointerId);
            navDrag.current = true;
            navTo(e);
          }}
          onPointerMove={(e) => {
            e.stopPropagation();
            if (navDrag.current) navTo(e);
          }}
          onPointerUp={(e) => {
            e.stopPropagation();
            navDrag.current = false;
          }}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <img src={p.url} alt="" draggable={false} />
          <span
            className="develop-canvas__nav-view"
            style={{ left: `${seen.x * 100}%`, top: `${seen.y * 100}%`, width: `${seen.w * 100}%`, height: `${seen.h * 100}%` }}
          />
        </div>
      )}
      {session.before && <span className="develop-canvas__badge">Before</span>}
      {photo && <ZoomBar zoom={zoom} shown={level} onZoom={zoomTo} onStep={step} compare={compare} onCompare={session.setCompare} />}
      {slow && p && (
        <div className="develop-canvas__busy">
          <Spinner size="sm" label="Developing" />
        </div>
      )}
      {marks && <Marks marks={marks} />}
    </div>
  );
}
