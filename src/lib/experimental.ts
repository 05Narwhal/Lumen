import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";

/** Every AI task's models, tiers and timings on this PC (Round 14, Settings → Experimental). */
export const experimentalCatalogQuery = {
  queryKey: queryKeys.experimentalCatalog,
  queryFn: () => invoke<ExperimentalTask[]>("experimental_catalog"),
};

/** Whether a Lumen Prism model is installed, so Retouch can run (Round 14). Unknown reads as no. */
export function prismInstalled(catalog: Pick<ExperimentalTask, "task" | "models">[] | undefined): boolean {
  return (catalog?.find((t) => t.task === "retouch")?.models.length ?? 0) > 0;
}
