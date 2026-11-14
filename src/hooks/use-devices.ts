import { checkRoom } from "@/lib/import-space";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { DeviceSummary } from "@/types/generated/DeviceSummary";
import type { VolumeInfo } from "@/types/generated/VolumeInfo";

export const DEVICE_KIND_UNKNOWN = 0;
export const DEVICE_KIND_PHONE = 1;
export const DEVICE_KIND_CAMERA = 2;

const fail = (e: unknown, fallback: string) => toast({ message: isAppError(e) ? e.message : fallback });

/** The library's device sections (cameras and phones seen in EXIF). */
export function useDevices() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: queryKeys.devices, queryFn: () => invoke<DeviceSummary[]>("list_devices"), staleTime: 30_000 });
  const refresh = () => {
    qc.invalidateQueries({ queryKey: queryKeys.devices });
    invalidateLibraryData(qc);
  };
  const rename = useMutation({
    mutationFn: (args: { id: number; name: string }) => invoke<void>("rename_device", args),
    onSuccess: refresh,
    onError: (e) => fail(e, "Could not rename the device."),
  });
  const setHidden = useMutation({
    mutationFn: (args: { id: number; hidden: boolean }) => invoke<void>("set_device_hidden", args),
    onSuccess: (_r, args) => {
      refresh();
      if (args.hidden) toast({ message: "Device hidden", undo: () => void invoke("set_device_hidden", { id: args.id, hidden: false }).then(refresh) });
    },
    onError: (e) => fail(e, "Could not update the device."),
  });
  const devices = q.data ?? [];
  return {
    devices,
    visible: devices.filter((d) => !d.hidden),
    hidden: devices.filter((d) => d.hidden),
    isLoading: q.isLoading,
    rename: rename.mutateAsync,
    setHidden: setHidden.mutateAsync,
  };
}

/** Memory cards (volumes with DCIM) as the watcher reports them; live via `device://volumes`. */
export function useVolumes() {
  const qc = useQueryClient();
  const q = useQuery({ queryKey: queryKeys.volumes, queryFn: () => invoke<VolumeInfo[]>("list_volumes"), staleTime: Infinity });
  const startImport = useCallback(
    async (mount: string) => {
      const room = await checkRoom({ kind: "volume", id: mount });
      if (room === false) return null;
      const id = await invoke<number>("start_volume_import", { mount, limitBytes: room });
      qc.invalidateQueries({ queryKey: queryKeys.batches });
      return id;
    },
    [qc],
  );
  const forget = useCallback(async (key: string) => {
    await invoke<number>("forget_volume", { key });
    toast({ message: "This card's history was cleared" });
  }, []);
  return { volumes: q.data ?? [], startImport, forget };
}
