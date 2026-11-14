import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { HomeSummary } from "@/types/generated/HomeSummary";

/**
 * Home's extras: the latest Develop drafts, files Lumen can't open, the biggest places. Under
 * the "stats" key, so a library change refreshes it with the stats line.
 */
export function useHomeSummary() {
  const q = useQuery({ queryKey: queryKeys.homeSummary, queryFn: () => invoke<HomeSummary>("home_summary") });
  return { summary: q.data ?? null, isLoading: q.isLoading };
}
