import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { useSearchParams } from "react-router";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import { isRunningPhase } from "@/lib/repair-page";
import { restoreIds } from "@/lib/unreadable";
import type { LibraryHealth } from "@/types/generated/LibraryHealth";
import type { RepairSourceRef } from "@/types/generated/RepairSourceRef";
import type { RepairStatus } from "@/types/generated/RepairStatus";
import type { SourceFolder } from "@/types/generated/SourceFolder";
import { AttentionList } from "./attention-list";
import { HealthCard } from "./health-card";
import { SourceChips } from "./source-chips";
import { SourceResult } from "./source-result";
import "./styles.scss";

/**
 * Import → Repair (Round 22 B4, layout C): the library's health, the places the originals may be,
 * and one list card, which turns into the check of a source when one has been checked. The only
 * place in the app that deals with files Lumen can't open. A check writes nothing.
 */
export function RepairScreen() {
  const qc = useQueryClient();
  const [params] = useSearchParams();
  // `?assets=` (*Restore…*): look only for these photos.
  const assets = params.get("assets");
  const ids = useQuery({ queryKey: queryKeys.restoreIds(assets ?? ""), queryFn: () => restoreIds(assets!), enabled: assets !== null }).data;
  const folders = useQuery({
    queryKey: queryKeys.restoreFolders(assets ?? ""),
    queryFn: () => invoke<SourceFolder[]>("unreadable_source_folders", { ids }),
    enabled: ids !== undefined && ids.length > 0,
  }).data;
  const restoring = ids && ids.length > 0 ? { ids, folders: folders ?? [] } : null;
  const status = useQuery({ queryKey: queryKeys.repairStatus, queryFn: () => invoke<RepairStatus>("get_repair_status") });
  const health = useQuery({ queryKey: queryKeys.repairHealth, queryFn: () => invoke<LibraryHealth>("get_library_health") }).data;
  // The source this visit checked; its result shows until the person goes back to the health view.
  // A result left from an earlier visit, or the end of a recovery, is not a result to show.
  const [source, setSource] = useState<string | null>(null);
  const s = status.data;
  const running = isRunningPhase(s?.phase);

  const check = async (ref: RepairSourceRef, label: string) => {
    try {
      await invoke("start_repair_check", { source: ref, onlyAssets: restoring?.ids ?? null });
      qc.setQueryData(queryKeys.repairImported, null);
      setSource(label);
      void qc.invalidateQueries({ queryKey: queryKeys.repairStatus });
      void qc.invalidateQueries({ queryKey: ["repair", "items"] });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the check." });
    }
  };

  return (
    <div className="repair">
      <HealthCard repair={s} />
      <SourceChips onCheck={(ref, label) => void check(ref, label)} restoring={restoring} disabled={running} />
      {source !== null && s?.phase === "done" && !running ? (
        <SourceResult status={s} name={source} onBack={() => setSource(null)} restoring={restoring !== null} />
      ) : (
        <AttentionList health={health} sourceName={source} onRecoverStarted={() => setSource(null)} />
      )}
    </div>
  );
}
