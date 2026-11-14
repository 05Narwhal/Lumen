import { IconPlus } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useContextMenu } from "@/components/context-menu";
import { toast } from "@/components/toast";
import { historyKey, type DevelopSession } from "@/hooks/use-develop-session";
import { sameRecipe } from "@/lib/develop";
import { invoke, isAppError } from "@/lib/ipc";
import type { HistoryStep } from "@/types/generated/HistoryStep";
import { InlineName, Section } from "./parts";

const failed = (e: unknown, what: string) => toast({ message: isAppError(e) ? e.message : what });

function useHistory(assetId: number) {
  return useQuery({ queryKey: historyKey(assetId), queryFn: () => invoke<HistoryStep[]>("list_history", { assetId }) });
}

/**
 * History (spec §1): every recorded change, newest first, the current one marked. Clicking a
 * step shows it; the next change continues from there and drops the steps after it.
 */
export function History({ session, assetId }: { session: DevelopSession; assetId: number }) {
  const qc = useQueryClient();
  const steps = (useHistory(assetId).data ?? []).filter((s) => s.snapshotName === null);
  const newestFirst = [...steps].reverse();
  const current = newestFirst.find((s) => sameRecipe(s.recipe, session.draft))?.id ?? null;
  const clear = async () => {
    const kept = steps;
    try {
      await invoke("clear_history", { assetId });
      await qc.invalidateQueries({ queryKey: historyKey(assetId) });
      toast({
        message: "Cleared history",
        undo: () =>
          void (async () => {
            for (const s of kept) await invoke("push_history", { assetId, recipe: s.recipe, label: s.label });
            await qc.invalidateQueries({ queryKey: historyKey(assetId) });
          })(),
      });
    } catch (e) {
      failed(e, "The history could not be cleared.");
    }
  };
  return (
    <Section
      title="History"
      action={
        <button type="button" className="develop-sidebar__text-btn" disabled={steps.length === 0} onClick={() => void clear()}>
          Clear
        </button>
      }
    >
      {newestFirst.length === 0 ? (
        <p className="develop-sidebar__empty">Changes you make appear here.</p>
      ) : (
        <ol className="develop-sidebar__list" aria-label="History">
          {newestFirst.map((s) => (
            <li key={s.id}>
              <button type="button" className="develop-sidebar__row" aria-current={s.id === current ? "step" : undefined} onClick={() => session.goTo(s.recipe, s.id)}>
                {s.label}
              </button>
            </li>
          ))}
        </ol>
      )}
    </Section>
  );
}

/** Snapshots (spec §1): named states of the photo, kept however long the history grows. */
export function Snapshots({ session, assetId }: { session: DevelopSession; assetId: number }) {
  const qc = useQueryClient();
  const { open: openMenu } = useContextMenu();
  const snaps = (useHistory(assetId).data ?? []).filter((s) => s.snapshotName !== null);
  const [naming, setNaming] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);
  const refresh = () => qc.invalidateQueries({ queryKey: historyKey(assetId) });
  const create = async (name: string | null) => {
    setNaming(false);
    if (!name) return;
    try {
      await invoke("save_snapshot", { assetId, name, recipe: session.draft });
      await refresh();
    } catch (e) {
      failed(e, "The snapshot could not be saved.");
    }
  };
  const rename = async (s: HistoryStep, name: string | null) => {
    setRenaming(null);
    if (!name || name === s.snapshotName) return;
    try {
      await invoke("rename_snapshot", { id: s.id, name });
      await refresh();
    } catch (e) {
      failed(e, "The snapshot could not be renamed.");
    }
  };
  const remove = async (s: HistoryStep) => {
    try {
      await invoke("delete_snapshot", { id: s.id });
      await refresh();
      toast({ message: `Deleted snapshot “${s.snapshotName}”`, undo: () => void invoke("save_snapshot", { assetId, name: s.snapshotName, recipe: s.recipe }).then(refresh) });
    } catch (e) {
      failed(e, "The snapshot could not be deleted.");
    }
  };
  return (
    <Section
      title="Snapshots"
      action={
        <button type="button" className="develop-sidebar__icon" aria-label="New snapshot" title="Keep the current settings as a snapshot" onClick={() => setNaming(true)}>
          <IconPlus size={14} />
        </button>
      }
    >
      {naming && <InlineName value={`Snapshot ${snaps.length + 1}`} label="Snapshot name" onDone={(n) => void create(n)} />}
      {snaps.length === 0 && !naming ? (
        <p className="develop-sidebar__empty">Keep a version to come back to.</p>
      ) : (
        <ul className="develop-sidebar__list" aria-label="Snapshots">
          {snaps.map((s) => (
            <li key={s.id}>
              {renaming === s.id ? (
                <InlineName value={s.snapshotName ?? ""} label="Snapshot name" onDone={(n) => void rename(s, n)} />
              ) : (
                <button
                  type="button"
                  className="develop-sidebar__row"
                  aria-current={sameRecipe(s.recipe, session.draft) ? "true" : undefined}
                  onClick={() => session.goTo(s.recipe, null)}
                  onContextMenu={(e) =>
                    openMenu(e, [
                      { label: "Rename", onSelect: () => setRenaming(s.id) },
                      { label: "Delete", danger: true, onSelect: () => void remove(s) },
                    ])
                  }
                >
                  {s.snapshotName}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}
