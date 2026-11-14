import { Progress } from "@mantine/core";
import { IconPlayerPlayFilled } from "@tabler/icons-react";
import { useState } from "react";
import { BATCH_RUNNING, useImport } from "@/hooks/use-import";
import { jobsStore } from "@/lib/jobs-store";
import type { DevicePreview } from "@/types/generated/DevicePreview";
import type { DeviceStorage } from "@/types/generated/DeviceStorage";
import { formatStorage } from "@/utils/format";
import { mediaUrl } from "@/utils/media";

export const PLACEHOLDERS = 12;

function Thumb({ token, fileName, video }: DevicePreview) {
  const [loaded, setLoaded] = useState(false);
  return (
    <figure className="connected-route__thumb" title={fileName}>
      <img src={mediaUrl("device-thumb", token)} alt={fileName} loading="lazy" data-loaded={loaded} onLoad={() => setLoaded(true)} />
      {video && (
        <span className="connected-route__video" aria-hidden>
          <IconPlayerPlayFilled size={12} />
        </span>
      )}
    </figure>
  );
}

/**
 * The newest items as small squares, and when there are more than fit, the last square
 * counts the rest instead of showing one more photo: "+311 more" says how big the source
 * is at a glance. `null` while loading draws empty squares.
 */
export function Previews({ newest, total, onMore }: { newest: DevicePreview[] | null; total: number; onMore?: () => void }) {
  if (!newest) {
    return (
      <div className="connected-route__grid">
        {Array.from({ length: PLACEHOLDERS }, (_, i) => (
          <div key={i} className="connected-route__thumb" aria-hidden />
        ))}
      </div>
    );
  }
  const more = total > newest.length;
  const shown = more ? newest.slice(0, -1) : newest;
  const rest = total - shown.length;
  return (
    <div className="connected-route__grid">
      {shown.map((p) => (
        <Thumb key={p.token} {...p} />
      ))}
      {more &&
        (onMore ? (
          // The count is the way in to everything on the device (Round 4E).
          <button
            type="button"
            className="connected-route__thumb connected-route__more"
            aria-label={`${rest.toLocaleString()} more on the device`}
            title="Browse everything on it"
            onClick={onMore}
          >
            <strong>+{rest.toLocaleString()}</strong>
            <span>more</span>
          </button>
        ) : (
          <div className="connected-route__thumb connected-route__more" aria-label={`${rest.toLocaleString()} more on the device`}>
            <strong>+{rest.toLocaleString()}</strong>
            <span>more</span>
          </div>
        ))}
    </div>
  );
}

/**
 * Storage as one bar. With `mediaBytes` it has three parts — the photos and videos Lumen
 * would import, everything else, and what is free — otherwise used and free. Sizes count
 * in thousands, as the label on the card and the phone's own Settings do.
 */
export function Storage({ storage, mediaBytes }: { storage: DeviceStorage | null; mediaBytes?: number }) {
  if (!storage) return <p className="connected-route__muted">The device does not report its storage.</p>;
  const { totalBytes, freeBytes } = storage;
  const used = Math.max(0, totalBytes - freeBytes);
  const media = mediaBytes === undefined ? 0 : Math.min(mediaBytes, used);
  const other = used - media;
  const pct = (n: number) => (totalBytes ? Math.round((n / totalBytes) * 100) : 0);
  return (
    <div className="connected-route__storage">
      <Progress.Root size="md" radius="xl">
        {mediaBytes !== undefined && (
          <Progress.Section value={pct(media)} color="var(--accent)" aria-label={`Photos and videos: ${formatStorage(media)}`} />
        )}
        <Progress.Section
          value={pct(other)}
          color={mediaBytes === undefined ? "var(--accent)" : "var(--fg-muted)"}
          aria-label={mediaBytes === undefined ? `Used: ${formatStorage(other)}` : `Other: ${formatStorage(other)}`}
        />
      </Progress.Root>
      <div className="connected-route__legend">
        {mediaBytes !== undefined && <span data-part="media">Photos and videos {formatStorage(media)}</span>}
        {mediaBytes !== undefined && <span data-part="other">Other {formatStorage(other)}</span>}
        <span className="connected-route__free">{`${formatStorage(freeBytes)} free of ${formatStorage(totalBytes)}`}</span>
      </div>
    </div>
  );
}

/** Whether batch `batchId` is still running, and its live progress from the activity store. */
export function useBatchProgress(batchId: number | null) {
  const { batches } = useImport();
  const job = jobsStore.use().jobs.find((j) => j.id === `import:${batchId}`);
  const running = batchId !== null && (job !== undefined || batches.some((b) => b.id === batchId && b.state === BATCH_RUNNING));
  return { running, job };
}

export function ImportProgress({ batchId }: { batchId: number | null }) {
  const { running, job } = useBatchProgress(batchId);
  if (!running) return null;
  const pct = Math.round((job?.progress ?? 0) * 100);
  return (
    <section className="connected-route__progress" aria-label="Import">
      <div className="connected-route__progress-line">
        <strong>{job?.label ?? "Starting the import…"}</strong>
        <span>{pct}%</span>
      </div>
      <Progress value={pct} size="sm" radius="xl" aria-label="Import progress" animated striped />
      {job?.detail && <span className="connected-route__muted connected-route__current">{job.detail}</span>}
    </section>
  );
}
