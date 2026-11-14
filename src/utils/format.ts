export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/**
 * A disk or card size in powers of 1000: a 128 GB card reads "128 GB", as it says on the
 * label and as Apple and camera makers count. `formatBytes` (powers of 1024) is for files.
 */
export function formatStorage(n: number): string {
  const units = ["KB", "MB", "GB", "TB", "PB"];
  let v = n / 1000;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

/** Time left for an import, as a person would say it. */
export function formatTimeLeft(ms: number): string {
  const min = Math.round(ms / 60_000);
  if (ms < 60_000) return "less than a minute left";
  if (min < 60) return `about ${min} min left`;
  const h = Math.floor(min / 60);
  const rest = min % 60;
  return rest === 0 ? `about ${h} h left` : `about ${h} h ${rest} min left`;
}
