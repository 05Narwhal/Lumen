import { useQuery } from "@tanstack/react-query";
import { IconBrandApple } from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { DeviceBrowser } from "@/components/device-browser";
import { PageHeader } from "@/components/page-header";
import { useMacDisks } from "@/hooks/use-mac-disks";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ConnectedDevice } from "@/types/generated/ConnectedDevice";
import { FolderBrowser } from "./folder-browser";
import { DeviceGone } from "./gone";
import { LibraryCard } from "./library-card";
import { Storage } from "./parts";

/**
 * A Mac-formatted disk read through its partition device: its storage, a card per Apple
 * Photos library on it, and a browser to import any folder.
 */
export function MacDiskRoute({ id }: { id: string }) {
  const { disks } = useMacDisks();
  const disk = disks.find((d) => d.id === id);
  const q = useQuery({
    queryKey: queryKeys.connected("mac", id),
    queryFn: () => invoke<ConnectedDevice>("get_connected_device", { kind: "mac", id }),
    retry: false,
    staleTime: 30_000,
  });
  // *Browse…* from the sidebar's menu (Round 4G): the whole disk, from its root.
  const [params, setParams] = useSearchParams();
  const [browsing, setBrowsing] = useState(false);
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
  const name = q.data?.name ?? disk?.name ?? "Mac disk";
  if (q.error) return <DeviceGone name={name} error={q.error} retry={() => void q.refetch()} />;

  return (
    <div className="connected-route">
      <PageHeader title={name} search={false} />
      <div className="connected-route__body">
        <section className="connected-route__summary" aria-label="Device">
          <span className="connected-route__icon">
            <IconBrandApple size={26} />
          </span>
          <div className="connected-route__facts">
            <span className="connected-route__kind">Mac disk{disk ? ` · ${disk.fs === "apfs" ? "APFS" : "HFS+"}` : ""}</span>
            <strong>Read-only: nothing on it is changed.</strong>
            {q.data ? <Storage storage={q.data.storage} /> : <span className="connected-route__muted">Reading the disk…</span>}
          </div>
        </section>
        {disk?.libraries.map((path) => (
          <LibraryCard key={path} disk={id} path={path} />
        ))}
        <FolderBrowser disk={id} />
      </div>
      {browsing && <DeviceBrowser target={{ kind: "macFolder", disk: id, path: "/" }} title={name} onClose={() => setBrowsing(false)} />}
    </div>
  );
}
