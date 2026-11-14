import { stable } from "@/lib/query-keys";
import { addTag, EMPTY_SEQ, fromLegacy, isTagSeq, repair } from "@/lib/search-expr";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagSeq } from "@/types/generated/TagSeq";

/**
 * Search tags (Round 7, a sequence since Round 20) in the URL: `tags=` holds them as JSON, so Back, a reload and a
 * shared link keep a page's filter. Rust turns them into the query (`tag_search::to_query`).
 */

export const FAVORITES_TAG: SearchTag = {
  label: "Favorites",
  value: { type: "favorites" },
};

/** One string per tag value: two tags that narrow the same way are the same tag. */
export const tagKey = (t: SearchTag) => stable(t.value);

export const sameTag = (a: SearchTag, b: SearchTag) => tagKey(a) === tagKey(b);

function isTag(v: unknown): v is SearchTag {
  if (!v || typeof v !== "object") return false;
  const t = v as Partial<SearchTag>;
  return typeof t.label === "string" && !!t.value && typeof t.value === "object" && typeof (t.value as { type?: unknown }).type === "string";
}

/**
 * The tags of a URL as a sequence. `tags=` holds a `TagSeq` as JSON; a JSON array (a link from
 * before Round 20) is read as the old flat list; `favorites=1` (before Round 7) is the Favorites
 * tag. A hand-edited value never breaks the page: whatever is invalid is empty.
 */
export function parseTags(params: URLSearchParams): TagSeq {
  let seq: TagSeq = EMPTY_SEQ;
  const raw = params.get("tags");
  if (raw) {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (Array.isArray(parsed)) seq = fromLegacy(parsed.filter(isTag));
      else if (isTagSeq(parsed)) seq = repair(parsed);
    } catch {
      // Ignored, as a bad `filters=` is.
    }
  }
  return params.get("favorites") ? addTag(seq, FAVORITES_TAG) : seq;
}

/** Writes `value` into `params`, in place; a flat list (every existing writer of links) is read as the old rule. */
export function writeTags(params: URLSearchParams, value: TagSeq | SearchTag[]) {
  const seq = Array.isArray(value) ? fromLegacy(value) : repair(value);
  params.delete("favorites");
  if (seq.items.length) params.set("tags", JSON.stringify(seq));
  else params.delete("tags");
}

/** Accents folded and case lowered, as Rust's `query_parse::fold` matches. */
export const foldText = (s: string) =>
  s
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLowerCase();

/**
 * `label` cut into runs, `true` where a typed word begins one of its words: what the list
 * shows in bold.
 */
export function highlight(label: string, input: string): [string, boolean][] {
  const words = foldText(input)
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  if (!words.length) return [[label, false]];
  const out: [string, boolean][] = [];
  const re = /[\p{L}\p{N}]+|[^\p{L}\p{N}]+/gu;
  for (const [part] of label.matchAll(re)) {
    const folded = foldText(part);
    const n = /[\p{L}\p{N}]/u.test(part) ? Math.max(0, ...words.filter((w) => folded.startsWith(w)).map((w) => w.length)) : 0;
    // Folding can change a word's length (ß → ss); only bold when it did not.
    if (n > 0 && folded.length === part.length) {
      out.push([part.slice(0, n), true]);
      if (n < part.length) out.push([part.slice(n), false]);
    } else {
      out.push([part, false]);
    }
  }
  return out;
}
