const MB = 1024 * 1024;
const GB = 1024 * MB;

/** "500 MB", "1.9 GB": the sizes Setup shows. Whole MB below 10 GB's worth of tenths. */
export function formatBytes(bytes: number): string {
  if (bytes >= GB) return `${(bytes / GB).toFixed(bytes >= 10 * GB ? 0 : 1)} GB`;
  return `${Math.max(1, Math.round(bytes / MB))} MB`;
}

/**
 * A folder path short enough for one line: the drive and user folder, `…`, then as many of the
 * last folders as fit (the last one always whole). The full path goes in a `title`.
 */
export function shortPath(path: string, max = 56): string {
  if (path.length <= max) return path;
  const sep = path.includes("\\") ? "\\" : "/";
  const parts = path.split(sep);
  const head = parts.slice(0, 3);
  let tail = [parts[parts.length - 1]];
  for (let i = parts.length - 2; i >= head.length; i--) {
    const next = [parts[i], ...tail];
    if ([...head, "…", ...next].join(sep).length > max) break;
    tail = next;
  }
  if (head.length + tail.length >= parts.length) return path;
  return [...head, "…", ...tail].join(sep);
}

export function formatTimeLeft(seconds: number): string {
  if (seconds < 60) return "under a minute left";
  return `${Math.round(seconds / 60)} min left`;
}
