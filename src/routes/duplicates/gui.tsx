import { Button } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { IconCopy } from "@tabler/icons-react";
import { useState } from "react";
import { Link } from "react-router";
import { DuplicateSetRow } from "@/components/duplicate-set";
import { EmptyState } from "@/components/empty-state";
import { PageHeader } from "@/components/page-header";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { DuplicateResolution } from "@/types/generated/DuplicateResolution";
import type { DuplicateSet } from "@/types/generated/DuplicateSet";
import type { ResolveUndo } from "@/types/generated/ResolveUndo";
import "./styles.scss";

const photos = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);

/** Home's card (Round 4D): shown only when there is something to review. */
export function DuplicatesCard() {
  const count = useQuery({ queryKey: queryKeys.duplicateCount, queryFn: () => invoke<[number, number]>("count_duplicate_sets") });
  const [sets, removable] = count.data ?? [0, 0];
  if (sets === 0) return null;
  return (
    <section className="duplicates-card" aria-label="Duplicates">
      <IconCopy size={22} className="duplicates-card__icon" aria-hidden />
      <div className="duplicates-card__text">
        <strong>{sets === 1 ? "1 set of duplicates" : `${sets.toLocaleString()} sets of duplicates`}</strong>
        <span>{removable === 1 ? "1 photo could go" : `${removable.toLocaleString()} photos could go`}</span>
      </div>
      <Button component={Link} to="/duplicates" size="xs" variant="light">
        Review
      </Button>
    </section>
  );
}

/**
 * `/duplicates` (Round 4D): exact copies and near-identical shots in one list, newest first.
 * Each set suggests a photo to keep; removing moves the others to the Trash after the keeper
 * takes on their albums, marks and keywords, and every change can be undone.
 */
export function DuplicatesRoute() {
  const qc = useQueryClient();
  const list = useQuery({ queryKey: queryKeys.duplicates, queryFn: () => invoke<DuplicateSet[]>("list_duplicate_sets") });
  // The user's choice of keeper per set, by the set's key; the suggestion otherwise.
  const [keepers, setKeepers] = useState<Record<string, number>>({});
  const sets = list.data ?? [];
  const keepOf = (s: DuplicateSet) => keepers[s.key] ?? s.keep;
  const resolutionOf = (s: DuplicateSet): DuplicateResolution => {
    const keep = keepOf(s);
    return { keep, remove: s.members.map((m) => m.id).filter((id) => id !== keep).sort((a, b) => a - b) };
  };
  const removable = sets.reduce((n, s) => n + s.members.length - 1, 0);
  // Exact copies can be removed on their own, leaving the similar shots to decide.
  const exactSets = sets.filter((s) => s.likeness === "exact");
  const exactRemovable = exactSets.reduce((n, s) => n + s.members.length - 1, 0);

  const refresh = () => {
    void invalidateLibraryData(qc);
    void qc.invalidateQueries({ queryKey: queryKeys.duplicates });
  };
  const failed = (e: unknown) => toast({ message: isAppError(e) ? e.message : "The duplicates could not be changed." });

  const remove = async (chosen: DuplicateSet[]) => {
    const resolutions = chosen.map(resolutionOf);
    const n = resolutions.reduce((k, r) => k + r.remove.length, 0);
    try {
      const undos = await invoke<ResolveUndo[]>("resolve_duplicates", { sets: resolutions });
      refresh();
      toast({
        message: `Moved ${photos(n)} to the Trash`,
        undo: () => void invoke("undo_resolve_duplicates", { undos }).then(refresh, failed),
      });
    } catch (e) {
      failed(e);
    }
  };

  const dismiss = async (s: DuplicateSet) => {
    const ids = s.members.map((m) => m.id);
    try {
      await invoke("dismiss_duplicates", { ids });
      void qc.invalidateQueries({ queryKey: queryKeys.duplicates });
      toast({
        message: "Marked as not duplicates",
        undo: () => void invoke("undo_dismiss_duplicates", { ids }).then(() => qc.invalidateQueries({ queryKey: queryKeys.duplicates }), failed),
      });
    } catch (e) {
      failed(e);
    }
  };

  return (
    <div className="duplicates-route">
      <PageHeader
        title="Duplicates"
        back={{ to: "/", label: "Home" }}
        search={false}
        count={sets.length > 0 ? `${sets.length.toLocaleString()} ${sets.length === 1 ? "set" : "sets"} · ${photos(removable)} could go` : undefined}
        toolbar={
          sets.length > 0 && (
            <>
              {exactSets.length > 0 && exactSets.length < sets.length && (
                <Button size="sm" variant="default" onClick={() => void remove(exactSets)}>
                  Remove exact copies ({exactRemovable.toLocaleString()})
                </Button>
              )}
              <Button size="sm" onClick={() => void remove(sets)}>
                Remove all suggested ({removable.toLocaleString()})
              </Button>
            </>
          )
        }
      />
      {list.isSuccess && sets.length === 0 ? (
        <EmptyState
          icon={<IconCopy size={40} />}
          title="No duplicates found."
          description="Lumen looks again after every import."
          action={
            <Button component={Link} to="/">
              Back to Home
            </Button>
          }
        />
      ) : (
        <div className="duplicates-route__list">
          {sets.map((s) => (
            <DuplicateSetRow
              key={s.key}
              set={s}
              keep={keepOf(s)}
              onKeep={(id) => setKeepers((k) => ({ ...k, [s.key]: id }))}
              onRemove={() => void remove([s])}
              onDismiss={() => void dismiss(s)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
