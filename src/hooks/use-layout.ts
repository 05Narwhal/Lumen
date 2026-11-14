import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { useMemo, useRef } from "react";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import type { LayoutSummary } from "@/types/generated/LayoutSummary";
import type { GroupBy } from "@/types/generated/GroupBy";
import type { LayoutScope } from "@/types/generated/LayoutScope";

export const LIBRARY_SCOPE: LayoutScope = { mediaType: null, deviceId: null, albumId: null, favorites: false, trash: false, sortByDate: false, query: null, memoryId: null, personId: null, petId: null, vault: false, bestMatch: false, collectionId: null };

export const ROW_CHUNK = 40;

export interface LayoutParams {
  width: number;
  rowHeight: number;
  gap: number;
  scope: LayoutScope;
  /** How a time-ordered scope is cut into sections. Ignored for manual order. */
  groupBy?: GroupBy;
}

export function useLayout({ width, rowHeight, gap, scope, groupBy = "month" }: LayoutParams) {
  const q = useQuery({
    queryKey: queryKeys.layout(width, rowHeight, gap, scope, groupBy),
    queryFn: () => invoke<LayoutSummary>("get_layout", { width, rowHeight, gap, scope, groupBy }),
    enabled: width > 0,
    placeholderData: keepPreviousData,
    staleTime: Infinity,
  });
  const fingerprint = useMemo(() => layoutFingerprint(q.data), [q.data]);
  // `isPlaceholder`: the summary on show is the previous layout's, kept while this one loads.
  return { summary: q.data, fingerprint, isLoading: q.isLoading, isPlaceholder: q.isPlaceholderData, error: q.error };
}

/**
 * Identifies one arrangement of rows. Row chunks are cached under it, so rows cut for
 * another arrangement (before photos were removed, say) are never drawn at this one's
 * offsets: they overlapped their neighbours, and a click or a shift-click range landed on
 * photos that were not where they appeared.
 */
export function layoutFingerprint(summary: LayoutSummary | undefined): string {
  if (!summary) return "";
  let h = 2166136261;
  for (const v of summary.rowHeights) h = Math.imul(h ^ v, 16777619);
  return `${summary.itemCount}:${summary.totalHeight}:${summary.rowHeights.length}:${(h >>> 0).toString(36)}`;
}

/** Rows covering `[first, last]`, fetched in fixed chunks so scrolling reuses the cache. */
export function useLayoutRows({ width, rowHeight, gap, scope, groupBy = "month" }: LayoutParams, range: [number, number] | null, fingerprint = "") {
  const chunks = useMemo(() => {
    if (!range || width <= 0 || !fingerprint) return [] as number[];
    const [first, last] = range;
    const a = Math.floor(Math.max(0, first) / ROW_CHUNK);
    const b = Math.floor(Math.max(0, last) / ROW_CHUNK);
    return Array.from({ length: b - a + 1 }, (_, i) => a + i);
  }, [range, width, fingerprint]);

  const results = useQueries({
    queries: chunks.map((chunk) => ({
      queryKey: queryKeys.layoutRows(width, rowHeight, gap, scope, chunk, groupBy, fingerprint),
      queryFn: () =>
        invoke<LayoutRow[]>("get_layout_rows", {
          width,
          rowHeight,
          gap,
          scope,
          groupBy,
          start: chunk * ROW_CHUNK,
          count: ROW_CHUNK,
        }),
      // No placeholder rows: rows laid out for another width must never be drawn
      // under this width's row heights (they overlapped when the sidebar collapsed).
      staleTime: Infinity,
    })),
  });

  // `useQueries` returns a new array every render. Rebuilding the map from it gave every
  // render a new map, which re-rendered each of the grid's tiles on every scroll frame — a
  // thousand of them in the Years view. The map changes only when a chunk's data does.
  const datas = results.map((r) => r.data);
  const last = useRef<{ datas: (LayoutRow[] | undefined)[]; map: Map<number, LayoutRow> } | null>(null);
  const prev = last.current;
  if (prev && prev.datas.length === datas.length && prev.datas.every((d, i) => d === datas[i])) return prev.map;
  const map = new Map<number, LayoutRow>();
  for (const d of datas) for (const row of d ?? []) map.set(row.index, row);
  last.current = { datas, map };
  return map;
}
