import type { PlaceKind } from "@/types/generated/PlaceKind";
import type { ReviewAction } from "@/types/generated/ReviewAction";
import type { ReviewGroup } from "@/types/generated/ReviewGroup";
import type { ReviewItem } from "@/types/generated/ReviewItem";
import type { ReviewState } from "@/types/generated/ReviewState";

export type ReviewTab = "check" | "moments" | "done";

/** The same rules as `ReviewItem::needs_check` / `is_done` in Rust, which also count the tabs. */
export const needsCheck = (i: ReviewItem) => i.status === "pending" && (i.state === "guess" || i.flag !== null);
export const isDone = (i: ReviewItem) => i.status === "confirmed" || (i.status === "pending" && !needsCheck(i));

const inTab = (i: ReviewItem, tab: ReviewTab) => {
  if (i.status === "rejected") return false;
  return tab === "check" ? needsCheck(i) : tab === "done" ? isDone(i) : true;
};

/** What a tab shows: its photos, and the moments (in the order the plugin gave) that hold them. */
export function tabView(state: ReviewState, tab: ReviewTab): { items: ReviewItem[]; moments: ReviewGroup[] } {
  const items = state.items.filter((i) => inTab(i, tab));
  const present = new Set(items.map((i) => i.group));
  return { items, moments: state.groups.filter((g) => present.has(g.id)) };
}

export const itemsOf = (items: ReviewItem[], group: string) => items.filter((i) => i.group === group);

/** The photos an action on a moment applies to (`items` are already the tab's): confirm what is still pending. */
export const targets = (items: ReviewItem[], kind: "confirm" | "reject"): number[] =>
  items.filter((i) => kind === "reject" || i.status === "pending").map((i) => i.asset);

export const confirmOf = (assets: number[]): ReviewAction => ({ kind: "confirm", assets });
export const rejectOf = (assets: number[]): ReviewAction => ({ kind: "reject", assets });
export const moveOf = (assets: number[], lat: number, lon: number): ReviewAction => ({ kind: "move", assets, lat, lon });

export interface Pin {
  key: string;
  lat: number;
  lon: number;
  state: PlaceKind;
  assets: number[];
  group: string;
  /** Some of the photos on this pin were already confirmed. */
  confirmed: boolean;
}

/**
 * One pin per spot and state, so a moment of three thousand photos in one street is one pin
 * that moves them all, not three thousand elements. The spot is the plugin's first candidate.
 */
export function pinsOf(items: ReviewItem[]): Pin[] {
  const pins = new Map<string, Pin>();
  for (const i of items) {
    const c = i.candidates[0];
    if (!c) continue;
    const key = `${i.group}|${i.state}|${c.lat.toFixed(4)}|${c.lon.toFixed(4)}`;
    const p = pins.get(key) ?? { key, lat: c.lat, lon: c.lon, state: i.state, assets: [], group: i.group, confirmed: false };
    p.assets.push(i.asset);
    p.confirmed ||= i.status === "confirmed";
    pins.set(key, p);
  }
  return [...pins.values()];
}

/** The screen's own sentence for a photo's flag. */
export const FLAG_TEXT = {
  "far-from-group": "Far from the others in its moment",
  "impossible-speed": "Too far from the previous photo for the time between",
  "low-confidence": "Not sure of this place",
} as const;

export const STATE_TEXT: Record<PlaceKind, string> = { anchor: "Known place", inferred: "Worked out from neighbours", guess: "Guess" };

/** Step through a list with the arrow keys, staying inside it. */
export function step<T>(list: T[], current: T | null, delta: 1 | -1): T | null {
  if (list.length === 0) return null;
  const at = current == null ? -1 : list.indexOf(current);
  return list[Math.min(list.length - 1, Math.max(0, at + delta))];
}
