// What an import row's menu does (Round 4G): show the folder it came from (read-only), retry
// the files that failed, copy the errors, or dismiss the row from *Recent imports*.

import type { QueryClient } from "@tanstack/react-query";
import { revealPath } from "@/lib/dialogs";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { importRowMenu, type MenuItem } from "@/lib/menus";
import { queryKeys } from "@/lib/query-keys";
import { createStore } from "@/lib/store";
import type { ImportBatchSummary } from "@/types/generated/ImportBatchSummary";
import type { ImportFailure } from "@/types/generated/ImportFailure";

/** `photos_core::batches::SOURCE_FOLDER`: a folder or files picked on this computer. */
export const SOURCE_FOLDER = 0;
const KEY = "lumen.imports.dismissed";

const load = (): number[] => {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "[]");
    return Array.isArray(v) ? v.filter((n) => typeof n === "number") : [];
  } catch {
    return [];
  }
};

/**
 * Rows dismissed from *Recent imports*, remembered on this computer only: the batch stays in
 * the catalogue (its photos say which import brought them), it just leaves the list.
 */
export const dismissedImports = createStore<ReadonlySet<number>>(new Set(load()));

function setDismissed(id: number, on: boolean) {
  dismissedImports.set((s) => {
    const next = new Set(s);
    if (on) next.add(id);
    else next.delete(id);
    try {
      localStorage.setItem(KEY, JSON.stringify([...next]));
    } catch {
      /* private mode: dismissed for this session only */
    }
    return next;
  });
}

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** `path: reason`, one failure a line, for pasting into a message. */
export const errorText = (failures: ImportFailure[]) => failures.map((f) => `${f.path}: ${f.reason}`).join("\n");

export function importRowItems(batch: ImportBatchSummary, qc: QueryClient): MenuItem[] {
  const folder = batch.sourceKind === SOURCE_FOLDER;
  const failed = batch.failedCount > 0;
  return importRowMenu({
    showInFolder:
      folder && batch.sourcePath
        ? () => void revealPath(batch.sourcePath!).catch(() => toast({ message: "That folder is no longer there." }))
        : undefined,
    retryFailed:
      folder && failed
        ? async () => {
            try {
              await invoke<number>("retry_import_failures", { batchId: batch.id });
              await qc.invalidateQueries({ queryKey: queryKeys.batches });
            } catch (e) {
              fail(e, "Could not retry the import.");
            }
          }
        : undefined,
    copyErrors: failed
      ? async () => {
          try {
            const failures = await invoke<ImportFailure[]>("get_import_failures", { batchId: batch.id });
            await navigator.clipboard.writeText(errorText(failures));
            toast({ message: failures.length === 1 ? "Copied the error" : `Copied ${failures.length} errors` });
          } catch (e) {
            fail(e, "Could not copy the errors.");
          }
        }
      : undefined,
    dismiss: () => {
      setDismissed(batch.id, true);
      toast({ message: "Import dismissed", undo: () => setDismissed(batch.id, false) });
    },
  });
}
