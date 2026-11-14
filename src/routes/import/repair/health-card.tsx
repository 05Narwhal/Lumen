import { Button } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { jobsStore } from "@/lib/jobs-store";
import { queryKeys } from "@/lib/query-keys";
import { healthParts, isRunningPhase, phaseLine } from "@/lib/repair-page";
import type { CheckStatus } from "@/types/generated/CheckStatus";
import type { LibraryHealth } from "@/types/generated/LibraryHealth";
import type { RepairStatus } from "@/types/generated/RepairStatus";
import { formatRelative } from "@/utils/date";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/**
 * The top of the Repair page (Round 22 B4): how many files open, one stacked bar of what the
 * library knows without a source, and the check of the whole library. While a check, a re-import
 * or a recovery runs the bar is its progress and the line under it says what it is doing.
 */
export function HealthCard({ repair }: { repair: RepairStatus | undefined }) {
  const qc = useQueryClient();
  const health = useQuery({ queryKey: queryKeys.repairHealth, queryFn: () => invoke<LibraryHealth>("get_library_health") }).data;
  const check = useQuery({ queryKey: queryKeys.check, queryFn: () => invoke<CheckStatus>("get_check_status") }).data;
  const checkLine = jobsStore.use().jobs.find((j) => j.id === "check");

  const repairing = isRunningPhase(repair?.phase);
  const checking = check?.running != null;
  const running = repairing || checking;

  const startCheck = async () => {
    try {
      await invoke("start_check_library");
      void qc.invalidateQueries({ queryKey: queryKeys.check });
    } catch (e) {
      fail(e, "Could not start the check.");
    }
  };
  const cancel = () => {
    const cmd = repairing ? "cancel_repair" : "cancel_check";
    void invoke(cmd).catch((e: unknown) => fail(e, "Could not stop it."));
  };

  if (!health) return <section className="repair-health" aria-label="Library health" />;

  const parts = healthParts(health);
  const open = health.total - health.unreadable;
  // The bar's fill while something runs: files done of files in all.
  const fill = repairing && repair ? (repair.total > 0 ? repair.done / repair.total : 0) : (checkLine?.progress ?? 0);
  const line = repairing && repair ? phaseLine(repair.phase, repair.current) : (checkLine?.detail ? `Checking the library · ${checkLine.detail}` : "Checking the library");
  const large = repairing && repair && repair.bytesTotal > 0 ? ` · ${Math.round((repair.bytesDone / repair.bytesTotal) * 100)} %` : "";

  return (
    <section className="repair-health" aria-label="Library health">
      <div className="repair-health__head">
        <strong className="repair-health__title">{`${open.toLocaleString()} of ${health.total.toLocaleString()} open fine`}</strong>
        {running ? (
          <Button size="xs" variant="default" onClick={cancel}>
            Cancel
          </Button>
        ) : (
          <Button size="xs" onClick={() => void startCheck()}>
            Check the library
          </Button>
        )}
      </div>
      {running ? (
        <div
          className="repair-health__bar repair-health__bar--progress"
          role="progressbar"
          aria-label="Progress"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(fill * 100)}
        >
          <span style={{ width: `${Math.min(100, fill * 100)}%` }} />
        </div>
      ) : (
        <div className="repair-health__bar" role="img" aria-label="Library health">
          {parts
            .filter((p) => p.n > 0)
            .map((p) => (
              <span key={p.key} data-part={p.key} style={{ flexGrow: p.n }} title={`${p.label}: ${p.n.toLocaleString()}`} />
            ))}
        </div>
      )}
      {running ? (
        <p className="repair-health__line" role="status">{`${line}${large}`}</p>
      ) : (
        <>
          <ul className="repair-health__legend" aria-label="Legend">
            {parts.map((p) => (
              <li key={p.key} data-part={p.key}>
                <i aria-hidden /> {p.label} <b>{p.n.toLocaleString()}</b>
              </li>
            ))}
          </ul>
          <p className="repair-health__line">{health.checkedAt == null ? "Not checked yet" : `Checked ${formatRelative(health.checkedAt)}`}</p>
        </>
      )}
    </section>
  );
}
