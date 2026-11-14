import { ActionIcon, Button, NumberInput, SegmentedControl, Tooltip } from "@mantine/core";
import { IconCrop, IconFlipHorizontal, IconFlipVertical, IconLine, IconRotate2, IconRotateClockwise2 } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { fitAspect, flipShown, rotateBy, type GridMode } from "@/components/crop-tool";
import { DevelopSlider } from "@/components/develop-slider";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { hasGeometry, signed } from "@/lib/develop";
import { CROP_ASPECTS, cropAspect, cropUi, FULL_CROP } from "@/lib/develop-tools";
import { invoke } from "@/lib/ipc";
import type { DevelopBaseInfo } from "@/types/generated/DevelopBaseInfo";
import type { Recipe } from "@/types/generated/Recipe";

const GRIDS: { label: string; value: GridMode }[] = [
  { label: "Thirds", value: "thirds" },
  { label: "Grid", value: "fine" },
  { label: "Auto", value: "auto" },
  { label: "Off", value: "off" },
];

/** Open the crop tool (`R`). */
export function openCrop(session: DevelopSession) {
  cropUi.set((u) => ({ ...u, leveling: false }));
  session.setTool("crop");
}

/** *Done* or Enter: the drags since the last step become one step, *Crop*. */
export function cropDone(session: DevelopSession) {
  session.commit("Crop");
  cropUi.set((u) => ({ ...u, leveling: false }));
  session.setTool(null);
}

/** Esc: the drags since the last step are dropped. Turns and flips are steps already. */
export function cropCancel(session: DevelopSession) {
  session.discard();
  cropUi.set((u) => ({ ...u, leveling: false }));
  session.setTool(null);
}

/** The frame's width ÷ height after the recipe's quarter turns. */
export function useFrameAspect(assetId: number, recipe: Recipe): number {
  const info = useQuery({ queryKey: ["develop-base-info", assetId], queryFn: () => invoke<DevelopBaseInfo>("develop_base_info", { assetId }) });
  const w = info.data?.width || 3;
  const h = info.data?.height || 2;
  return recipe.rotate % 180 === 90 ? h / w : w / h;
}

/**
 * *Crop* beside *Auto* in the tools row: opens the tool, or finishes it when open. It reads as
 * pressed while the tool is open and wears the accent while the photo has a crop or a turn.
 */
export function CropButton({ session }: { session: DevelopSession }) {
  const open = session.tool === "crop";
  return (
    <Button
      size="compact-sm"
      variant={open ? "filled" : hasGeometry(session.draft) ? "light" : "default"}
      leftSection={<IconCrop size={14} />}
      aria-pressed={open}
      onClick={() => (open ? cropDone(session) : openCrop(session))}
      disabled={!session.loaded}
      title="Crop and straighten (R)"
    >
      Crop
    </Button>
  );
}

/**
 * The Crop tool (Phase 8e), opened by *Crop* in the tools row: the aspect presets, turns and
 * flips, straighten with a level line, the guides, *Reset crop*, and *Done* / *Cancel*. The
 * canvas draws the crop rectangle meanwhile. Closed, it takes no room at all.
 */
export function CropPanel({ session }: { session: DevelopSession }) {
  const ui = cropUi.use();
  const frameAspect = useFrameAspect(session.assetId, session.draft);
  const r = session.draft;
  if (session.tool !== "crop") return null;

  const choose = (value: number | "original" | null | "custom") => {
    const aspect = value === "custom" ? ui.custom[0] / ui.custom[1] : value;
    cropUi.set((u) => ({ ...u, aspect }));
    const ratio = cropAspect({ ...ui, aspect }, frameAspect);
    if (ratio) session.editRecipe((d) => ({ ...d, crop: fitAspect(d.crop ?? FULL_CROP, ratio, frameAspect) }));
  };
  const setCustom = (i: 0 | 1, v: number | string) => {
    const n = typeof v === "number" ? v : Number.parseFloat(v);
    if (!Number.isFinite(n) || n <= 0) return;
    const custom: [number, number] = i === 0 ? [n, ui.custom[1]] : [ui.custom[0], n];
    cropUi.set((u) => ({ ...u, custom }));
    const ratio = custom[0] / custom[1];
    cropUi.set((u) => ({ ...u, aspect: ratio }));
    session.editRecipe((d) => ({ ...d, crop: fitAspect(d.crop ?? FULL_CROP, ratio, frameAspect) }));
  };
  const isCustom = typeof ui.aspect === "number" && !CROP_ASPECTS.some((a) => typeof a.value === "number" && Math.abs(a.value - (ui.aspect as number)) < 1e-6);
  const active = (v: number | "original" | null | "custom") => (v === "custom" ? isCustom : typeof v === "number" ? typeof ui.aspect === "number" && Math.abs(v - ui.aspect) < 1e-6 : v === ui.aspect);

  return (
    <section className="develop-crop" aria-label="Crop">
      <div className="develop-crop__head">
        <span className="develop-crop__title">Crop</span>
        <div className="develop-crop__actions">
          {hasGeometry(r) && (
            <button type="button" className="develop-panel__link" onClick={() => session.applyRecipe((d) => ({ ...d, crop: null, rotate: 0, straighten: 0, flip_h: false, flip_v: false }), "Reset crop")}>
              Reset crop
            </button>
          )}
          <Button size="compact-sm" variant="subtle" onClick={() => cropCancel(session)}>
            Cancel
          </Button>
          <Button size="compact-sm" onClick={() => cropDone(session)}>
            Done
          </Button>
        </div>
      </div>
      <div className="develop-crop__aspects" role="group" aria-label="Aspect ratio">
        {CROP_ASPECTS.map((a) => (
          <button key={a.label} type="button" className="develop-crop__chip" aria-pressed={active(a.value)} onClick={() => choose(a.value)}>
            {a.label}
          </button>
        ))}
      </div>
      {isCustom && (
        <div className="develop-crop__custom">
          <NumberInput size="xs" aria-label="Width" min={0.1} step={1} value={ui.custom[0]} onChange={(v) => setCustom(0, v)} />
          <span aria-hidden>:</span>
          <NumberInput size="xs" aria-label="Height" min={0.1} step={1} value={ui.custom[1]} onChange={(v) => setCustom(1, v)} />
        </div>
      )}
      <div className="develop-crop__tools">
        <Tooltip label="Rotate left">
          <ActionIcon variant="default" aria-label="Rotate left" onClick={() => session.applyRecipe((d) => rotateBy(d, -1), "Rotate")}>
            <IconRotate2 size={16} />
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Rotate right">
          <ActionIcon variant="default" aria-label="Rotate right" onClick={() => session.applyRecipe((d) => rotateBy(d, 1), "Rotate")}>
            <IconRotateClockwise2 size={16} />
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Flip horizontal">
          <ActionIcon variant="default" aria-label="Flip horizontal" onClick={() => session.applyRecipe((d) => flipShown(d, "h"), "Flip")}>
            <IconFlipVertical size={16} />
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Flip vertical">
          <ActionIcon variant="default" aria-label="Flip vertical" onClick={() => session.applyRecipe((d) => flipShown(d, "v"), "Flip")}>
            <IconFlipHorizontal size={16} />
          </ActionIcon>
        </Tooltip>
        <Tooltip label="Draw a level line">
          <ActionIcon
            variant={ui.leveling ? "filled" : "default"}
            aria-label="Draw a level line"
            aria-pressed={ui.leveling}
            onClick={() => cropUi.set((u) => ({ ...u, leveling: !u.leveling }))}
          >
            <IconLine size={16} />
          </ActionIcon>
        </Tooltip>
      </div>
      <DevelopSlider
        label="Straighten"
        value={r.straighten}
        min={-45}
        max={45}
        step={0.1}
        format={(v) => `${signed(v, 1)}°`}
        onChange={(v) => session.editRecipe((d) => ({ ...d, straighten: v }))}
        onCommit={(v) => session.applyRecipe((d) => ({ ...d, straighten: v }), "Straighten")}
      />
      <div className="develop-crop__guides">
        <span>Guides</span>
        <SegmentedControl size="xs" data={GRIDS} value={ui.grid} onChange={(g) => cropUi.set((u) => ({ ...u, grid: g as GridMode }))} aria-label="Guides" />
      </div>
    </section>
  );
}
