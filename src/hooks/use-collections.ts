import { useQuery } from "@tanstack/react-query";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { Collection } from "@/types/generated/Collection";

/**
 * The built-in collections with counts and covers. Favorites always shows; the rest only
 * once the library holds something of that kind, so a new library isn't a wall of zeros.
 */
export function useCollections() {
  const q = useQuery({ queryKey: queryKeys.collections, queryFn: () => invoke<Collection[]>("list_collections") });
  const visible = (q.data ?? []).filter((c) => c.id === "favorites" || c.count > 0);
  return { collections: visible, isLoading: q.isLoading };
}
