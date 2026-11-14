import { useQueries, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AiMaskInfo } from "@/types/generated/AiMaskInfo";
import type { AiMaskKind } from "@/types/generated/AiMaskKind";
import type { AiMaskList } from "@/types/generated/AiMaskList";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";
import type { Mask } from "@/types/generated/Mask";
import type { Settings } from "@/types/generated/Settings";
import type { MaskShape } from "@/types/generated/MaskShape";
import { mediaUrl } from "@/utils/media";
import { ZERO_ADJUST } from "./develop";
import type { AiRaster } from "./mask-raster";

/** Phase 8f: masks Lumen Facet finds (Sky, Subject, Background, Person n), as plain functions. */

export type AiShape = Extract<MaskShape, { kind: "ai" }>;

export const AI_NAMES: Record<AiMaskKind, string> = { sky: "Sky", subject: "Subject", background: "Background", person: "Person" };

/** "Sky", "Person 2": a person's instance counts from one. */
export const aiLabel = (m: Pick<AiMaskInfo, "kind" | "instance">) => (m.kind === "person" ? `Person ${m.instance + 1}` : AI_NAMES[m.kind]);

export const aiShape = (m: AiMaskInfo, add: boolean): AiShape => ({ kind: "ai", add, target: m.kind, instance: m.instance, raster: m.hash });

/** A new mask after the highest id holding one AI shape, named after what it selects. */
export function newAiMask(masks: Mask[], m: AiMaskInfo): Mask {
  const id = masks.reduce((n, x) => Math.max(n, x.id), 0) + 1;
  return { id, name: aiLabel(m), shapes: [aiShape(m, true)], invert: false, opacity: 1, adjust: ZERO_ADJUST, visible: true };
}

export const addAiShape = (masks: Mask[], id: number, m: AiMaskInfo, add: boolean): Mask[] =>
  masks.map((x) => (x.id === id ? { ...x, shapes: [...x.shapes, aiShape(m, add)] } : x));

/**
 * The mask with each AI shape pointed at `list`'s raster of the same target and instance
 * (*Detect again*, or a mask pasted from another photo). A shape with no such raster here is
 * left as it was; nothing changed returns `mask` itself.
 */
export function retarget(mask: Mask, list: AiMaskList): Mask {
  let changed = false;
  const shapes = mask.shapes.map((s) => {
    if (s.kind !== "ai") return s;
    const hit = list.masks.find((m) => m.kind === s.target && m.instance === s.instance);
    if (!hit || hit.hash === s.raster) return s;
    changed = true;
    return { ...s, raster: hit.hash };
  });
  return changed ? { ...mask, shapes } : mask;
}

/** A raster's bytes as fetched: its weights, `"missing"` when this photo has none, absent while loading. */
export type RasterState = AiRaster | "missing";

/** Indexes of `mask`'s AI shapes whose raster this photo doesn't have (*Not detected on this photo*). */
export const missingRasters = (mask: Mask, rasters: Readonly<Record<string, RasterState | undefined>>): number[] =>
  mask.shapes.flatMap((s, i) => (s.kind === "ai" && rasters[s.raster] === "missing" ? [i] : []));

/** `photos://ai-mask/` bytes: width and height, u32 little-endian, then the weights. Null if damaged. */
export function parseRaster(buf: ArrayBuffer): AiRaster | null {
  if (buf.byteLength < 8) return null;
  const head = new DataView(buf, 0, 8);
  const width = head.getUint32(0, true);
  const height = head.getUint32(4, true);
  const weights = new Uint8Array(buf, 8);
  return weights.length === width * height ? { width, height, weights } : null;
}

/** Facet's switch is on and its model is ready (the Masks row names it): the palette offers AI masks. */
export function aiMasksReady(settings: Settings | undefined, catalog: Pick<ExperimentalTask, "task" | "builtin">[] | undefined): boolean {
  return (settings?.smart?.enabled ?? true) && !!settings?.smart?.experimental?.aiMasks &&!!catalog?.some((r) => r.task === "masks" && r.builtin);
}

export const aiMasksQuery = (assetId: number) => ({
  queryKey: queryKeys.aiMasks(assetId),
  queryFn: () => invoke<AiMaskList>("ai_masks_list", { assetId }),
});

/** What the Masks panel offers on this photo; hidden (`available: false`) while loading. */
export function useAiMasks(assetId: number): AiMaskList {
  const q = useQuery(aiMasksQuery(assetId));
  return q.data ?? HIDDEN;
}

const HIDDEN: AiMaskList = { available: false, scanned: false, masks: [] };

async function fetchRaster(assetId: number, hash: string): Promise<RasterState> {
  const res = await fetch(mediaUrl("ai-mask", `${assetId}/${hash}`));
  if (res.status === 404) return "missing";
  if (!res.ok) throw new Error(`ai-mask ${res.status}`);
  return parseRaster(await res.arrayBuffer()) ?? "missing";
}

/**
 * The rasters `masks`' AI shapes name, fetched over `photos://ai-mask/` (never `invoke`) and
 * kept by hash; they never change, so each is fetched once.
 */
export function useAiRasters(assetId: number, masks: readonly Mask[]): Readonly<Record<string, RasterState | undefined>> {
  const list = masks.flatMap((m) => m.shapes.flatMap((s) => (s.kind === "ai" ? [s.raster] : [])));
  const key = [...new Set(list)].sort().join(",");
  const hashes = useMemo(() => (key ? key.split(",") : []), [key]);
  // A stable `combine` lets the query library hand back the same object until a raster lands,
  // so the tint and the thumbnails redraw only then.
  const combine = useCallback(
    (results: { data?: RasterState }[]) => {
      const out: Record<string, RasterState | undefined> = {};
      results.forEach((r, i) => {
        out[hashes[i]] = r.data;
      });
      return out;
    },
    [hashes],
  );
  return useQueries({
    queries: hashes.map((hash) => ({ queryKey: queryKeys.aiRaster(assetId, hash), queryFn: () => fetchRaster(assetId, hash), staleTime: Number.POSITIVE_INFINITY, retry: false })),
    combine,
  });
}

/** `useAiRasters` as `rasterize` takes them, kept while nothing new has landed. */
export function useLoadedRasters(assetId: number, masks: readonly Mask[]) {
  const fetched = useAiRasters(assetId, masks);
  return { fetched, rasters: useMemo(() => loadedRasters(fetched), [fetched]) };
}

/** The fetched rasters in the shape `rasterize` takes: a missing one weighs nothing. */
export function loadedRasters(rasters: Readonly<Record<string, RasterState | undefined>>): Record<string, AiRaster> {
  const out: Record<string, AiRaster> = {};
  for (const [hash, r] of Object.entries(rasters)) if (r && r !== "missing") out[hash] = r;
  return out;
}
