import { Button } from "@mantine/core";
import { IconDeviceSdCard, IconDeviceDesktop, IconFolderOpen } from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { openPath } from "@tauri-apps/plugin-opener";
import { invoke } from "@/lib/ipc";
import { formatStorage } from "@/utils/format";
import type { StorageDisk } from "@/types/generated/StorageDisk";
import type { StorageOverview } from "@/types/generated/StorageOverview";
import type { StoragePart } from "@/types/generated/StoragePart";
import type { TempUsage } from "@/types/generated/TempUsage";
import { SettingGroup, SettingRow } from "./row";
import { TempFilesGroup, TempWarning } from "./temp-files";

export const storageQuery = {
  queryKey: ["storage", "overview"] as const,
  queryFn: () => invoke<StorageOverview>("storage_overview"),
  // A card can be put in or taken out while the page is open.
  refetchInterval: 10_000,
};

// Walks the library's folders, so it is read on open and after a clear, not every 10 s.
export const tempUsageQuery = {
  queryKey: ["storage", "temp"] as const,
  queryFn: () => invoke<TempUsage>("get_temp_usage"),
};

/**
 * Settings → Storage: each disk with a bar of what is used and what is free, the library's
 * own folders, and what Lumen keeps outside it. Sizes are read on open, nothing is changed.
 */
export function StorageSection() {
  const q = useQuery(storageQuery);
  const temp = useQuery(tempUsageQuery).data;
  const o = q.data;
  if (!o) return <p className="settings-storage__loading">{q.isError ? "Lumen couldn't read the disks." : "Measuring…"}</p>;
  const libraryTotal = o.library.reduce((s, p) => s + p.bytes, 0);
  const appTotal = o.app.reduce((s, p) => s + p.bytes, 0);
  return (
    <>
      <TempWarning usage={temp} />
      <SettingGroup title="Disks">
        {o.disks.map((d) => (
          <DiskRow key={d.mount} disk={d} library={d.holdsLibrary ? libraryTotal : 0} />
        ))}
      </SettingGroup>
      {o.libraryPath && (
        <SettingGroup title={`Library · ${formatStorage(libraryTotal)}`}>
          <Parts parts={o.library} total={libraryTotal} />
          <SettingRow label="Folder" hint={<code>{o.libraryPath}</code>}>
            <Button size="xs" variant="default" leftSection={<IconFolderOpen size={14} />} onClick={() => void openPath(o.libraryPath!).catch(() => {})}>
              Show
            </Button>
          </SettingRow>
        </SettingGroup>
      )}
      {temp && temp.rows.length > 0 && <TempFilesGroup usage={temp} />}
      {o.app.length > 0 && (
        <SettingGroup title={`Lumen · ${formatStorage(appTotal)}`}>
          <Parts parts={o.app} total={appTotal} />
        </SettingGroup>
      )}
    </>
  );
}

function DiskRow({ disk: d, library }: { disk: StorageDisk; library: number }) {
  const used = Math.max(0, d.total - d.available);
  const pct = (n: number) => `${Math.min(100, (n / d.total) * 100)}%`;
  const name = d.label ? `${d.label} (${d.mount.replace(/[\\/]$/, "")})` : d.mount;
  const kind = [d.removable ? "Removable" : "Internal", d.fileSystem, d.holdsLibrary && "holds your library", d.hasPhotos && "has photos to import"].filter(Boolean).join(" · ");
  return (
    <div className="settings-storage__disk">
      <div className="settings-storage__disk-head">
        {d.removable ? <IconDeviceSdCard size={18} /> : <IconDeviceDesktop size={18} />}
        <span className="settings-storage__disk-name">{name}</span>
        <span className="settings-storage__disk-free">
          {formatStorage(d.available)} free of {formatStorage(d.total)}
        </span>
      </div>
      <div
        className="settings-storage__bar"
        role="meter"
        aria-label={`${name} used`}
        aria-valuemin={0}
        aria-valuemax={d.total}
        aria-valuenow={used}
      >
        {library > 0 && <span className="settings-storage__fill" data-part="library" style={{ width: pct(library) }} />}
        <span className="settings-storage__fill" data-part="used" style={{ width: pct(Math.max(0, used - library)) }} />
      </div>
      <span className="settings-storage__disk-kind">
        {kind}
        {library > 0 && ` · library ${formatStorage(library)}`}
      </span>
    </div>
  );
}

function Parts({ parts, total }: { parts: StoragePart[]; total: number }) {
  return (
    <>
      {parts.map((p, i) => (
        <SettingRow key={p.label} label={p.label}>
          <span className="settings-storage__part">
            <span className="settings-storage__part-bar" data-i={Math.min(i, 5)} style={{ width: `${Math.max(2, (p.bytes / Math.max(1, total)) * 100)}%` }} />
            <span>{formatStorage(p.bytes)}</span>
          </span>
        </SettingRow>
      ))}
    </>
  );
}
