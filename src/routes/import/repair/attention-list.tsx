import { Button, Checkbox } from "@mantine/core";
import { IconPhotoOff } from "@tabler/icons-react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Link } from "react-router";
import { toast } from "@/components/toast";
import { useImport } from "@/hooks/use-import";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { canRecover, CAUSE_WORD, causeSummary, outcomeWord, recoveredWord } from "@/lib/repair-page";
import type { LibraryHealth } from "@/types/generated/LibraryHealth";
import type { RecoveredChanges } from "@/types/generated/RecoveredChanges";
import type { RecoveredRow } from "@/types/generated/RecoveredRow";
import type { RepairOutcomeRow } from "@/types/generated/RepairOutcomeRow";
import type { UnreadableRowsPage } from "@/types/generated/UnreadableRowsPage";
import { formatRelative } from "@/utils/date";
import { formatBytes } from "@/utils/format";
import { mediaUrl } from "@/utils/media";

const PAGE = 100;
const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });
const plural = (n: number, one: string, many: string) => `${n.toLocaleString()} ${n === 1 ? one : many}`;

type Tab = "attention" | "recovered" | "lastRun" | "history";

/** A thumbnail that falls back to a placeholder: a damaged file has no derivative to show. */
function Thumb({ src }: { src: string | null }) {
  const [broken, setBroken] = useState(false);
  return (
    <span className="repair-row__thumb">
      {src && !broken ? <img src={src} alt="" loading="lazy" onError={() => setBroken(true)} /> : <IconPhotoOff size={18} aria-hidden />}
    </span>
  );
}

/**
 * The list card of the Repair page (Round 22 B4): the files that need a look, what a recovery
 * found, the last run, and the repair batches in the Import history.
 */
export function AttentionList({ health, sourceName, onRecoverStarted }: { health: LibraryHealth | undefined; sourceName: string | null; onRecoverStarted: () => void }) {
  const qc = useQueryClient();
  const recovered = useQuery({ queryKey: queryKeys.repairRecovered, queryFn: () => invoke<RecoveredRow[]>("get_recovered") }).data ?? [];
  const outcomes = useQuery({ queryKey: queryKeys.repairOutcomes, queryFn: () => invoke<RepairOutcomeRow[]>("get_repair_outcomes") }).data ?? [];
  const { batches } = useImport();
  const history = batches.filter((b) => b.sourceLabel.startsWith("Repair"));
  const unreadable = health?.unreadable ?? 0;
  const [picked, setPicked] = useState<Tab | null>(null);
  const tab: Tab = picked ?? (unreadable > 0 ? "attention" : recovered.length > 0 ? "recovered" : outcomes.length > 0 ? "lastRun" : "attention");

  if (health && unreadable === 0 && recovered.length === 0 && outcomes.length === 0 && history.length === 0) {
    return (
      <section className="repair-list repair-list--empty" aria-label="Files">
        <p>Every photo in this library opens fine.</p>
        <Button size="xs" onClick={() => void invoke("start_check_library").then(() => qc.invalidateQueries({ queryKey: queryKeys.check })).catch((e: unknown) => fail(e, "Could not start the check."))}>
          Check the library
        </Button>
      </section>
    );
  }

  const tabs: { id: Tab; label: string; n?: number }[] = [
    { id: "attention", label: "Needs attention", n: unreadable },
    { id: "recovered", label: "Recovered", n: recovered.length },
    { id: "lastRun", label: "Last run" },
    { id: "history", label: "History" },
  ];
  return (
    <section className="repair-list" aria-label="Files">
      <div className="repair-list__head">
        <div className="repair-list__chips" role="group" aria-label="Show">
          {tabs.map((t) => (
            <button key={t.id} type="button" className="repair-chip" aria-pressed={t.id === tab} onClick={() => setPicked(t.id)}>
              {t.label}
              {t.n !== undefined && ` ${t.n.toLocaleString()}`}
            </button>
          ))}
        </div>
        {health && health.causes.length > 0 && <span className="repair-list__causes">{causeSummary(health.causes)}</span>}
      </div>
      {tab === "attention" && <Attention unreadable={unreadable} onRecoverStarted={onRecoverStarted} />}
      {tab === "recovered" && <Recovered rows={recovered} invalidate={() => invalidateLibraryData(qc)} />}
      {tab === "lastRun" && <LastRun rows={outcomes} sourceName={sourceName ?? undefined} />}
      {tab === "history" && <History batches={history} />}
    </section>
  );
}

function Attention({ unreadable, onRecoverStarted }: { unreadable: number; onRecoverStarted: () => void }) {
  const qc = useQueryClient();
  const q = useInfiniteQuery({
    queryKey: queryKeys.unreadableRows,
    initialPageParam: null as UnreadableRowsPage["nextCursor"],
    queryFn: ({ pageParam }) => invoke<UnreadableRowsPage>("list_unreadable", { after: pageParam, limit: PAGE }),
    getNextPageParam: (last) => last.nextCursor,
  });
  const rows = q.data?.pages.flatMap((p) => p.items) ?? [];
  const [ticked, setTicked] = useState<number[]>([]);
  const refresh = () => {
    invalidateLibraryData(qc);
    void qc.invalidateQueries({ queryKey: ["unreadable"] });
    void qc.invalidateQueries({ queryKey: queryKeys.repairHealth });
  };
  const recover = async (ids: number[] | null) => {
    try {
      await invoke("start_recover", { ids });
      onRecoverStarted();
      void qc.invalidateQueries({ queryKey: queryKeys.repairStatus });
    } catch (e) {
      fail(e, "Could not start the recovery.");
    }
  };
  const putBack = async (ids: number[]) => {
    try {
      await invoke("restore_assets", { ids });
    } catch (e) {
      fail(e, "Could not put them back.");
    } finally {
      refresh();
    }
  };
  // The grid's own Trash command, so the 30-day rule and the undo are the same ones.
  const remove = async () => {
    const ids = ticked;
    try {
      await invoke("trash_assets", { ids });
      setTicked([]);
      toast({ message: ids.length === 1 ? "Moved to Trash" : `Moved ${ids.length} to Trash`, undo: () => void putBack(ids) });
    } catch (e) {
      fail(e, "Could not move to Trash.");
    } finally {
      refresh();
    }
  };
  const toggle = (id: number) => setTicked((t) => (t.includes(id) ? t.filter((x) => x !== id) : [...t, id]));
  // Every page, not just the loaded ones (unreadable files are few, so loading them all is fine).
  const selectAll = async () => {
    let last = q.data;
    let more = q.hasNextPage;
    while (more) {
      const next = await q.fetchNextPage();
      // A failed page resolves instead of throwing and leaves hasNextPage true: stop, don't spin.
      if (next.isError) return;
      last = next.data;
      more = next.hasNextPage;
    }
    setTicked((last?.pages.flatMap((p) => p.items) ?? rows).map((r) => r.id));
  };
  const allTicked = rows.length > 0 && !q.hasNextPage && rows.every((r) => ticked.includes(r.id));

  if (!q.isSuccess) return null;
  if (rows.length === 0) return <p className="repair-list__none">No file needs attention.</p>;
  return (
    <>
      <div className="repair-list__head">
        <Checkbox
          size="xs"
          label="Select all"
          checked={allTicked}
          indeterminate={ticked.length > 0 && !allTicked}
          onChange={(e) => (e.currentTarget.checked ? void selectAll() : setTicked([]))}
        />
      </div>
      <ul className="repair-rows" aria-label="Needs attention">
        {rows.map((r) => (
          <li key={r.id} className="repair-row">
            <Checkbox size="xs" aria-label={`Select ${r.filename}`} checked={ticked.includes(r.id)} onChange={() => toggle(r.id)} />
            <Thumb src={mediaUrl("thumb", r.id)} />
            <span className="repair-row__main">
              <strong>{r.filename}</strong>
              <small>{`${r.extension.toUpperCase()} · ${formatBytes(r.byteSize)}`}</small>
            </span>
            <span className="repair-row__cause">{CAUSE_WORD[r.cause]}</span>
            {canRecover(r.cause) ? (
              <Button size="compact-xs" variant="default" onClick={() => void recover([r.id])}>
                Recover
              </Button>
            ) : (
              <span />
            )}
          </li>
        ))}
      </ul>
      {q.hasNextPage && (
        <Button size="xs" variant="default" loading={q.isFetchingNextPage} onClick={() => void q.fetchNextPage()}>
          Show more
        </Button>
      )}
      <div className="repair-list__foot">
        <Button size="xs" onClick={() => void recover(ticked.length > 0 ? ticked : null)}>
          {`Recover what's readable (${(ticked.length > 0 ? ticked.length : unreadable).toLocaleString()})`}
        </Button>
        <Button size="xs" variant="default" disabled={ticked.length === 0} onClick={() => void remove()}>
          Remove selected
        </Button>
      </div>
    </>
  );
}

function Recovered({ rows, invalidate }: { rows: RecoveredRow[]; invalidate: () => void }) {
  const qc = useQueryClient();
  const [big, setBig] = useState<number | null>(null);
  const refresh = () => {
    invalidate();
    void qc.invalidateQueries({ queryKey: ["repair"] });
    void qc.invalidateQueries({ queryKey: ["unreadable"] });
  };
  const undo = async (ids: number[]) => {
    try {
      await invoke("undo_keep_recovered", { ids });
    } catch (e) {
      fail(e, "Could not undo.");
    } finally {
      refresh();
    }
  };
  const keep = async (ids: number[]) => {
    try {
      const changes = await invoke<RecoveredChanges>("keep_recovered", { ids });
      if (changes.done.length > 0) {
        toast({ message: changes.done.length === 1 ? "Kept the recovered file" : `Kept ${changes.done.length} recovered files`, undo: () => void undo(changes.done) });
      }
      changes.failed.slice(0, 1).forEach((f) => toast({ message: f.reason }));
    } catch (e) {
      fail(e, "Could not keep it.");
    } finally {
      refresh();
    }
  };
  const discard = async (ids: number[]) => {
    try {
      await invoke("discard_recovered", { ids });
    } catch (e) {
      fail(e, "Could not discard it.");
    } finally {
      refresh();
    }
  };
  if (rows.length === 0) return <p className="repair-list__none">Nothing recovered waiting for you.</p>;
  const all = rows.map((r) => r.assetId);
  return (
    <>
      <div className="repair-list__bulk">
        <Button size="compact-xs" onClick={() => void keep(all)}>
          Keep all
        </Button>
        <Button size="compact-xs" variant="default" onClick={() => void discard(all)}>
          Discard all
        </Button>
      </div>
      <ul className="repair-rows" aria-label="Recovered">
        {rows.map((r) => (
          <li key={r.assetId} className="repair-row repair-row--recovered">
            <button
              type="button"
              className="repair-row__thumb repair-row__thumb--button"
              aria-label={`Show ${r.name} larger`}
              aria-expanded={big === r.assetId}
              onClick={() => setBig(big === r.assetId ? null : r.assetId)}
            >
              {r.kind === "video" ? <IconPhotoOff size={18} aria-hidden /> : <img src={mediaUrl("recovered", r.assetId)} alt="" loading="lazy" />}
            </button>
            <span className="repair-row__main">
              <strong>{r.name}</strong>
              <small>{recoveredWord(r)}</small>
            </span>
            <Button size="compact-xs" onClick={() => void keep([r.assetId])}>
              Keep
            </Button>
            <Button size="compact-xs" variant="default" onClick={() => void discard([r.assetId])}>
              Discard
            </Button>
            {big === r.assetId && (
              <div className="repair-row__big">
                {r.kind === "video" ? <video src={mediaUrl("recovered", r.assetId)} controls /> : <img src={mediaUrl("recovered", r.assetId)} alt={r.name} />}
              </div>
            )}
          </li>
        ))}
      </ul>
    </>
  );
}

function LastRun({ rows, sourceName }: { rows: RepairOutcomeRow[]; sourceName?: string }) {
  if (rows.length === 0) return <p className="repair-list__none">Nothing has been repaired in this session yet.</p>;
  return (
    <ul className="repair-rows" aria-label="Last run">
      {rows.map((r, i) => (
        <li key={`${r.dir}/${r.name}/${i}`} className="repair-row repair-row--outcome" data-outcome={r.outcome}>
          <span className="repair-row__main">
            <strong>{r.name}</strong>
            <small>{r.dir || "/"}</small>
          </span>
          <span className="repair-row__cause">{outcomeWord(r, sourceName)}</span>
        </li>
      ))}
    </ul>
  );
}

function History({ batches }: { batches: ReturnType<typeof useImport>["batches"] }) {
  if (batches.length === 0) return <p className="repair-list__none">No repair has been run yet.</p>;
  return (
    <ul className="repair-rows" aria-label="Repair history">
      {batches.map((b) => (
        <li key={b.id} className="repair-row repair-row--outcome">
          <span className="repair-row__main">
            <strong>{b.sourceLabel}</strong>
            <small>{formatRelative(b.startedAt)}</small>
          </span>
          <span className="repair-row__cause">{plural(b.itemCount, "photo", "photos")}</span>
          <Button size="compact-xs" variant="default" component={Link} to="/library" aria-label="Open in Library">
            Open
          </Button>
        </li>
      ))}
    </ul>
  );
}
