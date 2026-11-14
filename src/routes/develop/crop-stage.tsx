import { useEffect, useRef, useState, type PointerEvent } from "react";
import { EditCanvas, nearestSpin, shownGrid } from "@/components/crop-tool";
import { useFrameAspect } from "@/components/develop-panels";
import { Spinner } from "@/components/spinner";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { cropAspect, cropUi, levelFromLine } from "@/lib/develop-tools";

type Point = { x: number; y: number };

/**
 * The canvas while the crop tool is open (Phase 8e): the photo rendered without geometry,
 * laid out by the quick editor's own `EditCanvas` with the crop rectangle over it, so turns,
 * flips and straighten animate and cost no render. With *Draw a level line* on, a drag across
 * the photo straightens it so the line comes out level.
 */
export function CropStage({ session, assetId }: { session: DevelopSession; assetId: number }) {
  const ui = cropUi.use();
  const r = session.draft;
  const frameAspect = useFrameAspect(assetId, r);
  const imageAspect = r.rotate % 180 === 90 ? 1 / frameAspect : frameAspect;
  const p = session.preview;
  const src = p && p.assetId === assetId && p.uncropped ? p.url : null;
  const [spin, setSpin] = useState(r.rotate);
  useEffect(() => setSpin((s) => nearestSpin(s, r.rotate)), [r.rotate]);
  const [straightening, setStraightening] = useState(false);

  const line = useRef<{ from: Point; box: DOMRect } | null>(null);
  const [drawn, setDrawn] = useState<{ from: Point; to: Point } | null>(null);
  const at = (e: PointerEvent<HTMLDivElement>, box: DOMRect): Point => ({ x: e.clientX - box.left, y: e.clientY - box.top });
  const down = (e: PointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    const box = e.currentTarget.getBoundingClientRect();
    const from = at(e, box);
    line.current = { from, box };
    setDrawn({ from, to: from });
    setStraightening(true);
  };
  const move = (e: PointerEvent<HTMLDivElement>) => {
    const l = line.current;
    if (l) setDrawn({ from: l.from, to: at(e, l.box) });
  };
  const up = (e: PointerEvent<HTMLDivElement>) => {
    const l = line.current;
    line.current = null;
    setDrawn(null);
    setStraightening(false);
    if (!l) return;
    const to = at(e, l.box);
    const next = levelFromLine(session.draft.straighten, l.from, to);
    if (next !== session.draft.straighten) session.applyRecipe((d) => ({ ...d, straighten: next }), "Straighten");
    cropUi.set((u) => ({ ...u, leveling: false }));
  };

  return (
    <div className="develop-crop-stage">
      <EditCanvas
        src={src}
        alt="The photo being cropped"
        initialAspect={imageAspect}
        recipe={r}
        spin={spin}
        cropping
        aspect={cropAspect(ui, frameAspect)}
        grid={shownGrid(ui.grid, true, straightening)}
        live={straightening}
        onCrop={(rect) => session.editRecipe((d) => ({ ...d, crop: rect }))}
      />
      {!src && (
        <div className="develop-crop-stage__wait">
          <Spinner size="md" label="Rendering the whole photo" />
        </div>
      )}
      {ui.leveling && (
        <div
          className="develop-crop-stage__level"
          role="application"
          aria-label="Draw along something that should be level"
          onPointerDown={down}
          onPointerMove={move}
          onPointerUp={up}
          onPointerCancel={() => {
            line.current = null;
            setDrawn(null);
            setStraightening(false);
          }}
        >
          {drawn && (
            <svg className="develop-crop-stage__line" aria-hidden>
              <line className="develop-crop-stage__stroke" x1={drawn.from.x} y1={drawn.from.y} x2={drawn.to.x} y2={drawn.to.y} />
            </svg>
          )}
        </div>
      )}
    </div>
  );
}
