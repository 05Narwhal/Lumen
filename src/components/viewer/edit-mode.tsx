import { Button, Tooltip } from "@mantine/core";
import { IconArrowBackUp, IconArrowForwardUp } from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Spinner } from "@/components/spinner";
import { toast } from "@/components/toast";
import { isMac } from "@/components/window-controls";
import { useEditSession, type EditSession } from "@/hooks/use-edit-session";
import { matches } from "@/lib/keybinds";
import { cutRange, keepRange, segmentsOf, trimFor } from "@/lib/trim";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import type { Segment } from "@/types/generated/Segment";
import { mediaUrl } from "@/utils/media";
import { EditCanvas, fitAspect, flipShown, nearestSpin, rotateBy, shownGrid, type GridMode } from "@/components/crop-tool";
import { EditToolbar, GRID_ORDER, type AspectChoice } from "./edit-toolbar";
import { TrimPanel, TrimTimeline, type Selection } from "./trim-panel";
import { VideoPlayer } from "./video-player";

const GRID_KEY = "lumen.edit.grid";

function readGrid(): GridMode {
  try {
    const v = localStorage.getItem(GRID_KEY);
    return GRID_ORDER.includes(v as GridMode) ? (v as GridMode) : "auto";
  } catch {
    return "auto";
  }
}

const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);

/**
 * The viewer in edit mode (`?asset=<id>&edit=1`): a photo gets the quick editor (crop,
 * turns, flips, straighten), a video the trimmer. Both fill the viewer, with their controls
 * floating over the bottom and Undo / Redo, Revert, Cancel and Done in the top bar (UX spec
 * §6). Escape and Cancel discard the draft without asking, because nothing was saved.
 */
export function EditMode({ asset, fallbackSrc, onExit }: { asset: AssetDetail; fallbackSrc: string; onExit: () => void }) {
  return asset.mediaType === 1 ? <TrimMode asset={asset} onExit={onExit} /> : <PhotoEditMode asset={asset} fallbackSrc={fallbackSrc} onExit={onExit} />;
}

function EditBar({ asset, session, onExit, busy }: { asset: AssetDetail; session: EditSession; onExit: () => void; busy?: boolean }) {
  const mod = isMac() ? "⌘" : "Ctrl+";
  return (
    <div className="viewer__top edit-bar">
      <div className="viewer__meta">
        <span className="viewer__title-row">
          <span className="viewer__name">Editing {asset.filename}</span>
          {busy && <Spinner label="Rendering the preview" />}
        </span>
        <span className="viewer__sub">The original is never changed.</span>
      </div>
      <div className="viewer__spacer" />
      <div className="edit-bar__history">
        <Tooltip label={`Undo (${mod}Z)`} openDelay={500} withArrow>
          <button type="button" className="viewer__icon" aria-label="Undo" disabled={!session.canUndo} onClick={session.undo}>
            <IconArrowBackUp size={18} />
          </button>
        </Tooltip>
        <Tooltip label={`Redo (${mod}Shift+Z)`} openDelay={500} withArrow>
          <button type="button" className="viewer__icon" aria-label="Redo" disabled={!session.canRedo} onClick={session.redo}>
            <IconArrowForwardUp size={18} />
          </button>
        </Tooltip>
      </div>
      {session.saved && (
        <Button
          variant="subtle"
          onClick={async () => {
            await session.revert();
            onExit();
          }}
        >
          Revert to original
        </Button>
      )}
      <Button
        variant="default"
        onClick={() => {
          session.cancel();
          onExit();
        }}
      >
        Cancel
      </Button>
      <Button
        onClick={async () => {
          await session.done();
          onExit();
        }}
        disabled={!session.loaded}
      >
        Done
      </Button>
    </div>
  );
}

/** Undo and redo (Ctrl/Cmd+Z, Ctrl/Cmd+Shift+Z, Ctrl+Y by default), for both editors. */
function historyKey(e: KeyboardEvent, session: EditSession, scope: "editor" | "trim"): boolean {
  if (matches(e, `${scope}.undo`)) session.undo();
  else if (matches(e, `${scope}.redo`)) session.redo();
  else return false;
  e.preventDefault();
  return true;
}

/**
 * A video's trimmer. The kept segments are the draft; the player previews them (skipping
 * the cuts) and the timeline and controls under it edit them.
 */
function TrimMode({ asset, onExit }: { asset: AssetDetail; onExit: () => void }) {
  const session = useEditSession(asset.id, { preview: false });
  const video = useRef<HTMLVideoElement>(null);
  const [timeMs, setTimeMs] = useState(0);
  const [durationMs, setDurationMs] = useState(asset.durationMs ?? 0);
  const [selection, setSelection] = useState<Selection | null>(null);
  const segments = segmentsOf(session.draft.trim, durationMs);

  const seek = (ms: number) => {
    if (video.current) video.current.currentTime = ms / 1000;
    setTimeMs(ms);
  };
  const apply = (next: Segment[] | null) => {
    if (!next) {
      toast({ message: "That would leave nothing to play." });
      return;
    }
    session.set({ trim: trimFor(next, durationMs) });
    setSelection(null);
  };
  const cut = () => selection && apply(cutRange(segments, selection.a, selection.b));
  const keep = () => selection && apply(keepRange(segments, selection.a, selection.b));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return;
      if (historyKey(e, session, "trim")) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (matches(e, "trim.cancel")) {
        e.preventDefault();
        if (selection) setSelection(null);
        else {
          session.cancel();
          onExit();
        }
      } else if (matches(e, "trim.mark-in")) {
        setSelection({
          a: timeMs,
          b: Math.max(timeMs, selection?.b ?? durationMs),
        });
      } else if (matches(e, "trim.mark-out")) {
        setSelection({ a: Math.min(timeMs, selection?.a ?? 0), b: timeMs });
      } else if (matches(e, "trim.cut") && selection) {
        e.preventDefault();
        cut();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  return (
    <>
      <div className="viewer__stage edit-stage trim-stage">
        <div className="trim-stage__video">
          <VideoPlayer
            id={asset.id}
            extension={asset.extension}
            trim={session.draft.trim}
            videoRef={video}
            onTime={(ms) => {
              setTimeMs(ms);
              const d = video.current?.duration;
              if (!durationMs && d && Number.isFinite(d)) setDurationMs(d * 1000);
            }}
          />
        </div>
        {durationMs > 0 && (
          <div className="trim-stage__controls">
            <TrimTimeline
              durationMs={durationMs}
              segments={segments}
              selection={selection}
              timeMs={timeMs}
              posterUrl={mediaUrl("thumb", asset.id, { s: 256 })}
              onSeek={seek}
              onSelect={setSelection}
            />
            <TrimPanel
              durationMs={durationMs}
              segments={segments}
              selection={selection}
              timeMs={timeMs}
              onSelect={setSelection}
              onCut={cut}
              onKeep={keep}
              onReset={() => {
                session.set({ trim: null });
                setSelection(null);
              }}
            />
          </div>
        )}
      </div>
      <EditBar asset={asset} session={session} onExit={onExit} />
    </>
  );
}

function PhotoEditMode({ asset, fallbackSrc, onExit }: { asset: AssetDetail; fallbackSrc: string; onExit: () => void }) {
  const [cropping, setCropping] = useState(false);
  const [aspect, setAspect] = useState<AspectChoice>(null);
  const [grid, setGridState] = useState<GridMode>(readGrid);
  const [straightening, setStraightening] = useState(false);
  const session = useEditSession(asset.id, { clientGeometry: true });
  const { draft, set } = session;

  // The shape as displayed, before any edit: stored sizes are the sensor's, so orientations
  // 5–8 swap them.
  const [w0, h0] = (asset.orientation ?? 1) >= 5 ? [asset.height, asset.width] : [asset.width, asset.height];
  const initialAspect = w0 && h0 ? w0 / h0 : 1.5;
  const [frameAspect, setFrameAspect] = useState(initialAspect);

  // The on-screen quarter-turn angle follows the recipe the short way round, so 270 → 0
  // turns a quarter rather than three.
  const [spin, setSpin] = useState(draft.rotate);
  useEffect(() => setSpin((s) => nearestSpin(s, draft.rotate)), [draft.rotate]);

  const setGrid = useCallback((g: GridMode) => {
    setGridState(g);
    try {
      localStorage.setItem(GRID_KEY, g);
    } catch {
      // A private window keeps the choice for this session only.
    }
  }, []);

  const chooseAspect = (a: AspectChoice) => {
    setAspect(a);
    const ratio = a === "original" ? frameAspect : a;
    if (ratio)
      set({
        crop: fitAspect(draft.crop ?? { x: 0, y: 0, w: 1, h: 1 }, ratio, frameAspect),
      });
  };
  const numericAspect = aspect === "original" ? frameAspect : aspect;

  // The crop in the original's pixels, for cropping to an exact size.
  const quarter = draft.rotate % 180 === 90;
  const [fw, fh] = quarter ? [h0, w0] : [w0, h0];
  const crop = draft.crop ?? { x: 0, y: 0, w: 1, h: 1 };
  const cropSize = fw && fh ? { width: Math.round(fw * crop.w), height: Math.round(fh * crop.h) } : null;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target)) return;
      if (historyKey(e, session, "editor")) return;
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      if (matches(e, "editor.cancel")) {
        e.preventDefault();
        if (cropping) setCropping(false);
        else {
          session.cancel();
          onExit();
        }
      } else if (matches(e, "editor.done-crop")) {
        if (cropping) {
          e.preventDefault();
          setCropping(false);
        }
      } else if (matches(e, "editor.crop")) setCropping((v) => !v);
      else if (matches(e, "editor.rotate-left")) set((d) => rotateBy(d, -1));
      else if (matches(e, "editor.rotate-right")) set((d) => rotateBy(d, 1));
      else if (matches(e, "editor.flip-h")) set((d) => flipShown(d, "h"));
      else if (matches(e, "editor.flip-v")) set((d) => flipShown(d, "v"));
      else if (matches(e, "editor.grid")) setGrid(GRID_ORDER[(GRID_ORDER.indexOf(grid) + 1) % GRID_ORDER.length]);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // Until Rust's unedited render arrives, the viewer's picture stands in — but only when
  // nothing is saved, or the saved geometry would be drawn twice.
  const src = session.previewUrl ?? (session.loaded && !session.saved ? fallbackSrc : null);

  return (
    <>
      <div className="viewer__stage edit-stage">
        <EditCanvas
          src={src}
          alt={asset.filename}
          initialAspect={initialAspect}
          recipe={draft}
          spin={spin}
          cropping={cropping}
          aspect={numericAspect}
          grid={shownGrid(grid, cropping, straightening)}
          live={straightening}
          onCrop={(r) => set({ crop: r }, "crop")}
          onFrameAspect={setFrameAspect}
        />
      </div>
      <EditBar asset={asset} session={session} onExit={onExit} busy={session.rendering && !session.previewUrl} />
      <div className="edit-dock">
        <EditToolbar
          session={session}
          cropping={cropping}
          onCropping={setCropping}
          aspect={aspect}
          onAspect={chooseAspect}
          grid={grid}
          onGrid={setGrid}
          onStraightening={setStraightening}
          cropSize={cropping ? cropSize : null}
        />
      </div>
    </>
  );
}
