import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import type { Neighbors } from "@/types/generated/Neighbors";
import type { SlotNeighbors } from "@/types/generated/SlotNeighbors";
import type { ViewerScope } from "@/lib/viewer-scope";

export function useAsset(id: number | null) {
  return useQuery({
    queryKey: queryKeys.asset(id ?? -1),
    queryFn: () => invoke<AssetDetail>("get_asset", { id }),
    enabled: id !== null,
    staleTime: 60_000,
  });
}

/** Previous and next inside the grid that opened the viewer; `null` data when the photo is
 *  not in it. Disabled (no request) when no grid scope is given. */
export function useScopeNeighbors(scope: ViewerScope | null, id: number | null) {
  return useQuery({
    queryKey: queryKeys.scopeNeighbors(scope, id ?? -1),
    queryFn: () => invoke<Neighbors | null>("get_scope_neighbors", { ...scope, id }),
    enabled: id !== null && scope !== null,
    staleTime: 60_000,
  });
}

/** [`useScopeNeighbors`] for a collection's photos by album (Round 19 §10): a photo can be in
 *  two sections, so the answer names the section of each neighbour and `section` says which
 *  copy is open (`null`: the first). */
export function useSectionNeighbors(scope: ViewerScope | null, id: number | null, section: number | null) {
  return useQuery({
    queryKey: queryKeys.sectionNeighbors(scope, id ?? -1, section),
    queryFn: () => invoke<SlotNeighbors | null>("get_section_neighbors", { ...scope, id, section }),
    enabled: id !== null && scope !== null,
    staleTime: 60_000,
  });
}

export function useNeighbors(id: number | null) {
  return useQuery({
    queryKey: queryKeys.neighbors(id ?? -1),
    queryFn: () => invoke<Neighbors>("get_asset_neighbors", { id }),
    enabled: id !== null,
    staleTime: 60_000,
  });
}
