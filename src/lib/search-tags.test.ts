import { describe, expect, it } from "vitest";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagSeq } from "@/types/generated/TagSeq";
import { fromLegacy, plainTags } from "./search-expr";
import { FAVORITES_TAG, parseTags, writeTags } from "./search-tags";

const tag = (label: string, value: SearchTag["value"]): SearchTag => ({ label, value });
const live = tag("Live Photos", { type: "kind", kind: "live" });
const hdr = tag("HDR", { type: "kind", kind: "hdr" });
const anna = tag("Anna", { type: "person", id: 1 });

const url = (tags?: unknown, extra: Record<string, string> = {}) => {
  const p = new URLSearchParams(extra);
  if (tags !== undefined) p.set("tags", typeof tags === "string" ? tags : JSON.stringify(tags));
  return p;
};

describe("search tags in the URL", () => {
  it("reads a flat list from a link of before Round 20 as the old rule: one field is or", () => {
    const seq = parseTags(url([live, anna, hdr]));
    expect(seq.items).toHaveLength(2);
    expect(seq.items[0]).toMatchObject({ type: "group" });
    expect(seq.joins).toEqual(["and"]);
  });

  it("round trips a sequence, and a flat list written is the same as the sequence it reads as", () => {
    const seq: TagSeq = { items: [{ type: "tag", tag: live }, { type: "tag", not: true, tag: anna }], joins: ["or"] };
    const p = new URLSearchParams();
    writeTags(p, seq);
    expect(parseTags(p)).toEqual(seq);
    const q = new URLSearchParams();
    writeTags(q, [live, hdr, anna]);
    expect(parseTags(q)).toEqual(fromLegacy([live, hdr, anna]));
    expect(plainTags(parseTags(q))).toHaveLength(3);
    writeTags(q, []);
    expect(q.has("tags")).toBe(false);
  });

  it("adds Favorites for the old favorites=1, once", () => {
    expect(parseTags(url(undefined, { favorites: "1" })).items).toHaveLength(1);
    expect(parseTags(url([FAVORITES_TAG], { favorites: "1" })).items).toHaveLength(1);
    const p = new URLSearchParams({ favorites: "1" });
    writeTags(p, parseTags(p));
    expect(p.has("favorites")).toBe(false);
  });

  it("never breaks on a hand-edited value, and repairs a sloppy one", () => {
    expect(parseTags(url("{not json")).items).toEqual([]);
    expect(parseTags(url({ items: "x", joins: [] })).items).toEqual([]);
    expect(parseTags(url({ items: [{ type: "tag", tag: { label: 1 } }], joins: [] })).items).toEqual([]);
    const sloppy = { items: [{ type: "tag", tag: live }, { type: "tag", tag: anna }, { type: "group", seq: { items: [], joins: [] } }], joins: [] };
    expect(parseTags(url(sloppy))).toEqual({ items: [{ type: "tag", tag: live }, { type: "tag", tag: anna }], joins: ["and"] });
  });
});
