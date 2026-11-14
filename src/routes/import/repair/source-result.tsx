import { Button, Checkbox } from "@mantine/core";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { repairImportedMessage } from "@/lib/repair-progress";
import { canTick, isTicked, NOTHING, selectAll, tickedCount, toggle, type RepairSelection } from "@/lib/repair-selection";
import type { RepairClass } from "@/types/generated/RepairClass";
import type { RepairItemsPage } from "@/types/generated/RepairItemsPage";
import type { RepairResult } from "@/types/generated/RepairResult";
import type { RepairStatus } from "@/types/generated/RepairStatus";
import { formatBytes } from "@/utils/format";

const PAGE = 100;
const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });
const count = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

/** The filters, in the order the result reads them. */
const FILTERS: { status: RepairClass; label: string; key: "new" | "damaged" | "sourceDamaged" | "sidecar" | "inTrash" }[] = [
  { status: "new", label: "Not in the library", key: "new" },
  { status: "damaged", label: "Can be repaired", key: "damaged" },
  { status: "sourceDamaged", label: "Damaged on source too", key: "sourceDamaged" },
  { status: "sidecar", label: "Sidecar missing", key: "sidecar" },
  { status: "inTrash", label: "In Trash", key: "inTrash" },
];

const classWord = (status: RepairClass, name: string): string =>
  ({
    safe: "Safe",
    new: "Not in the library",
    damaged: "Can be repaired",
    sourceDamaged: `Damaged on ${name} too`,
    sidecar: "Sidecar missing",
    inTrash: "In Trash",
  })[status];

/**
 * What a checked source found (Round 22 B4): a crumb back to the library's health, the chips, the
 * rows and *Re-import selected*. Round 21's selection model is kept. A copy that is damaged on the
 * source too cannot be ticked: it can be recovered from the library's own file, or looked for
 * somewhere else.
 */
export function SourceResult({
  status,
  name,
  onBack,
  restoring,
}: {
  status: RepairStatus;
  name: string;
  onBack: () => void;
  restoring: boolean;
}) {
  const qc = useQueryClient();
  const c = status.counts;
  const total = c.new + c.damaged + c.sourceDamaged + c.sidecar + c.inTrash;
  // *Restore…* looks only at the damaged copies and has every one of them ticked to begin with.
  const [picked, setPicked] = useState<RepairClass | null>(restoring ? "damaged" : null);
  const filter = picked ?? FILTERS.find((f) => c[f.key] > 0)?.status ?? "new";
  const [selection, setSelection] = useState<RepairSelection>(restoring ? selectAll("damaged") : NOTHING);
  const [busy, setBusy] = useState(false);
  const inFilter = FILTERS.find((f) => f.status === filter)!;
  const n = tickedCount(selection, c[inFilter.key]);

  const items = useInfiniteQuery({
    queryKey: queryKeys.repairItems(filter),
    initialPageParam: null as RepairItemsPage["nextCursor"],
    queryFn: ({ pageParam }) => invoke<RepairItemsPage>("get_repair_items", { status: filter, after: pageParam, limit: PAGE }),
    getNextPageParam: (last) => last.nextCursor,
    enabled: c[inFilter.key] > 0,
  });
  const rows = items.data?.pages.flatMap((p) => p.items) ?? [];
  // Set by `repair://imported`; the check that follows keeps it on screen.
  const imported = useQuery<RepairResult | null>({ queryKey: queryKeys.repairImported, queryFn: () => null, enabled: false, initialData: null }).data;

  const pick = (f: RepairClass) => {
    setPicked(f);
    setSelection(NOTHING);
  };
  const reimport = async () => {
    setBusy(true);
    try {
      await invoke("start_repair_import", { selection });
      setSelection(NOTHING);
      void qc.invalidateQueries({ queryKey: queryKeys.repairStatus });
    } catch (e) {
      fail(e, "Could not start the re-import.");
    } finally {
      setBusy(false);
    }
  };
  const recover = (assetId: number) =>
    void invoke("start_recover", { ids: [assetId] })
      .then(() => {
        onBack();
        return qc.invalidateQueries({ queryKey: queryKeys.repairStatus });
      })
      .catch((e: unknown) => fail(e, "Could not start the recovery."));

  return (
    <section className="repair-result" aria-label="Result">
      <button type="button" className="repair-result__crumb" onClick={onBack}>
        ← Library health
      </button>
      <div className="repair-result__verdict">
        <strong>
          {status.cancelled
            ? "Stopped before the end. This is not a result."
            : total === 0
              ? `Everything on ${name} is safely in the library.`
              : `${count(total, "file needs", "files need")} a look on ${name}.`}
        </strong>
        {c.safe > 0 && total > 0 && <span>{count(c.safe, "file is", "files are")} safely in the library.</span>}
      </div>
      {imported && (
        <div className="repair-result__imported" role="status">
          <span>{repairImportedMessage(imported)}</span>
          {imported.notes.length > 0 && (
            <ul>
              {imported.notes.map((note) => (
                <li key={note}>{note}</li>
              ))}
            </ul>
          )}
        </div>
      )}
      {total > 0 && (
        <>
          <div className="repair-list__chips" role="group" aria-label="Show">
            {FILTERS.map((f) => (
              <button key={f.status} type="button" className="repair-chip" aria-pressed={f.status === filter} disabled={c[f.key] === 0} onClick={() => pick(f.status)}>
                {f.label} ({c[f.key].toLocaleString()})
              </button>
            ))}
          </div>
          <div className="repair-result__actions">
            {canTick(filter) ? (
              <Checkbox
                size="xs"
                label="Select all"
                checked={"all" in selection && selection.except.length === 0}
                indeterminate={n > 0 && !("all" in selection && selection.except.length === 0)}
                onChange={(e) => setSelection(e.currentTarget.checked ? selectAll(filter) : NOTHING)}
              />
            ) : (
              <span className="repair-result__hint">{filter === "inTrash" ? "Put it back from Trash" : "Not on the source either"}</span>
            )}
            <Button size="xs" disabled={n === 0} loading={busy} onClick={() => void reimport()}>
              Re-import selected ({n.toLocaleString()})
            </Button>
          </div>
          <ul className="repair-rows" aria-label="Files">
            {rows.map((r) => (
              <li key={r.id} className="repair-row repair-row--source" data-status={r.status}>
                <Checkbox
                  size="xs"
                  aria-label={`Select ${r.name}`}
                  disabled={!canTick(r.status)}
                  checked={canTick(r.status) && isTicked(selection, r.id)}
                  onChange={() => setSelection((s) => toggle(s, r.id))}
                />
                <span className="repair-row__main">
                  <strong>{r.name}</strong>
                  <small>{r.dir || "/"}</small>
                </span>
                <span className="repair-row__size">{formatBytes(r.byteSize)}</span>
                <span className="repair-row__cause">{classWord(r.status, name)}</span>
                {r.status === "sourceDamaged" && r.assetId != null ? (
                  <span className="repair-row__acts">
                    <Button size="compact-xs" variant="default" onClick={() => recover(r.assetId!)}>
                      Recover
                    </Button>
                    <Button size="compact-xs" variant="default" onClick={onBack}>
                      Look somewhere else
                    </Button>
                  </span>
                ) : (
                  <span />
                )}
              </li>
            ))}
          </ul>
          {items.hasNextPage && (
            <Button size="xs" variant="default" loading={items.isFetchingNextPage} onClick={() => void items.fetchNextPage()}>
              Show more
            </Button>
          )}
        </>
      )}
    </section>
  );
}
