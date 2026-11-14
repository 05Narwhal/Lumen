import { ActionIcon, Button, Menu, Switch } from "@mantine/core";
import { IconBrush, IconDots, IconEye, IconEyeOff, IconMinus, IconPlus } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Reorder } from "framer-motion";
import { Fragment, useEffect, useRef, useState } from "react";
import { AiButton } from "@/components/ai-button";
import { promptName } from "@/components/confirm-dialog";
import { Items, type MenuItem, useContextMenu } from "@/components/context-menu";
import { DevelopSlider } from "@/components/develop-slider";
import { toast } from "@/components/toast";
import { useAiTask } from "@/hooks/use-ai-task";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { addAiShape, AI_NAMES, missingRasters, newAiMask, retarget, useAiMasks, useLoadedRasters } from "@/lib/ai-masks";
import { developOf, ZERO_ADJUST } from "@/lib/develop";
import { invoke } from "@/lib/ipc";
import { matches } from "@/lib/keybinds";
import { aiMaskMenu, type MaskActions, maskMenu } from "@/lib/menus/masks";
import { queryKeys } from "@/lib/query-keys";
import type { AiMaskInfo } from "@/types/generated/AiMaskInfo";
import type { AiMaskKind } from "@/types/generated/AiMaskKind";
import type { AiMaskList } from "@/types/generated/AiMaskList";
import {
  addShape,
  brushPixels,
  brushPosition,
  brushRadiusOf,
  brushSizeAt,
  duplicateMask,
  maskUi,
  moveMask,
  newMask,
  removeMask,
  SHAPE_NAMES,
  updateMask,
  type ShapeKind,
} from "@/lib/develop-masks";
import { type AiRasters, frameMap, rasterize } from "@/lib/mask-raster";
import type { DevelopBaseInfo } from "@/types/generated/DevelopBaseInfo";
import type { Mask } from "@/types/generated/Mask";
import { aiMaskRequests } from "./requests";
import "./styles.scss";

const KINDS: ShapeKind[] = ["brush", "linear", "radial"];

/** A small picture of a mask's weights over the photo's shape. */
function Thumb({ mask, aspect, rasters }: { mask: Mask; aspect: number; rasters: AiRasters }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const w = aspect >= 1 ? 40 : Math.max(8, Math.round(40 * aspect));
  const h = aspect >= 1 ? Math.max(8, Math.round(40 / aspect)) : 40;
  useEffect(() => {
    const ctx = ref.current?.getContext?.("2d");
    if (!ctx) return;
    const weights = rasterize(mask, frameMap(w, h, { rotate: 0, flip_h: false, flip_v: false, straighten: 0, crop: null }), w, h, rasters);
    const img = ctx.createImageData(w, h);
    for (let i = 0; i < weights.length; i++) {
      const v = Math.round(weights[i] * 255);
      img.data.set([v, v, v, 255], i * 4);
    }
    ctx.putImageData(img, 0, 0);
  }, [mask, w, h, rasters]);
  return <canvas ref={ref} className="develop-masks__thumb" width={w} height={h} aria-hidden />;
}

/** The hand-drawn shapes, then (Facet on and installed) the AI ones under a divider. */
function ShapeEntries({ hand, ai }: { hand: (k: ShapeKind) => void; ai: MenuItem[] | null }) {
  return (
    <>
      {KINDS.map((k) => (
        <Menu.Item key={k} onClick={() => hand(k)}>
          {SHAPE_NAMES[k]}
        </Menu.Item>
      ))}
      {ai && (
        <Fragment>
          <Menu.Divider />
          <Menu.Label>AI</Menu.Label>
          {/* The panel sits at the window's right edge: submenus open towards the photo, not over the row. */}
          <Items items={ai} subPosition="left-start" />
        </Fragment>
      )}
    </>
  );
}

const noneFound = (kind: AiMaskKind) => (kind === "person" ? "No people found" : `No ${AI_NAMES[kind].toLowerCase()} found`);

/**
 * The Masks panel (Phase 8e): the photo's masks, *New mask*, and the selected mask's shapes,
 * invert, opacity and local sliders. Selecting a mask hands the canvas to it: its handles, the
 * brush and the overlay.
 */
export function DevelopMasks({ session, aspect }: { session: DevelopSession; aspect: number }) {
  const ui = maskUi.use();
  const masks = developOf(session.draft).masks ?? [];
  const selected = masks.find((m) => m.id === ui.selected) ?? null;
  // The full photo's long edge in pixels, which the brush's Size is measured in (shared with the canvas's query).
  const baseInfo = useQuery({ queryKey: ["develop-base-info", session.assetId], queryFn: () => invoke<DevelopBaseInfo>("develop_base_info", { assetId: session.assetId }) });
  const edge = Math.max(baseInfo.data?.width ?? 0, baseInfo.data?.height ?? 0) || 4000;

  const editMasks = (change: (m: Mask[]) => Mask[]) => session.update((d) => ({ ...d, masks: change(d.masks ?? []) }));
  const applyMasks = (change: (m: Mask[]) => Mask[], label: string) => session.apply((d) => ({ ...d, masks: change(d.masks ?? []) }), label);

  const select = (id: number | null) => {
    maskUi.set((u) => ({ ...u, selected: id, painting: null }));
    session.setTool(id === null ? null : "masks");
  };

  // A mask that went away (undo, another photo) is no longer selected.
  useEffect(() => {
    if (ui.selected !== null && !selected && session.loaded) select(null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: only when the selection dangles
  }, [ui.selected, selected, session.loaded]);

  const create = (kind: ShapeKind) => {
    const m = newMask(masks, kind);
    applyMasks((all) => [...all, m], "New mask");
    maskUi.set((u) => ({ ...u, selected: m.id, painting: kind === "brush" ? { add: true } : null }));
    session.setTool("masks");
  };

  const shape = (kind: ShapeKind, add: boolean) => {
    if (!selected) return;
    if (kind === "brush") {
      maskUi.set((u) => ({ ...u, painting: { add } }));
      return;
    }
    applyMasks((all) => addShape(all, selected.id, kind, add), add ? "Add to mask" : "Subtract from mask");
  };

  // Phase 8f: Lumen Facet's masks, offered only while its switch is on and the model is here.
  const client = useQueryClient();
  const ai = useAiMasks(session.assetId);
  const aiTask = useAiTask<AiMaskList>("masks");
  const { fetched, rasters } = useLoadedRasters(session.assetId, masks);

  /** Runs Facet on the open photo (or answers from what it found); null when cancelled. */
  const detect = (again: boolean) =>
    aiTask.run(async (task) => {
      const list = await invoke<AiMaskList>("ai_masks_detect", { assetId: session.assetId, again, task });
      client.setQueryData(queryKeys.aiMasks(session.assetId), list);
      return list;
    });

  /** Finds first when Facet hasn't looked yet, then uses the first mask of `kind`. */
  const findThen = async (kind: AiMaskKind, use: (m: AiMaskInfo) => void) => {
    const list = ai.scanned ? ai : await detect(false);
    if (!list) return;
    const m = list.masks.filter((x) => x.kind === kind).sort((x, y) => x.instance - y.instance)[0];
    if (m) use(m);
    else toast({ message: noneFound(kind) });
  };

  const createAi = (m: AiMaskInfo) => {
    const mask = newAiMask(masks, m);
    applyMasks((all) => [...all, mask], `New ${mask.name.toLowerCase()} mask`);
    maskUi.set((u) => ({ ...u, selected: mask.id, painting: null }));
    session.setTool("masks");
  };
  const shapeAi = (m: AiMaskInfo, add: boolean) => selected && applyMasks((all) => addAiShape(all, selected.id, m, add), add ? "Add to mask" : "Subtract from mask");

  const aiMenu = (use: (m: AiMaskInfo) => void) =>
    ai.available ? aiMaskMenu(ai, { pick: use, find: (k) => void findThen(k, use), again: () => void detect(true) }) : null;

  /**
   * *Detect again* on the selected mask, also the way out of *Not detected on this photo*: points
   * its AI shapes at this photo's rasters, running Facet only when what it has doesn't answer.
   */
  const redetect = async (m: Mask, force: boolean) => {
    const here = !force && ai.scanned ? retarget(m, ai) : null;
    const fixed = here && here !== m ? here : await detect(force || ai.scanned).then((l) => (l ? retarget(m, l) : null));
    if (!fixed) return;
    if (fixed !== m) applyMasks((all) => all.map((x) => (x.id === m.id ? fixed : x)), "Detect mask again");
    else if (missingRasters(m, fetched).length > 0) toast({ message: "Nothing like it on this photo" });
  };

  // The palette's *New sky mask* and the like (`requestAiMask`).
  const asked = aiMaskRequests.use();
  const seen = useRef(asked.n);
  const latest = useRef<(k: Exclude<AiMaskKind, "person">) => void>(() => {});
  latest.current = (k) => ai.available && session.loaded && void findThen(k, createAi);
  useEffect(() => {
    if (asked.n === seen.current || !asked.kind) return;
    seen.current = asked.n;
    latest.current(asked.kind);
  }, [asked]);

  const missing = selected ? missingRasters(selected, fetched) : [];
  const hasAi = !!selected?.shapes.some((s) => s.kind === "ai");

  const remove = (m: Mask) => {
    applyMasks((all) => removeMask(all, m.id), "Delete mask");
    if (ui.selected === m.id) select(null);
    toast({ message: `Deleted ${m.name}`, undo: () => session.undo() });
  };

  const rename = async (m: Mask) => {
    const name = await promptName({ title: "Rename mask", initial: m.name });
    if (name?.trim()) applyMasks((all) => updateMask(all, m.id, { name: name.trim() }), "Rename mask");
  };

  /** The list reads like layers (Round 21 §3): the mask applied last is the top row. */
  const move = (m: Mask, to: number) => {
    const from = masks.findIndex((x) => x.id === m.id);
    const last = masks.length - 1;
    if (from < 0 || Math.min(last, Math.max(0, to)) === from) return;
    const label = to >= last ? "Bring mask to front" : to <= 0 ? "Send mask to back" : to > from ? "Bring mask forward" : "Send mask backward";
    applyMasks((all) => moveMask(all, m.id, to), label);
  };

  const actionsFor = (m: Mask): MaskActions => ({
    rename: () => void rename(m),
    duplicate: () => applyMasks((all) => duplicateMask(all, m.id), "Duplicate mask"),
    invert: () => applyMasks((all) => updateMask(all, m.id, { invert: !m.invert }), "Invert mask"),
    toggleVisible: () => applyMasks((all) => updateMask(all, m.id, { visible: !m.visible }), m.visible ? "Hide mask" : "Show mask"),
    moveTo: (to) => move(m, to),
    resetAdjust: () => applyMasks((all) => updateMask(all, m.id, { adjust: ZERO_ADJUST }), "Reset mask adjustments"),
    remove: () => remove(m),
  });
  const menuOf = (m: Mask) => maskMenu(m, masks.findIndex((x) => x.id === m.id), masks.length, actionsFor(m));
  const { open: openMenu } = useContextMenu();

  // The order while a row is being dragged; one history step on drop, none without a move.
  const [dragging, setDragging] = useState<Mask[] | null>(null);
  const drop = () => {
    if (!dragging) return;
    const ids = [...dragging].reverse().map((m) => m.id);
    setDragging(null);
    if (ids.some((id, i) => id !== masks[i]?.id)) applyMasks((all) => ids.map((id) => all.find((m) => m.id === id)).filter((m): m is Mask => !!m), "Reorder masks");
  };

  // Mask mode (Round 19 §1): the bar stays at the top of the column while the sections scroll
  // under it; the shapes, brush and Overlay follow, then DevelopPanels' sections on this mask.
  if (selected) {
    return (
      <>
        <div className="develop-masks__bar">
          <div className="develop-masks__bar-row">
            <span className="develop-masks__tag">Mask</span>
            <span className="develop-masks__bar-name" title="Double-click to rename" onDoubleClick={() => void rename(selected)}>
              {selected.name}
            </span>
            <Menu position="bottom-end" withinPortal>
              <Menu.Target>
                <ActionIcon variant="subtle" size="sm" aria-label="Mask options">
                  <IconDots size={14} />
                </ActionIcon>
              </Menu.Target>
              <Menu.Dropdown>
                <Items items={menuOf(selected)} subPosition="left-start" />
              </Menu.Dropdown>
            </Menu>
            <Button size="compact-sm" variant="default" onClick={() => select(null)}>
              Done
            </Button>
          </div>
        </div>
        <section className="develop-masks develop-masks--selected" aria-label="Mask shapes">
          <DevelopSlider
            label="Opacity"
            min={0}
            max={100}
            neutral={100}
            value={selected.opacity * 100}
            format={(v) => `${Math.round(v)} %`}
            onChange={(v) => editMasks((all) => updateMask(all, selected.id, { opacity: v / 100 }))}
            onCommit={(v) => {
              editMasks((all) => updateMask(all, selected.id, { opacity: v / 100 }));
              session.commit("Mask opacity");
            }}
          />
          <div className="develop-masks__tools">
            <Menu withinPortal>
              <Menu.Target>
                <Button size="compact-xs" variant="default" leftSection={<IconPlus size={12} />} aria-label="Add" title="Add">
                  <span className="develop-masks__label">Add</span>
                </Button>
              </Menu.Target>
              <Menu.Dropdown className="develop-masks__menu">
                <ShapeEntries hand={(k) => shape(k, true)} ai={aiMenu((m) => shapeAi(m, true))} />
              </Menu.Dropdown>
            </Menu>
            <Menu withinPortal>
              <Menu.Target>
                <Button size="compact-xs" variant="default" leftSection={<IconMinus size={12} />} aria-label="Subtract" title="Subtract">
                  <span className="develop-masks__label">Subtract</span>
                </Button>
              </Menu.Target>
              <Menu.Dropdown className="develop-masks__menu">
                <ShapeEntries hand={(k) => shape(k, false)} ai={aiMenu((m) => shapeAi(m, false))} />
              </Menu.Dropdown>
            </Menu>
          </div>
          <div className="develop-masks__switches">
            <Switch size="xs" label="Invert" checked={selected.invert} onChange={(e) => applyMasks((all) => updateMask(all, selected.id, { invert: e.currentTarget.checked }), "Invert mask")} />
            <Switch size="xs" label="Overlay (O)" checked={ui.overlay} onChange={(e) => maskUi.set((u) => ({ ...u, overlay: e.currentTarget.checked }))} />
          </div>
          {missing.length > 0 ? (
            <p className="develop-masks__note" role="status">
              Not detected on this photo.{" "}
              {ai.available && (
                <button type="button" className="develop-panel__link" disabled={!!aiTask.state} onClick={() => void redetect(selected, false)}>
                  Detect again
                </button>
              )}
            </p>
          ) : (
            hasAi &&
            ai.available && (
              <button type="button" className="develop-panel__link develop-masks__again" disabled={!!aiTask.state} onClick={() => void redetect(selected, true)}>
                Detect again
              </button>
            )
          )}
          {ui.painting && (
            <div className="develop-masks__brush">
              <div className="develop-masks__brush-head">
                <IconBrush size={14} aria-hidden />
                <span>{ui.painting.add ? "Brush" : "Erase"}</span>
                <button type="button" className="develop-panel__link" onClick={() => maskUi.set((u) => ({ ...u, painting: null }))}>
                  Done painting
                </button>
              </div>
              {/* The diameter in pixels of the full photo, 1 px to 60 % of its long edge, on a log scale. */}
              <DevelopSlider
                label="Size"
                min={0}
                max={100}
                step={0.1}
                neutral={brushPosition(brushPixels(0.05, edge), edge) * 100}
                value={brushPosition(brushPixels(ui.brush.radius, edge), edge) * 100}
                format={(v) => `${Math.round(brushSizeAt(v / 100, edge))} px`}
                onChange={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, radius: brushRadiusOf(brushSizeAt(v / 100, edge), edge) } }))}
                onCommit={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, radius: brushRadiusOf(brushSizeAt(v / 100, edge), edge) } }))}
              />
              <DevelopSlider
                label="Feather"
                min={0}
                max={100}
                neutral={50}
                value={ui.brush.feather * 100}
                format={(v) => String(Math.round(v))}
                onChange={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, feather: v / 100 } }))}
                onCommit={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, feather: v / 100 } }))}
              />
              <DevelopSlider
                label="Flow"
                min={1}
                max={100}
                neutral={100}
                value={ui.brush.flow * 100}
                format={(v) => String(Math.round(v))}
                onChange={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, flow: v / 100 } }))}
                onCommit={(v) => maskUi.set((u) => ({ ...u, brush: { ...u.brush, flow: v / 100 } }))}
              />
            </div>
          )}
        </section>
      </>
    );
  }

  return (
    <section className="develop-masks" aria-label="Masks">
      <div className="develop-masks__head">
        <span className="develop-masks__title">Masks</span>
        {aiTask.state ? (
          // Facet is looking: the button is its progress, and a click cancels (Round 12 §5).
          <AiButton label="Finding masks" state={aiTask.state} onRun={() => {}} onCancel={() => void aiTask.cancel()} />
        ) : (
          <Menu position="bottom-end" withinPortal>
            <Menu.Target>
              <Button size="compact-sm" variant="default" leftSection={<IconPlus size={14} />} disabled={!session.loaded}>
                New mask
              </Button>
            </Menu.Target>
            <Menu.Dropdown className="develop-masks__menu">
              <ShapeEntries hand={create} ai={aiMenu(createAi)} />
            </Menu.Dropdown>
          </Menu>
        )}
      </div>
      {masks.length > 0 && (
        <Reorder.Group
          as="div"
          axis="y"
          values={dragging ?? [...masks].reverse()}
          onReorder={setDragging}
          className="develop-masks__list"
          role="listbox"
          aria-label="Masks list"
        >
          {(dragging ?? [...masks].reverse()).map((m) => (
            // biome-ignore lint/a11y/useFocusableInteractiveElement: the row is the option; its buttons are separate stops
            <Reorder.Item
              as="div"
              key={m.id}
              value={m}
              role="option"
              aria-selected={m.id === ui.selected}
              tabIndex={0}
              className="develop-masks__row"
              onDragEnd={drop}
              onClick={() => select(m.id)}
              onContextMenu={(e) => openMenu(e, menuOf(m))}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  select(m.id);
                } else if (matches(e.nativeEvent, "develop.mask-forward") || matches(e.nativeEvent, "develop.mask-backward")) {
                  e.preventDefault();
                  const at = masks.findIndex((x) => x.id === m.id);
                  move(m, matches(e.nativeEvent, "develop.mask-forward") ? Math.min(masks.length - 1, at + 1) : Math.max(0, at - 1));
                }
              }}
            >
              <Thumb mask={m} aspect={aspect} rasters={rasters} />
              <span className="develop-masks__name">{m.name}</span>
              <ActionIcon
                variant="subtle"
                size="sm"
                aria-label={m.visible ? `Hide ${m.name}` : `Show ${m.name}`}
                onClick={(e) => {
                  e.stopPropagation();
                  applyMasks((all) => updateMask(all, m.id, { visible: !m.visible }), m.visible ? "Hide mask" : "Show mask");
                }}
              >
                {m.visible ? <IconEye size={14} /> : <IconEyeOff size={14} />}
              </ActionIcon>
            </Reorder.Item>
          ))}
        </Reorder.Group>
      )}
    </section>
  );
}
