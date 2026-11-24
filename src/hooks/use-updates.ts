import { useQuery, useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { useNavigate } from "react-router";
import { toast } from "@/components/toast";
import { UPDATES_STATE } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { UpdateReceipt } from "@/types/generated/UpdateReceipt";
import type { UpdateStatus } from "@/types/generated/UpdateStatus";

/** Where updates are (Round 17); Rust pushes each change on `updates://state`. */
export function useUpdateStatus() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: queryKeys.updateStatus, queryFn: () => invoke<UpdateStatus>("update_status"), staleTime: Infinity });
  const set = (s: UpdateStatus) => qc.setQueryData(queryKeys.updateStatus, s);
  const failed = (e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not reach the updater." });
  return {
    status: status.data,
    check: () => invoke<UpdateStatus>("check_for_updates").then(set, failed),
    download: () => invoke<UpdateStatus>("download_update").then(set, failed),
    installNow: () => invoke<null>("install_update_now").catch(failed),
  };
}

/** Versions already announced as ready, so a state sent twice is told once. */
const toldReady = new Set<string>();
let receiptAsked = false;

/**
 * Mounted once in the shell: the one-time *Updated to …* toast after an update, and the one
 * *ready* toast when a download is staged. The state follows Rust's events.
 */
export function useUpdateNotices() {
  const qc = useQueryClient();
  const navigate = useNavigate();
  useEffect(() => {
    if (!receiptAsked) {
      receiptAsked = true;
      void invoke<UpdateReceipt | null>("take_update_receipt")
        .catch(() => null)
        .then((r) => {
          if (r) toast({ message: r.security ? `Lumen installed a security fix (${r.version})` : `Updated to ${r.version}`, action: { label: "What's new", run: () => navigate("/settings?section=updates") }, durationMs: 10_000 });
        });
    }
    const un = listen<UpdateStatus>(UPDATES_STATE, (e) => {
      qc.setQueryData(queryKeys.updateStatus, e.payload);
      const s = e.payload.state;
      if (s.state === "ready" && s.attempts === 0 && !toldReady.has(s.version)) {
        toldReady.add(s.version);
        toast({
          message: `Lumen ${s.version} is ready. It installs when you close Lumen.`,
          action: { label: "Install now", run: () => void invoke("install_update_now").catch(() => undefined) },
          durationMs: 10_000,
        });
      }
    });
    return () => {
      void un.then((f) => f());
    };
  }, [qc, navigate]);
}

/** Tests only: forget what was announced. */
export function resetUpdateNotices() {
  toldReady.clear();
  receiptAsked = false;
}
