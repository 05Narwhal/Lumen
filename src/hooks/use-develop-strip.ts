import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { isEmptySource, type DevelopSource } from "@/lib/develop-source";
import { invoke } from "@/lib/ipc";
import type { AssetQuery } from "@/types/generated/AssetQuery";
import type { StripItem } from "@/types/generated/StripItem";
import type { StripPage } from "@/types/generated/StripPage";

const NO_ITEMS: StripItem[] = [];

/** Photos fetched per page of the filmstrip. */
export const STRIP_PAGE = 200;

export const stripKey = (source: DevelopSource) => ["develop-strip", JSON.stringify(source)] as const;
export const stripIdsKey = (source: DevelopSource) => ["develop-strip-ids", JSON.stringify(source)] as const;

const args = (source: DevelopSource) => ({ scope: source.scope, only: source.only, sort: source.sort });

/** The filmstrip's photos, a page at a time by keyset. */
export function useDevelopStrip(source: DevelopSource) {
  const q = useInfiniteQuery({
    queryKey: stripKey(source),
    initialPageParam: null as StripPage["nextCursor"],
    queryFn: ({ pageParam }) => invoke<StripPage>("strip_page", { ...args(source), cursor: pageParam, limit: STRIP_PAGE }),
    getNextPageParam: (last) => last.nextCursor,
    enabled: !isEmptySource(source),
  });
  // The same array while the pages are unchanged: Develop keys effects on it (Round 4A).
  const items: StripItem[] = useMemo(() => q.data?.pages.flatMap((p) => p.items) ?? NO_ITEMS, [q.data]);
  return { items, hasMore: !!q.hasNextPage, loadMore: () => void (q.hasNextPage && !q.isFetchingNextPage && q.fetchNextPage()), loaded: q.isSuccess };
}

/** Every id in the strip, in order: for its count and the selection commands. */
export function useStripIds(source: DevelopSource) {
  const empty = isEmptySource(source);
  const q = useQuery({ queryKey: stripIdsKey(source), queryFn: () => invoke<number[]>("strip_ids", args(source)), enabled: !empty });
  // Nothing chosen is a settled, empty strip, not one still loading.
  return empty ? { ...q, data: [] as number[], isSuccess: true as const } : q;
}

/** The ids of the strip narrowed further by `extra` (picked, rejected, a label). */
export function stripIdsWhere(source: DevelopSource, extra: Partial<AssetQuery>) {
  const base = source.scope.query;
  const query = { ...(base ?? {}), ...extra } as AssetQuery;
  return invoke<number[]>("strip_ids", { ...args(source), scope: { ...source.scope, query } });
}
