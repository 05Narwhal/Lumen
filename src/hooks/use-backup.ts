import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { BackupStatus } from "@/types/generated/BackupStatus";
import type { Schedule } from "@/types/generated/Schedule";

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** The open library's backup (Round 5A): where, how often, how the last run went. */
export function useBackup() {
  const qc = useQueryClient();
  const status = useQuery({ queryKey: queryKeys.backup, queryFn: () => invoke<BackupStatus>("get_backup") });
  const set = (s: BackupStatus) => qc.setQueryData(queryKeys.backup, s);

  const setDestination = useMutation({
    mutationFn: (destination: string | null) => invoke<BackupStatus>("set_backup_destination", { destination }),
    onSuccess: set,
    // A refused destination (inside the library, another library's backup) says why.
    onError: (e) => fail(e, "Could not use that folder."),
  });
  const setSchedule = useMutation({
    mutationFn: (schedule: Schedule) => invoke<BackupStatus>("set_backup_schedule", { schedule }),
    onSuccess: set,
    onError: (e) => fail(e, "Could not change the schedule."),
  });
  const start = useMutation({
    mutationFn: () => invoke<void>("start_backup"),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.backup }),
    onError: (e) => fail(e, "Could not start the backup."),
  });
  const cancel = useMutation({
    mutationFn: () => invoke<void>("cancel_backup"),
    onSuccess: () => qc.invalidateQueries({ queryKey: queryKeys.backup }),
  });

  /** Asks for a folder or drive, then uses it. */
  const chooseDestination = async () => {
    const picked = await openDialog({ directory: true, title: "Back up to…" });
    if (typeof picked === "string") await setDestination.mutateAsync(picked).catch(() => {});
  };

  return {
    status: status.data,
    isLoading: status.isLoading,
    chooseDestination,
    clearDestination: () => setDestination.mutateAsync(null),
    setSchedule: setSchedule.mutateAsync,
    start: start.mutateAsync,
    cancel: cancel.mutateAsync,
  };
}
