import type { Recipe } from "@/types/generated/Recipe";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { Settings } from "@/types/generated/Settings";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";
import { sameTag } from "./search-tags";
import { createStore } from "./store";

/**
 * Text in photos (Round 13-3): what the viewer, the palette and the filters share. Everything
 * about text is hidden unless smart features and *Text in photos (Glyph)* are both on.
 */
// Defensive: settings from before Round 12 (or a partial test fixture) have no `smart` block.
export const readTextOn = (s: Settings | null | undefined): boolean => !!s?.smart?.enabled && !!s.smart.experimental?.readText;

/** Every line, one per line, in reading order: what *Copy all* copies. */
export const allText = (lines: Pick<ShownTextLine, "text">[]): string => lines.map((l) => l.text).join("\n");

const SENTENCE_END = /[.!?…]["”')\]]*$/;

/** A line's top and bottom edge (0..1 of the photo), from its quad's four corners. */
function lineSpan(quad: ShownTextLine["quad"]): [number, number] {
  const ys = [quad[1], quad[3], quad[5], quad[7]];
  return [Math.min(...ys), Math.max(...ys)];
}

/**
 * The lines read as running text (the info panel's inset): consecutive lines are joined with a
 * space, a word split by a hyphen at a line's end is made whole, and a new sentence starts
 * after `. ! ? …` or where a block ends (the next line starts lower than one line height
 * below this one). The quads are only used for that gap.
 */
export function joinSentences(lines: Pick<ShownTextLine, "text" | "quad">[]): string[] {
  const out: string[] = [];
  let cur = "";
  let prev: Pick<ShownTextLine, "quad"> | null = null;
  const flush = () => {
    if (cur) out.push(cur);
    cur = "";
  };
  for (const l of lines) {
    const text = l.text.trim().replace(/\s+/g, " ");
    if (!text) continue;
    if (prev) {
      const [top, bottom] = lineSpan(prev.quad);
      if (lineSpan(l.quad)[0] - bottom > bottom - top) flush();
    }
    if (!cur) cur = text;
    else if (/\p{L}-$/u.test(cur) && /^\p{Ll}/u.test(text)) cur = cur.slice(0, -1) + text;
    else cur += ` ${text}`;
    prev = l;
    if (SENTENCE_END.test(cur)) flush();
  }
  flush();
  return out;
}

const SEARCH_LABEL_MAX = 32;

/** Text as a menu item quotes it: spaces folded, cut at 32 characters. */
export function searchLabel(text: string): string {
  const t = text.trim().replace(/\s+/g, " ");
  return t.length > SEARCH_LABEL_MAX ? `${t.slice(0, SEARCH_LABEL_MAX)}…` : t;
}

/** The grid tag for photos with text in them. */
export const HAS_TEXT_TAG: SearchTag = { label: "Has text", value: { type: "hasText" } };

export const hasTextTag = (tags: SearchTag[]): boolean => tags.some((t) => sameTag(t, HAS_TEXT_TAG));

/**
 * The photo's aspect as shown: a quarter turn swaps it, a crop (0..1 of the turned frame)
 * scales it. The overlay draws in these proportions so its text is never stretched.
 */
export function shownAspectOf(aspect: number, r: Pick<Recipe, "rotate" | "crop"> | null | undefined): number {
  if (!r) return aspect;
  const turned = r.rotate % 180 === 90 ? 1 / aspect : aspect;
  return r.crop ? (turned * r.crop.w) / r.crop.h : turned;
}

/** *Show text in photo* from the palette: the photo whose overlay to toggle. */
export const textOverlayRequest = createStore<number | null>(null);

/** Puts `text` on the clipboard; false when the clipboard refused. */
export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
