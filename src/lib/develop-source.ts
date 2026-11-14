import type { NavigateFunction } from "react-router";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import { EMPTY_QUERY } from "@/hooks/use-search";
import { createStore } from "@/lib/store";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { LayoutScope } from "@/types/generated/LayoutScope";
import type { StripSort } from "@/types/generated/StripSort";

/**
 * What Develop's filmstrip shows (spec §1): a scope (the library, an album, a search), or
 * exactly the photos it was opened on — held here, not in the URL, because a selection can
 * be thousands of ids. The top bar's mark filters and sort live here too, so they survive
 * moving from photo to photo.
 */
export interface DevelopSource {
  scope: LayoutScope;
  /** The photos Develop was opened on, in order; null for the whole scope. */
  only: number[] | null;
  /** Named in the top bar: "Library", "Selection", an album's name. */
  label: string;
  sort: StripSort;
}

/** Develop starts empty: the photos to work on are chosen, never the whole library. */
export const DEFAULT_DEVELOP_SOURCE: DevelopSource = { scope: LIBRARY_SCOPE, only: [], label: "Develop", sort: "captured" };

/** Nothing has been chosen to develop yet. */
export const isEmptySource = (s: DevelopSource) => s.only !== null && s.only.length === 0;

export const developSourceStore = createStore<DevelopSource>(DEFAULT_DEVELOP_SOURCE);

/**
 * Open Develop on `source`, at `assetId` or the strip's first photo. With no source, the
 * strip holds just `assetId`, or nothing (the user then chooses photos).
 */
export function openDevelop(navigate: NavigateFunction, source: Partial<Omit<DevelopSource, "sort">> = {}, assetId?: number) {
  const fallback = assetId !== undefined && source.only === undefined && source.scope === undefined ? { only: [assetId], label: "This photo" } : {};
  developSourceStore.set((s) => ({ ...DEFAULT_DEVELOP_SOURCE, sort: s.sort, ...fallback, ...source }));
  navigate(assetId ? `/develop/${assetId}` : "/develop");
}

/** Develop on exactly these photos, from the first. */
export function developPhotos(navigate: NavigateFunction, ids: number[]) {
  if (ids.length === 0) return;
  openDevelop(navigate, { only: ids, label: "Selection" }, ids[0]);
}

/** Add photos to the strip (keeping its order, without repeats). */
export function addToDevelop(ids: number[]) {
  developSourceStore.set((s) => {
    const have = s.only ?? [];
    const seen = new Set(have);
    return { ...s, only: [...have, ...ids.filter((i) => !seen.has(i))], label: s.only && s.only.length > 0 ? s.label : "Selection" };
  });
}

/** The photo last open in Develop: `/develop` goes back to it while the strip still holds it. */
export const lastDevelopPhoto = createStore<number | null>(null);

/** Empty the strip, back to *Choose photos* (the sort is kept). Returns what it held, for Undo. */
export function clearDevelop(): DevelopSource {
  const prev = developSourceStore.get();
  developSourceStore.set((s) => ({ ...DEFAULT_DEVELOP_SOURCE, sort: s.sort }));
  return prev;
}

/**
 * Take `ids` out of the strip, whose photos are `current` in order (a whole album becomes
 * an explicit list of the rest). Returns what it held, for Undo.
 */
export function removeFromDevelop(ids: number[], current: number[]): DevelopSource {
  const prev = developSourceStore.get();
  const gone = new Set(ids);
  developSourceStore.set((s) => ({ ...s, only: current.filter((i) => !gone.has(i)) }));
  return prev;
}

/** Put back what `clearDevelop` or `removeFromDevelop` returned. */
export function restoreDevelop(prev: DevelopSource) {
  developSourceStore.set(prev);
}

/** The source with the top bar's filters applied to its scope's query. */
export function withFilters(source: DevelopSource, patch: Partial<AssetQuery>): DevelopSource {
  const base = source.scope.query ?? EMPTY_QUERY;
  return { ...source, scope: { ...source.scope, query: { ...base, ...patch } } };
}

const STRIP_KEY = "lumen.develop.strip-hidden";

function readStripHidden(): boolean {
  try {
    return localStorage.getItem(STRIP_KEY) === "1";
  } catch {
    return false;
  }
}

/** Whether Develop's filmstrip is folded away, remembered per viewer. */
export const stripHiddenStore = createStore<boolean>(readStripHidden());

export function toggleDevelopStrip() {
  stripHiddenStore.set((h) => {
    try {
      localStorage.setItem(STRIP_KEY, h ? "0" : "1");
    } catch {
      // A private window: remembered for this visit only.
    }
    return !h;
  });
}
