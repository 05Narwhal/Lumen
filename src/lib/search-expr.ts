import { stable } from "@/lib/query-keys";
import type { Join } from "@/types/generated/Join";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagItem } from "@/types/generated/TagItem";
import type { TagSeq } from "@/types/generated/TagSeq";
import type { TagValue } from "@/types/generated/TagValue";

/**
 * Search tags as a sequence with `and` / `or` / `not` and groups (Round 20 §3). This is the only
 * code that edits a sequence; every edit returns the repaired form, so the page never holds a
 * dangling operator. `repair`, `plainTags` and `orKey` are held to Rust (`photos-core::tag_expr`)
 * by `search-expr-golden.json`, which a Rust test writes.
 */

/** Item indices down through nested groups: `[1, 0]` is the first item of the second item. */
export type Path = number[];

export const EMPTY_SEQ: TagSeq = { items: [], joins: [] };

export const MAX_TAGS = 24;
export const MAX_GROUP_DEPTH = 4;

type Budget = { n: number };

// Two tags that narrow the same way are the same tag (`search-tags.ts` says the same; not imported, to keep the modules acyclic).
const tagKey = (t: SearchTag) => stable(t.value);
const sameTag = (a: SearchTag, b: SearchTag) => tagKey(a) === tagKey(b);

const tagItem = (tag: SearchTag, not = false): TagItem => (not ? { type: "tag", not: true, tag } : { type: "tag", tag });
const groupItem = (seq: TagSeq, not = false): TagItem => (not ? { type: "group", not: true, seq } : { type: "group", seq });
const withNot = (item: TagItem, not: boolean): TagItem => (item.type === "tag" ? tagItem(item.tag, not) : groupItem(item.seq, not));

/** Whether a parsed JSON value is a well-formed sequence (every tag has a label and a typed value). */
export function isTagSeq(v: unknown): v is TagSeq {
  if (!v || typeof v !== "object") return false;
  const s = v as Partial<TagSeq>;
  if (!Array.isArray(s.items) || !Array.isArray(s.joins)) return false;
  if (!s.joins.every((j) => j === "and" || j === "or")) return false;
  return s.items.every((it: unknown) => {
    if (!it || typeof it !== "object") return false;
    const i = it as { type?: unknown; not?: unknown; tag?: unknown; seq?: unknown };
    if (i.not !== undefined && typeof i.not !== "boolean") return false;
    if (i.type === "group") return isTagSeq(i.seq);
    if (i.type !== "tag") return false;
    const t = i.tag as Partial<SearchTag> | undefined;
    return !!t && typeof t.label === "string" && !!t.value && typeof t.value === "object" && typeof (t.value as { type?: unknown }).type === "string";
  });
}

/**
 * One pass of `repair` over a sequence held by `depth` groups; `budget` is how many tags may
 * still be kept, in reading order. Mirrors `tag_expr::pass`.
 */
function pass(seq: TagSeq, depth: number, budget: Budget): TagSeq {
  // Rules 1 and 5 (the count): each item with the join before it, a missing join read as `and`.
  let items: [Join, TagItem][] = [];
  for (let i = 0; i < seq.items.length; i++) {
    if (budget.n === 0) break;
    const raw = seq.items[i];
    if (!raw) break;
    const join: Join = i === 0 ? "and" : (seq.joins[i - 1] ?? "and");
    let item: TagItem;
    if (raw.type === "tag") {
      budget.n -= 1;
      item = tagItem(raw.tag, !!raw.not);
    } else {
      item = groupItem(pass(raw.seq, depth + 1, budget), !!raw.not);
    }
    items.push([join, item]);
  }
  // Rule 2: an empty group goes with the join before it; a first one lets the next item lead.
  items = items.filter(([, it]) => !(it.type === "group" && it.seq.items.length === 0));
  // Rule 3: a group of one is that item, negated by either but not both.
  items = items.map(([j, it]) => {
    if (it.type === "group" && it.seq.items.length === 1) {
      const inner = it.seq.items[0];
      if (inner) return [j, withNot(inner, !!it.not !== !!inner.not)];
    }
    return [j, it];
  });
  // Rules 4 and 5 (the depth): a group is spliced into its parent when its brackets change
  // nothing, and always when it is deeper than the limit (its `not` then goes).
  const out: TagSeq = { items: [], joins: [] };
  const push = (join: Join, item: TagItem) => {
    if (out.items.length) out.joins.push(join);
    out.items.push(item);
  };
  items.forEach(([join, item], k) => {
    const before = k > 0 ? join : undefined;
    const after = items[k + 1]?.[0];
    if (item.type === "group" && (depth + 1 > MAX_GROUP_DEPTH || (!item.not && (item.seq.joins.every((j) => j === "and") || (before !== "and" && after !== "and"))))) {
      item.seq.items.forEach((inner, m) => push(m === 0 ? join : (item.seq.joins[m - 1] ?? "and"), inner));
    } else {
      push(join, item);
    }
  });
  return out;
}

/** The one normal form (Rust's `TagSeq::repaired`): joins fitted, empty groups dropped, groups of one unwrapped, groups whose brackets change nothing spliced, limits applied. */
export function repair(seq: TagSeq): TagSeq {
  let s = seq;
  for (let guard = 0; guard < 64; guard++) {
    const next = pass(s, 0, { n: MAX_TAGS });
    if (JSON.stringify(next) === JSON.stringify(s)) return s;
    s = next;
  }
  return s;
}

/**
 * How the old flat search treated two tags of this value's variant (spec §1's table): `null`
 * both must hold, `field:<kind>` either may, `single:<kind>` the last one wins.
 */
export function orKey(value: TagValue): string | null {
  switch (value.type) {
    case "media":
    case "kind":
    case "place":
    case "extension":
    case "shape":
    case "rating":
    case "months":
      return `field:${value.type}`;
    case "dates":
      return value.field && value.field !== "taken" ? `field:dates:${value.field}` : "field:dates";
    case "exif":
      return `field:exif:${value.field}`;
    case "range":
      return `field:range:${value.measure}`;
    case "flag":
    case "location":
      return `single:${value.type}`;
    default:
      return null;
  }
}

const fieldKey = (t: SearchTag): string | null => {
  const k = orKey(t.value);
  return k?.startsWith("field:") ? k : null;
};

/**
 * Tags as the sequence that says what the old flat list said (every URL and smart album from
 * before Round 20). Plain, except when the tags are one field's values alone: repair then lays
 * the lone group out as `A or B`, which Rust's `as_plain` does not take (the tree path reads it).
 */
export function fromLegacy(tags: SearchTag[]): TagSeq {
  const clusters: { key: string; tags: SearchTag[] }[] = [];
  for (const t of tags) {
    const field = fieldKey(t);
    const single = orKey(t.value)?.startsWith("single:") ? orKey(t.value) : null;
    const key = field ?? single ?? `tag:${tagKey(t)}`;
    const found = clusters.find((c) => c.key === key);
    if (!found) clusters.push({ key, tags: [t] });
    else if (single) found.tags = [t];
    else if (!found.tags.some((x) => sameTag(x, t))) found.tags.push(t);
  }
  const items = clusters.map((c) => {
    const first = c.tags[0];
    if (c.tags.length === 1 && first) return tagItem(first);
    return groupItem({ items: c.tags.map((t) => tagItem(t)), joins: c.tags.slice(1).map<Join>(() => "or") });
  });
  return repair({ items, joins: items.slice(1).map<Join>(() => "and") });
}

/** Every tag in reading order, with where it sits and whether it excludes photos (an odd number of `not`s above it). */
export function leaves(seq: TagSeq): { tag: SearchTag; not: boolean; path: Path }[] {
  const out: { tag: SearchTag; not: boolean; path: Path }[] = [];
  const walk = (s: TagSeq, excluded: boolean, at: Path) =>
    s.items.forEach((it, i) => {
      const not = excluded !== !!it.not;
      if (it.type === "tag") out.push({ tag: it.tag, not, path: [...at, i] });
      else walk(it.seq, not, [...at, i]);
    });
  walk(seq, false, []);
  return out;
}

export const flatTags = (seq: TagSeq): SearchTag[] => leaves(seq).map((l) => l.tag);

/** The flat list that says exactly what `seq` says, when there is one (Rust's `as_plain`): the old code can then run it unchanged. */
export function plainTags(seq: TagSeq): SearchTag[] | null {
  if (seq.joins.some((j) => j !== "and")) return null;
  const tags: SearchTag[] = [];
  const keys = new Set<string>();
  for (const item of seq.items) {
    let key: string | null = null;
    if (item.type === "tag") {
      if (item.not) return null;
      tags.push(item.tag);
      key = orKey(item.tag.value);
    } else {
      if (item.not || item.seq.items.length === 0 || item.seq.joins.length + 1 !== item.seq.items.length || item.seq.joins.some((j) => j !== "or")) return null;
      for (const inner of item.seq.items) {
        if (inner.type !== "tag" || inner.not) return null;
        const k = fieldKey(inner.tag);
        if (!k || (key !== null && key !== k)) return null;
        key = k;
        tags.push(inner.tag);
      }
    }
    if (key !== null) {
      if (keys.has(key)) return null;
      keys.add(key);
    }
  }
  return tags;
}

/** Runs `fn` on the sequence at `group` (`[]` is `seq` itself); a path that leaves the tree changes nothing. */
function editSeq(seq: TagSeq, group: Path, fn: (s: TagSeq) => TagSeq): TagSeq {
  const [i, ...rest] = group;
  if (i === undefined) return fn(seq);
  const item = seq.items[i];
  if (!item || item.type !== "group") return seq;
  const items = seq.items.slice();
  items[i] = groupItem(editSeq(item.seq, rest, fn), !!item.not);
  return { ...seq, items };
}

const lastOf = (path: Path): [Path, number] => [path.slice(0, -1), path[path.length - 1] ?? -1];

const isOrGroupOf = (it: TagItem, key: string): it is Extract<TagItem, { type: "group" }> =>
  it.type === "group" &&
  !it.not &&
  it.seq.items.length > 0 &&
  it.seq.joins.every((j) => j === "or") &&
  it.seq.items.every((x) => x.type === "tag" && !x.not && fieldKey(x.tag) === key);

/**
 * Adds `tag` (Round 20 §3). With no `how` (picked from the list) it keeps Round 7's feel: a
 * value of a field joins that field's tags with `or`, anything else is anded. With `how` it is
 * typed text (`or hdr`, `not screenshot`) and goes where it says.
 */
export function addTag(seq: TagSeq, tag: SearchTag, how?: { join: Join; not: boolean }): TagSeq {
  const s = repair(seq);
  if (how) return appendSeq(s, { items: [tagItem(tag, how.not)], joins: [] }, how.join);
  if (leaves(s).some((l) => sameTag(l.tag, tag))) return s; // Rule 1.
  const g = fieldKey(tag);
  if (g) {
    // Rule 2: the whole top level is this field's values joined by `or`.
    if (s.items.length && s.joins.every((j) => j === "or") && s.items.every((it) => it.type === "tag" && !it.not && fieldKey(it.tag) === g)) {
      return repair({ items: [...s.items, tagItem(tag)], joins: [...s.joins, "or"] });
    }
    // Rule 3: it joins the field's tag, or its all-`or` group, beside the other tags.
    const at = s.items.findIndex((it) => (it.type === "tag" && !it.not && fieldKey(it.tag) === g) || isOrGroupOf(it, g));
    const hit = s.items[at];
    if (hit) {
      const items = s.items.slice();
      items[at] =
        hit.type === "group" ? groupItem({ items: [...hit.seq.items, tagItem(tag)], joins: [...hit.seq.joins, "or"] }) : groupItem({ items: [hit, tagItem(tag)], joins: ["or"] });
      return repair({ items, joins: s.joins });
    }
  }
  return appendSeq(s, { items: [tagItem(tag)], joins: [] }, "and"); // Rule 4.
}

/**
 * Appends a whole sentence. `and` wraps an existing top level that has an `or` first; a
 * sentence of several items is one group unless its joins are all the lead's join.
 */
export function appendSeq(seq: TagSeq, more: TagSeq, lead: Join | null): TagSeq {
  const s = repair(seq);
  const m = repair(more);
  if (!m.items.length) return s;
  if (!s.items.length) return m;
  const join: Join = lead ?? "and";
  const base: TagSeq = join === "and" && s.joins.includes("or") ? { items: [groupItem(s)], joins: [] } : s;
  if (m.items.length === 1 || m.joins.every((j) => j === join)) {
    return repair({ items: [...base.items, ...m.items], joins: [...base.joins, join, ...m.joins] });
  }
  return repair({ items: [...base.items, groupItem(m)], joins: [...base.joins, join] });
}

/** Removes the item at `path` (a tag, or a whole group) with the join beside it. */
export function removeAt(seq: TagSeq, path: Path): TagSeq {
  const [parent, idx] = lastOf(path);
  return repair(
    editSeq(repair(seq), parent, (s) => {
      if (idx < 0 || idx >= s.items.length) return s;
      const joins = s.joins.slice();
      joins.splice(idx > 0 ? idx - 1 : 0, 1);
      return { items: s.items.filter((_, i) => i !== idx), joins };
    }),
  );
}

/** Swaps the last tag in reading order for `tag`, keeping its place and its `not` (a suggestion chosen over a typed sentence's last term). */
export function replaceLastTag(seq: TagSeq, tag: SearchTag): TagSeq {
  const swap = (s: TagSeq): TagSeq => {
    const i = s.items.length - 1;
    const it = s.items[i];
    if (!it) return s;
    const items = s.items.slice();
    items[i] = it.type === "tag" ? { ...it, tag } : { ...it, seq: swap(it.seq) };
    return { ...s, items };
  };
  return repair(swap(repair(seq)));
}

/** Removes the last tag in reading order (Backspace in the empty field). */
export function removeLast(seq: TagSeq): TagSeq {
  const last = leaves(repair(seq)).at(-1);
  return last ? removeAt(seq, last.path) : repair(seq);
}

/** Sets `joins[index]` of the sequence at `group` and the `not` of the item after it. */
export function setJoin(seq: TagSeq, group: Path, index: number, join: Join, not: boolean): TagSeq {
  return repair(
    editSeq(repair(seq), group, (s) => {
      const next = s.items[index + 1];
      if (!next || index < 0 || index >= s.joins.length) return s;
      const joins = s.joins.slice();
      joins[index] = join;
      const items = s.items.slice();
      items[index + 1] = withNot(next, not);
      return { items, joins };
    }),
  );
}

/** Flips the `not` of the item at `path`. */
export function toggleNot(seq: TagSeq, path: Path): TagSeq {
  const [parent, idx] = lastOf(path);
  return repair(
    editSeq(repair(seq), parent, (s) => {
      const it = s.items[idx];
      if (!it) return s;
      const items = s.items.slice();
      items[idx] = withNot(it, !it.not);
      return { ...s, items };
    }),
  );
}

/** Brackets items `index` and `index + 1` of the sequence at `group`. (An all-`and` pair folds back: repair keeps one normal form.) */
export function groupPair(seq: TagSeq, group: Path, index: number): TagSeq {
  return repair(
    editSeq(repair(seq), group, (s) => {
      const a = s.items[index];
      const b = s.items[index + 1];
      const between = s.joins[index];
      if (!a || !b || !between) return s;
      const joins = s.joins.slice();
      joins.splice(index, 1);
      const items = s.items.slice();
      items.splice(index, 2, groupItem({ items: [a, b], joins: [between] }));
      return { items, joins };
    }),
  );
}

const flip = (j: Join): Join => (j === "and" ? "or" : "and");

/**
 * Takes the group at `path` apart into its parent. A negated group is first turned into its
 * De Morgan form (`not (A or B)` = `not A and not B`), so the meaning is kept; repair then
 * decides whether its brackets can go.
 */
export function ungroup(seq: TagSeq, path: Path): TagSeq {
  const [parent, idx] = lastOf(path);
  return repair(
    editSeq(repair(seq), parent, (s) => {
      const it = s.items[idx];
      if (!it || it.type !== "group") return s;
      if (it.not) {
        const items = s.items.slice();
        items[idx] = groupItem({ items: it.seq.items.map((x) => withNot(x, !x.not)), joins: it.seq.joins.map(flip) });
        return { ...s, items };
      }
      return {
        items: [...s.items.slice(0, idx), ...it.seq.items, ...s.items.slice(idx + 1)],
        joins: [...s.joins.slice(0, idx), ...it.seq.joins, ...s.joins.slice(idx)],
      };
    }),
  );
}

/** The sequence in words, for a screen reader or a tooltip: `2026 and (Live Photos or HDR), not Screenshot`. */
export function describe(seq: TagSeq): string {
  return seq.items
    .map((it, i) => {
      const body = it.type === "tag" ? it.tag.label : `(${describe(it.seq)})`;
      const text = `${it.not ? "not " : ""}${body}`;
      if (i === 0) return text;
      const join = seq.joins[i - 1] ?? "and";
      return join === "and" && it.not ? `, ${text}` : ` ${join} ${text}`;
    })
    .join("");
}
