import { Button } from "@mantine/core";
import { IconBrandApple } from "@tabler/icons-react";
import { useState } from "react";
import { DeviceBrowser } from "@/components/device-browser";
import { toast } from "@/components/toast";
import { usePhotosLibrary } from "@/hooks/use-photos-library";
import { isAppError } from "@/lib/ipc";
import type { LibrarySummary } from "@/types/generated/LibrarySummary";
import { ImportProgress, Previews, useBatchProgress } from "./parts";

/** `/Photo Library/Julian's.photoslibrary` → `Julian's` */
export function libraryName(path: string) {
  const last = path.split(/[\\/]/).filter(Boolean).pop() ?? "Photos Library";
  return last.replace(/\.photoslibrary$/i, "");
}

/** What the import leaves behind, and why, in the words of someone who uses Photos. */
function skipped(s: LibrarySummary) {
  const parts: string[] = [];
  const n = (x: number) => x.toLocaleString();
  if (s.notDownloaded) parts.push(`${n(s.notDownloaded)} not downloaded from iCloud`);
  if (s.referenced) parts.push(`${n(s.referenced)} referenced (kept outside the library)`);
  if (s.burstFrames) parts.push(`${n(s.burstFrames)} unpicked burst frames`);
  if (s.trashed) parts.push(`${n(s.trashed)} recently deleted`);
  return parts;
}

/**
 * One Apple Photos library: how much it holds, what the import will leave out and why,
 * its newest photos, and the import with its progress.
 */
export function LibraryCard({ disk, path }: { disk: string | null; path: string }) {
  const { info, isLoading, error, startImport } = usePhotosLibrary(disk, path);
  const [batchId, setBatchId] = useState<number | null>(null);
  const [starting, setStarting] = useState(false);
  const [browsing, setBrowsing] = useState(false);
  const { running } = useBatchProgress(batchId);
  const s = info?.summary;

  const start = async () => {
    setStarting(true);
    try {
      setBatchId(await startImport());
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setStarting(false);
    }
  };

  const everything = !!info && info.newItems === info.totalFiles;
  const label = !info ? "Import library" : info.newItems === 0 ? "Up to date" : everything ? "Import library" : `Import ${info.newItems.toLocaleString()} new`;
  const left = s ? skipped(s) : [];

  return (
    <section className="connected-route__library" aria-label={`Photos library ${libraryName(path)}`}>
      <div className="connected-route__summary">
        <span className="connected-route__icon">
          <IconBrandApple size={26} />
        </span>
        <div className="connected-route__facts">
          <span className="connected-route__kind">Apple Photos library</span>
          <strong>{s?.name ?? libraryName(path)}</strong>
          {error ? (
            <span className="connected-route__muted">{isAppError(error) ? error.message : "The library could not be read."}</span>
          ) : isLoading || !s ? (
            <span className="connected-route__muted">Reading the library…</span>
          ) : (
            <>
              <span>
                {`${s.items.toLocaleString()} photos and videos`}
                {s.livePhotos > 0 && ` · ${s.livePhotos.toLocaleString()} Live Photos`}
              </span>
              {left.length > 0 && <span className="connected-route__muted">Left out: {left.join(" · ")}</span>}
              <span className="connected-route__muted">Favourites, albums, titles, captions, keywords, dates and places come along.</span>
            </>
          )}
        </div>
        <div className="connected-route__actions">
          <div className="connected-route__buttons">
            <Button variant="default" onClick={() => setBrowsing(true)} disabled={!info || info.totalFiles === 0}>
              Browse…
            </Button>
            <Button onClick={start} loading={starting} disabled={!info || info.newItems === 0 || running}>
              {label}
            </Button>
          </div>
        </div>
      </div>
      <ImportProgress batchId={batchId} />
      {!error && <Previews newest={info?.newest ?? null} total={s?.items ?? 0} onMore={() => setBrowsing(true)} />}
      {browsing && (
        <DeviceBrowser
          target={{ kind: "photosLibrary", disk, path }}
          title={s?.name ?? libraryName(path)}
          onClose={() => setBrowsing(false)}
          onImportStarted={setBatchId}
        />
      )}
    </section>
  );
}
