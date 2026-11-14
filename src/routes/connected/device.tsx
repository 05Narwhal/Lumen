import { importButtonLabel, interruptedImports } from "@/lib/import-messages";
import { Button } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { IconDeviceSdCard, IconDeviceUsb } from "@tabler/icons-react";
import { appleDeviceIcon } from "@/lib/device-icons";
import { useEffect, useRef, useState } from "react";
import { useNavigate, useSearchParams } from "react-router";
import { DeviceBrowser } from "@/components/device-browser";
import { PageHeader } from "@/components/page-header";
import { toast } from "@/components/toast";
import { useVolumes } from "@/hooks/use-devices";
import { useIphones } from "@/hooks/use-iphones";
import { useRunningImport } from "@/hooks/use-running-import";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ConnectedDevice } from "@/types/generated/ConnectedDevice";
import { DeviceGone } from "./gone";
import { ImportProgress, Previews, Storage, useBatchProgress } from "./parts";

/**
 * A connected card, drive or iPhone before anything is imported from it: its newest photos
 * so you know it is the right one, its storage, how much is new, and the import itself
 * with its progress.
 */
export function DeviceRoute({ kind, id }: { kind: string; id: string }) {
  // Before any early return: a hook must run on every render.
  const interrupted = interruptedImports.use();
  const qc = useQueryClient();
  const { volumes, startImport: startVolumeImport, forget } = useVolumes();
  const { devices: iphones, startImport: startIphoneImport } = useIphones();
  const q = useQuery({
    queryKey: queryKeys.connected(kind, id),
    queryFn: () => invoke<ConnectedDevice>("get_connected_device", { kind, id }),
    // A device that has gone does not come back by asking again; the watcher's events
    // and the Try again button refetch instead.
    retry: false,
    staleTime: 30_000,
  });
  const [batchId, setBatchId] = useState<number | null>(null);
  const [starting, setStarting] = useState(false);
  const [browsing, setBrowsing] = useState(false);
  // *Browse…* from the sidebar's menu arrives as `?browse=1` (Round 4G).
  const [params, setParams] = useSearchParams();
  useEffect(() => {
    if (params.get("browse") !== "1") return;
    setBrowsing(true);
    setParams(
      (p) => {
        p.delete("browse");
        return p;
      },
      { replace: true },
    );
  }, [params, setParams]);
  const volume = kind === "volume" ? volumes.find((v) => v.mount === id) : undefined;
  const phone = kind === "iphone" ? iphones.find((p) => p.udid === id) : undefined;
  // An import started here, from the browser or from the Import page, found again after
  // this page was left: its progress shows and the buttons wait for it.
  const elsewhere = useRunningImport(kind === "iphone" ? `ios:${id}` : volume?.key);
  useEffect(() => {
    if (elsewhere.batchId !== null) setBatchId(elsewhere.batchId);
  }, [elsewhere.batchId]);
  const { running: importing } = useBatchProgress(batchId);

  // Unplugged or ejected: the page closes rather than showing a device that is not there.
  const navigate = useNavigate();
  const present = kind === "volume" ? volume !== undefined : kind === "iphone" ? phone !== undefined : true;
  const seen = useRef(false);
  useEffect(() => {
    if (present) {
      seen.current = true;
      return;
    }
    if (!seen.current) return;
    toast({ message: `${q.data?.name ?? "The device"} was disconnected` });
    navigate("/import", { replace: true });
  }, [present, navigate, q.data?.name]);
  const device = q.data;
  // The name is known from the watcher before the (slow, on a phone) listing arrives.
  const name = device?.name ?? volume?.label ?? phone?.name ?? "Connected device";
  const removable = device?.removable ?? volume?.removable ?? true;
  const Icon = kind === "iphone" ? appleDeviceIcon(phone?.model) : removable ? IconDeviceSdCard : IconDeviceUsb;
  const what = kind === "iphone" ? (phone?.model?.startsWith("iPad") ? "iPad" : "iPhone") : removable ? "Memory card" : "Drive";

  const start = async () => {
    setStarting(true);
    try {
      setBatchId(kind === "iphone" ? await startIphoneImport(id) : await startVolumeImport(id));
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setStarting(false);
    }
  };
  const importAgain = async () => {
    if (!volume) return;
    await forget(volume.key);
    await qc.invalidateQueries({ queryKey: queryKeys.connected(kind, id) });
  };

  if (q.error) return <DeviceGone name={name} error={q.error} retry={() => void q.refetch()} />;

  const newItems = device?.newItems ?? 0;
  return (
    <div className="connected-route">
      <PageHeader title={name} search={false} />
      <div className="connected-route__body">
        <section className="connected-route__summary" aria-label="Device">
          <span className="connected-route__icon">
            <Icon size={26} />
          </span>
          <div className="connected-route__facts">
            <span className="connected-route__kind">{what}</span>
            {device ? (
              <>
                <strong>
                  {newItems === 0
                    ? `Nothing new · ${device.totalItems.toLocaleString()} items already imported`
                    : `${newItems.toLocaleString()} new of ${device.totalItems.toLocaleString()} items`}
                </strong>
                <Storage storage={device.storage} mediaBytes={device.totalBytes} />
              </>
            ) : (
              <span className="connected-route__muted">Reading what is on it…</span>
            )}
          </div>
          <div className="connected-route__actions">
            <div className="connected-route__buttons">
              <Button variant="default" onClick={() => setBrowsing(true)} disabled={!device || device.totalItems === 0}>
                Browse…
              </Button>
              <Button onClick={start} loading={starting} disabled={!device || newItems === 0 || importing}>
                {!device ? "Import" : importButtonLabel(newItems, interrupted.has(device.name))}
              </Button>
            </div>
            {kind === "volume" && device && newItems < device.totalItems && !importing && (
              <Button variant="subtle" size="xs" onClick={() => void importAgain()}>
                Import everything again
              </Button>
            )}
          </div>
        </section>

        <ImportProgress batchId={batchId} />

        <section aria-label="Newest on the device">
          <h2 className="connected-route__heading">Newest</h2>
          {device && device.newest.length === 0 ? (
            <p className="connected-route__muted">No photos or videos on it.</p>
          ) : (
            <Previews newest={device?.newest ?? null} total={device?.totalItems ?? 0} onMore={() => setBrowsing(true)} />
          )}
        </section>
        {browsing && (kind === "volume" || kind === "iphone") && (
          <DeviceBrowser
            target={{ kind, id }}
            title={name}
            onClose={() => setBrowsing(false)}
            onImportStarted={(batch) => {
              setBatchId(batch);
              void qc.invalidateQueries({ queryKey: queryKeys.connected(kind, id) });
            }}
          />
        )}
      </div>
    </div>
  );
}
