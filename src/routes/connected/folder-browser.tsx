import { Button, Switch } from "@mantine/core";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { IconChevronRight, IconFolder } from "@tabler/icons-react";
import { useState } from "react";
import { DeviceBrowser } from "@/components/device-browser";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { MacFolderListing } from "@/types/generated/MacFolderListing";
import { ImportProgress, useBatchProgress } from "./parts";

const join = (dir: string, name: string) => (dir === "/" ? `/${name}` : `${dir}/${name}`);

/**
 * Any folder on a Mac disk, one level at a time: the disk has no drive letter, so the
 * system's folder picker cannot see it. Photos libraries are not listed here; they have
 * their own cards above.
 */
export function FolderBrowser({ disk }: { disk: string }) {
  const qc = useQueryClient();
  const [path, setPath] = useState("/");
  const [subfolders, setSubfolders] = useState(true);
  const [batchId, setBatchId] = useState<number | null>(null);
  const [browsing, setBrowsing] = useState(false);
  const { running } = useBatchProgress(batchId);
  const q = useQuery({
    queryKey: queryKeys.macFolder(disk, path),
    queryFn: () => invoke<MacFolderListing>("list_mac_folder", { disk, path }),
    retry: false,
    staleTime: 60_000,
  });
  const listing = q.data;
  const crumbs = path.split("/").filter(Boolean);

  const importFolder = async () => {
    try {
      setBatchId(await invoke<number>("start_mac_folder_import", { disk, path, recursive: subfolders }));
      qc.invalidateQueries({ queryKey: queryKeys.batches });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    }
  };

  const empty = !!listing && listing.mediaHere === 0 && listing.folders.length === 0;
  return (
    <section className="connected-route__browser" aria-label="Folders on the disk">
      <h2 className="connected-route__heading">Folders</h2>
      <nav className="connected-route__crumbs" aria-label="Folder path">
        <button type="button" onClick={() => setPath("/")} aria-current={path === "/" ? "location" : undefined}>
          Disk
        </button>
        {crumbs.map((c, i) => {
          const to = "/" + crumbs.slice(0, i + 1).join("/");
          return (
            <span key={to}>
              <IconChevronRight size={14} aria-hidden />
              <button type="button" onClick={() => setPath(to)} aria-current={to === path ? "location" : undefined}>
                {c}
              </button>
            </span>
          );
        })}
      </nav>
      <div className="connected-route__browser-bar">
        <span className="connected-route__muted">
          {q.isLoading ? "Reading the folder…" : q.error ? "This folder could not be read." : `${(listing?.mediaHere ?? 0).toLocaleString()} photos and videos here`}
        </span>
        <Switch checked={subfolders} onChange={(e) => setSubfolders(e.currentTarget.checked)} label="Include subfolders" />
        <Button onClick={importFolder} disabled={!listing || empty || running || (!subfolders && listing.mediaHere === 0)}>
          Import this folder
        </Button>
        <Button variant="default" onClick={() => setBrowsing(true)} disabled={!listing || empty}>
          Browse…
        </Button>
      </div>
      {browsing && (
        <DeviceBrowser
          target={{ kind: "macFolder", disk, path }}
          title={crumbs[crumbs.length - 1] ?? "Disk"}
          onClose={() => setBrowsing(false)}
          onImportStarted={setBatchId}
        />
      )}
      <ImportProgress batchId={batchId} />
      {listing && listing.folders.length > 0 && (
        <ul className="connected-route__folders">
          {listing.folders.map((f) => (
            <li key={f}>
              <button type="button" onClick={() => setPath(join(path, f))}>
                <IconFolder size={18} aria-hidden />
                <span>{f}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
