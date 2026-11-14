import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { confirm } from "@/components/confirm-dialog";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { clearSelection } from "./use-selection";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

export const TRASH_RETENTION_DAYS = 30;

export function daysLeft(trashedAt: number, now = Date.now() / 1000): number {
  return Math.max(0, Math.ceil((trashedAt + TRASH_RETENTION_DAYS * 86_400 - now) / 86_400));
}

export function useTrash() {
  const qc = useQueryClient();
  const done = useCallback(() => {
    clearSelection();
    invalidateLibraryData(qc);
  }, [qc]);

  const restore = useCallback(
    async (ids: number[]) => {
      try {
        await invoke("restore_assets", { ids });
        done();
        toast({ message: ids.length === 1 ? "Restored" : `Restored ${ids.length} items` });
      } catch (e) {
        fail(e, "Could not restore.");
      }
    },
    [done],
  );

  /** Irreversible: confirms first (UX spec §8.2). */
  const deleteNow = useCallback(
    async (ids: number[]) => {
      const ok = await confirm({
        title: ids.length === 1 ? "Delete this item permanently?" : `Delete ${ids.length} items permanently?`,
        body: "The original files and all thumbnails are removed from the library folder. This cannot be undone.",
        confirmLabel: "Delete now",
        danger: true,
      });
      if (!ok) return;
      try {
        const n = await invoke<number>("delete_now", { ids });
        done();
        toast({ message: n === 1 ? "Deleted permanently" : `Deleted ${n} items permanently` });
      } catch (e) {
        fail(e, "Could not delete.");
      }
    },
    [done],
  );

  const emptyTrash = useCallback(
    async (count: number) => {
      const ok = await confirm({
        title: "Empty the trash?",
        body: `${count} item${count === 1 ? "" : "s"} will be deleted permanently, including the original files. This cannot be undone.`,
        confirmLabel: "Empty trash",
        danger: true,
      });
      if (!ok) return;
      try {
        const n = await invoke<number>("empty_trash");
        done();
        toast({ message: `Deleted ${n} item${n === 1 ? "" : "s"} permanently` });
      } catch (e) {
        fail(e, "Could not empty the trash.");
      }
    },
    [done],
  );

  return { restore, deleteNow, emptyTrash };
}
