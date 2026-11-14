import type { QueryClient } from "@tanstack/react-query";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { matches } from "@/lib/keybinds";
import { invoke, isAppError } from "@/lib/ipc";
import type { PreviousMark } from "@/types/generated/PreviousMark";

/** Colour labels in Lightroom's order; `value` is `assets.color_label`. */
export const LABELS = [
  { value: 1, name: "Red", color: "var(--label-red)" },
  { value: 2, name: "Yellow", color: "var(--label-yellow)" },
  { value: 3, name: "Green", color: "var(--label-green)" },
  { value: 4, name: "Blue", color: "var(--label-blue)" },
  { value: 5, name: "Purple", color: "var(--label-purple)" },
] as const;

export const FLAG_PICK = 1;
export const FLAG_REJECT = -1;
export const FLAG_NONE = 0;

export const labelName = (v: number) => LABELS.find((l) => l.value === v)?.name ?? "No label";
export const flagName = (v: number) => (v === FLAG_PICK ? "Picked" : v === FLAG_REJECT ? "Rejected" : "Unflagged");

export type MarkKind = "flag" | "label";

function message(kind: MarkKind, value: number, n: number): string {
  const what = n === 1 ? "" : ` ${n} items`;
  if (kind === "flag") {
    if (value === FLAG_PICK) return `Picked${what}`;
    if (value === FLAG_REJECT) return `Rejected${what}`;
    return `Flag cleared${n === 1 ? "" : ` on ${n} items`}`;
  }
  return value === 0 ? `Label cleared${n === 1 ? "" : ` on ${n} items`}` : `Labelled${what} ${labelName(value).toLowerCase()}`;
}

const command = (kind: MarkKind) => (kind === "flag" ? "set_flag" : "set_color_label");
const arg = (kind: MarkKind) => (kind === "flag" ? "flag" : "label");

/**
 * Set a flag or colour label on `ids`, with an Undo that puts back exactly what each photo
 * had — Rust returns the previous marks, grouped here into one call per old value.
 */
export async function applyMark(qc: QueryClient, kind: MarkKind, ids: number[], value: number): Promise<void> {
  if (ids.length === 0) return;
  try {
    const changed = await invoke<PreviousMark[]>(command(kind), { ids, [arg(kind)]: value });
    void invalidateLibraryData(qc);
    if (changed.length === 0) return;
    const byPrevious = new Map<number, number[]>();
    for (const c of changed) byPrevious.set(c.previous, [...(byPrevious.get(c.previous) ?? []), c.id]);
    toast({
      message: message(kind, value, ids.length),
      undo: () =>
        void Promise.all([...byPrevious].map(([previous, group]) => invoke(command(kind), { ids: group, [arg(kind)]: previous }))).then(() =>
          invalidateLibraryData(qc),
        ),
    });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : kind === "flag" ? "Could not set the flag." : "Could not set the label." });
  }
}

/**
 * The culling keys (spec §1): `P` pick, `X` reject, `U` unflag, `Alt+1`–`Alt+5` a colour
 * label and `Alt+0` none. Returns the mark a key event means, or null.
 */
export function markForKey(e: Pick<KeyboardEvent, "key" | "code" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">): { kind: MarkKind; value: number } | null {
  if (e.ctrlKey || e.metaKey) return null;
  if (e.altKey) {
    // `code`, because Alt+digit on some layouts produces a different `key`. The range 1–5 is
    // fixed; the person's own key for "no label" replaces Alt+0 (Round 19 §3).
    const m = /^Digit([1-5])$/.exec(e.code);
    if (m) return { kind: "label", value: Number(m[1]) };
    return matches(e as KeyboardEvent, "develop.label-none") ? { kind: "label", value: 0 } : null;
  }
  const ev = e as KeyboardEvent;
  if (matches(ev, "develop.pick")) return { kind: "flag", value: FLAG_PICK };
  if (matches(ev, "develop.reject")) return { kind: "flag", value: FLAG_REJECT };
  if (matches(ev, "develop.unflag")) return { kind: "flag", value: FLAG_NONE };
  return null;
}
