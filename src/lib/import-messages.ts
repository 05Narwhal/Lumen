import { createStore } from "@/lib/store";
import type { ImportSummary } from "@/types/generated/ImportSummary";

const n = (v: number) => v.toLocaleString("en-US");

/** The toast when an import ends. An unplugged device is not a failed file: say how to finish. */
export function importFinishedMessage(s: ImportSummary): string {
  if (s.interrupted) {
    const who = s.label || "The device";
    return `${who} was disconnected after ${n(s.imported)} imported. Plug it back in to import the rest.`;
  }
  const parts = [`${n(s.imported)} imported`];
  if (s.skipped) parts.push(`${n(s.skipped)} skipped`);
  if (s.failed) parts.push(`${n(s.failed)} failed`);
  return (s.cancelled ? "Import cancelled: " : "Import finished: ") + parts.join(", ");
}

/** Sources whose last import was cut short this session, by the name the history gives them. */
export const interruptedImports = createStore<Set<string>>(new Set());

/** Keeps `interruptedImports` current as imports end. */
export function noteImportFinished(s: ImportSummary) {
  const next = new Set(interruptedImports.get());
  if (s.interrupted) next.add(s.label);
  else if (!s.cancelled) next.delete(s.label);
  interruptedImports.set(next);
}

/** The connected-device page's import button. */
export function importButtonLabel(newItems: number, resumed: boolean): string {
  if (newItems === 0) return "Up to date";
  return resumed ? `Resume import (${n(newItems)} left)` : `Import ${n(newItems)} new`;
}
