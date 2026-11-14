import { useQuery } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { CatalogRef } from "@/types/generated/CatalogRef";
import type { LightroomImportStart } from "@/types/generated/LightroomImportStart";

/**
 * Lightroom Classic catalogs Lumen found by itself (Round 11): in Pictures, home, the top of
 * each drive and Lightroom's recent list. Refetched when drives come and go (`events.ts`),
 * so the Import screen offers Lightroom only while there is one.
 */
export function useLightroomCatalogs() {
  return useQuery({
    queryKey: queryKeys.lightroomCatalogs,
    queryFn: () => invoke<CatalogRef[]>("find_lightroom_catalogs"),
    staleTime: 60_000,
  });
}

const count = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/** "Importing 1,200 photos from Lightroom Catalog · 2 originals not found". */
export function catalogStartMessage(s: LightroomImportStart["summary"]): string {
  const parts = [`Importing ${count(s.items, "photo")} from ${s.name}`];
  if (s.missing.length > 0) parts.push(`${count(s.missing.length, "original")} not found`);
  return parts.join(" · ");
}

/** Starts a catalog's import (the normal import job) and says what it leaves behind. */
export async function startLightroomImport(path: string): Promise<void> {
  const r = await invoke<LightroomImportStart>("start_lightroom_import", { path });
  const missing = r.summary.missing;
  toast({
    message: catalogStartMessage(r.summary),
    action: missing.length > 0 ? { label: "Copy list", run: () => void navigator.clipboard?.writeText(missing.join("\n")) } : undefined,
  });
}
