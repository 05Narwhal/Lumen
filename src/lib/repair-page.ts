import type { LibraryHealth } from "@/types/generated/LibraryHealth";
import type { RepairOutcomeRow } from "@/types/generated/RepairOutcomeRow";
import type { RepairPhase } from "@/types/generated/RepairPhase";
import type { UnreadableCause } from "@/types/generated/UnreadableCause";

/** Round 22 B4: the words and sums of the Repair page, kept out of the components. */

/** The phases in which a check, a re-import or a recovery is working. */
export const RUNNING_PHASES: RepairPhase[] = ["listing", "checking", "probing", "copying", "verifying"];
export const isRunningPhase = (p: RepairPhase | undefined) => p !== undefined && RUNNING_PHASES.includes(p);

export const PHASE_WORD: Record<RepairPhase, string> = {
  idle: "",
  listing: "Listing files",
  checking: "Comparing with the library",
  probing: "Testing files",
  copying: "Copying",
  verifying: "Verifying",
  done: "",
};

/** The status line under the bar: the phase, then the file it is on. */
export const phaseLine = (phase: RepairPhase, current: string | null) => [PHASE_WORD[phase], current].filter(Boolean).join(" · ");

export const CAUSE_WORD: Record<UnreadableCause, string> = {
  incomplete: "File is incomplete",
  notAnImage: "Not an image inside",
  tooLarge: "Too large to open",
  unknown: "Can't be read",
};

/** The muted line beside the chips: *35 incomplete · 16 not an image*. */
const CAUSE_SHORT: Record<UnreadableCause, string> = { incomplete: "incomplete", notAnImage: "not an image", tooLarge: "too large", unknown: "unreadable" };
export const causeSummary = (causes: LibraryHealth["causes"]) =>
  causes
    .filter((c) => c.n > 0)
    .map((c) => `${c.n.toLocaleString()} ${CAUSE_SHORT[c.cause]}`)
    .join(" · ");

/** A file Recover can work on: too large is a good file Lumen simply cannot decode yet. */
export const canRecover = (cause: UnreadableCause) => cause !== "tooLarge";

/** The four parts of the health bar; they add up to `total` (what is not flagged is healthy). */
export function healthParts(h: LibraryHealth) {
  const bad = Math.max(0, h.unreadable);
  const missing = Math.max(0, h.missing);
  const recovered = Math.max(0, h.recovered);
  const healthy = Math.max(0, h.total - bad - missing - recovered);
  return [
    { key: "healthy", label: "Open fine", n: healthy },
    { key: "unreadable", label: "Can't be opened", n: bad },
    { key: "missing", label: "File missing or changed", n: missing },
    { key: "recovered", label: "Recovered", n: recovered },
  ] as const;
}

/** *Damaged on <source> too* needs the source's name; a run that has none says "the source". */
export function outcomeWord(row: RepairOutcomeRow, source = "the source"): string {
  switch (row.outcome) {
    case "repaired":
      return "Repaired";
    case "imported":
      return "Imported";
    case "sidecar":
      return "Sidecar added";
    case "sourceDamaged":
      return `Damaged on ${source} too`;
    case "sameFile":
      return "Same file as the library's";
    case "duplicate":
      return "Already in the library as another photo";
    case "failed":
      return row.reason ? `Failed: ${row.reason}` : "Failed";
  }
}

export const recoveredWord = (r: { kind: "partial" | "preview" | "video"; percent: number; width: number; height: number }) =>
  r.kind === "preview" ? `Preview only · ${r.width} × ${r.height}` : `${r.percent} % recovered`;
