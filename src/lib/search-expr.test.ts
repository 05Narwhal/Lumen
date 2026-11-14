import { describe, expect, it } from "vitest";
import type { Join } from "@/types/generated/Join";
import type { SearchTag } from "@/types/generated/SearchTag";
import type { TagItem } from "@/types/generated/TagItem";
import type { TagSeq } from "@/types/generated/TagSeq";
import type { TagValue } from "@/types/generated/TagValue";
import golden from "./search-expr-golden.json";
import {
  addTag,
  appendSeq,
  describe as describeSeq,
  EMPTY_SEQ,
  flatTags,
  fromLegacy,
  groupPair,
  leaves,
  orKey,
  plainTags,
  removeAt,
  removeLast,
  repair,
  replaceLastTag,
  setJoin,
  toggleNot,
  ungroup,
} from "./search-expr";

const tag = (label: string, value: TagValue): SearchTag => ({ label, value });
const live = tag("Live Photos", { type: "kind", kind: "live" });
const hdr = tag("HDR", { type: "kind", kind: "hdr" });
const videos = tag("Videos", { type: "media", media: 1 });
const photos = tag("Photos", { type: "media", media: 0 });
const paris = tag("Paris", { type: "place", place: { kind: "city", name: "Paris", country: "France" } } as TagValue);
const shot = tag("Screenshot", { type: "kind", kind: "screenshot" });
const anna = tag("Anna", { type: "person", id: 1 });
const y2026 = tag("2026", { type: "dates", from: 0, to: 1 });
const fav = tag("Favorites", { type: "favorites" });
const flagA = tag("Picked", { type: "flag", flag: 1 });
const flagB = tag("Rejected", { type: "flag", flag: 2 });

const T = (t: SearchTag, not = false): TagItem => (not ? { type: "tag", not: true, tag: t } : { type: "tag", tag: t });
const G = (seq: TagSeq, not = false): TagItem => (not ? { type: "group", not: true, seq } : { type: "group", seq });
const S = (items: TagItem[], ...joins: Join[]): TagSeq => ({ items, joins });

/** `A and (B or C)`, `not` before a negated item: structure at a glance. */
function show(seq: TagSeq): string {
  return seq.items
    .map((it, i) => {
      const lead = i > 0 ? ` ${seq.joins[i - 1]} ` : "";
      const body = it.type === "tag" ? it.tag.label : `(${show(it.seq)})`;
      return `${lead}${it.not ? "not " : ""}${body}`;
    })
    .join("");
}

describe("search-expr golden (held to Rust)", () => {
  for (const c of golden.sequences) {
    it(`repairs and reads "${c.name}"`, () => {
      const input = c.input as unknown as TagSeq;
      const repaired = c.repaired as unknown as TagSeq;
      expect(repair(input)).toEqual(repaired);
      expect(repair(repaired)).toEqual(repaired);
      expect(plainTags(repaired)).toEqual(c.plain);
    });
  }

  it("keys every sample value as Rust does", () => {
    for (const k of golden.keys) expect(orKey(k.value as unknown as TagValue)).toBe(k.key);
  });
});

describe("addTag without how", () => {
  it("ignores a tag that is already there", () => {
    const s = S([T(live)]);
    expect(addTag(s, live)).toEqual(s);
    expect(addTag(S([G(S([T(live), T(hdr)], "or"))]), hdr)).toEqual(S([T(live), T(hdr)], "or"));
  });

  it("rule 2: a top level of one field's values takes the next with or", () => {
    expect(show(addTag(S([T(live), T(hdr)], "or"), tag("Pano", { type: "kind", kind: "panorama" })))).toBe("Live Photos or HDR or Pano");
    expect(show(addTag(S([T(videos)]), photos))).toBe("Videos or Photos");
  });

  it("rule 3: it joins the field's tag or group beside other tags", () => {
    expect(show(addTag(S([T(anna), T(live)], "and"), hdr))).toBe("Anna and (Live Photos or HDR)");
    expect(show(addTag(S([G(S([T(live), T(hdr)], "or")), T(anna)], "and"), tag("Pano", { type: "kind", kind: "panorama" })))).toBe(
      "(Live Photos or HDR or Pano) and Anna",
    );
  });

  it("rule 4: anything else is anded; a top level with or is wrapped first", () => {
    expect(show(addTag(S([T(live)]), anna))).toBe("Live Photos and Anna");
    expect(show(addTag(EMPTY_SEQ, anna))).toBe("Anna");
    expect(show(addTag(S([T(anna), T(paris)], "or"), y2026))).toBe("(Anna or Paris) and 2026");
    // A flag is a single: never an or partner.
    expect(show(addTag(S([T(flagA)]), flagB))).toBe("Picked and Rejected");
  });

  it("does not treat a negated tag as an or partner", () => {
    expect(show(addTag(S([T(live, true)]), hdr))).toBe("not Live Photos and HDR");
  });
});

describe("addTag with how and appendSeq", () => {
  it("or appends as it is, and wraps an existing or", () => {
    expect(show(addTag(S([T(live)]), hdr, { join: "or", not: false }))).toBe("Live Photos or HDR");
    expect(show(addTag(S([T(anna), T(paris)], "or"), y2026, { join: "and", not: false }))).toBe("(Anna or Paris) and 2026");
    expect(show(addTag(S([T(live)]), shot, { join: "and", not: true }))).toBe("Live Photos and not Screenshot");
    expect(show(addTag(EMPTY_SEQ, shot, { join: "or", not: true }))).toBe("not Screenshot");
  });

  it("appends a sentence flat when its joins are the lead's, else as one group", () => {
    const base = S([T(anna)]);
    expect(show(appendSeq(base, S([T(live), T(hdr)], "and"), "and"))).toBe("Anna and Live Photos and HDR");
    expect(show(appendSeq(base, S([T(live), T(paris)], "or"), "and"))).toBe("Anna and (Live Photos or Paris)");
    expect(show(appendSeq(base, S([T(live), T(paris)], "or"), "or"))).toBe("Anna or Live Photos or Paris");
    expect(show(appendSeq(base, S([T(live), T(paris), T(y2026)], "and", "or"), "or"))).toBe("Anna or Live Photos and Paris or 2026");
    expect(show(appendSeq(base, S([T(live), T(paris), T(y2026)], "or", "and"), "and"))).toBe("Anna and (Live Photos or Paris and 2026)");
  });

  it("starts from an empty sequence, and ignores an empty sentence", () => {
    expect(show(appendSeq(EMPTY_SEQ, S([T(live), T(hdr)], "or"), "and"))).toBe("Live Photos or HDR");
    expect(appendSeq(S([T(anna)]), EMPTY_SEQ, "or")).toEqual(S([T(anna)]));
    expect(show(appendSeq(S([T(anna)]), S([T(live, true)]), null))).toBe("Anna and not Live Photos");
  });
});

describe("fromLegacy", () => {
  it("clusters by field in first-seen order and keeps the last of a single", () => {
    const s = fromLegacy([live, paris, hdr, flagA, anna, flagB, live]);
    expect(show(s)).toBe("(Live Photos or HDR) and Paris and Rejected and Anna");
    expect(plainTags(s)).not.toBeNull();
  });

  it("is empty for no tags and one tag for one", () => {
    expect(fromLegacy([])).toEqual(EMPTY_SEQ);
    expect(fromLegacy([fav])).toEqual(S([T(fav)]));
  });
});

describe("editing", () => {
  it("setJoin turns (Live or HDR) and 2026 into Live and HDR and 2026", () => {
    const s = S([G(S([T(live), T(hdr)], "or")), T(y2026)], "and");
    expect(show(setJoin(s, [0], 0, "and", false))).toBe("Live Photos and HDR and 2026");
  });

  it("setJoin sets the join and the not of the item after it", () => {
    const s = S([T(live), T(hdr)], "or");
    expect(show(setJoin(s, [], 0, "and", true))).toBe("Live Photos and not HDR");
    expect(show(setJoin(s, [], 0, "or", true))).toBe("Live Photos or not HDR");
    expect(show(setJoin(setJoin(s, [], 0, "and", true), [], 0, "and", false))).toBe("Live Photos and HDR");
  });

  it("toggleNot flips an item and a group", () => {
    const s = S([T(anna), G(S([T(live), T(hdr)], "or"))], "and");
    expect(show(toggleNot(s, [0]))).toBe("not Anna and (Live Photos or HDR)");
    expect(show(toggleNot(s, [1]))).toBe("Anna and not (Live Photos or HDR)");
    expect(show(toggleNot(toggleNot(s, [0]), [0]))).toBe("Anna and (Live Photos or HDR)");
  });

  it("groupPair and ungroup round trip", () => {
    const s = S([T(anna), T(live), T(hdr)], "and", "or");
    const grouped = groupPair(s, [], 1);
    expect(show(grouped)).toBe("Anna and (Live Photos or HDR)");
    expect(show(ungroup(grouped, [1]))).toBe("Anna and Live Photos or HDR");
    expect(ungroup(grouped, [1])).toEqual(repair(s));
  });

  it("groupPair works inside a group, and an and pair folds back", () => {
    const s = S([T(anna), G(S([T(live), T(hdr), T(paris)], "or", "or"))], "and");
    expect(show(groupPair(s, [1], 0))).toBe("Anna and (Live Photos or HDR or Paris)");
    // (a and b) has the same meaning bare, so repair keeps one normal form.
    expect(show(groupPair(S([T(anna), T(live)], "and"), [], 0))).toBe("Anna and Live Photos");
  });

  it("ungroup of a negated group applies De Morgan, so the meaning is kept", () => {
    const s = S([T(anna), G(S([T(live), T(hdr)], "or"), true)], "and");
    expect(show(ungroup(s, [1]))).toBe("Anna and not Live Photos and not HDR");
    const and = S([T(anna), G(S([T(live), T(hdr)], "and"), true)], "and");
    expect(show(ungroup(and, [1]))).toBe("Anna and (not Live Photos or not HDR)");
  });

  it("removeAt leaves no dangling join, whichever end it takes", () => {
    const s = S([T(live), T(hdr), T(anna)], "or", "and");
    expect(show(removeAt(s, [0]))).toBe("HDR and Anna");
    expect(show(removeAt(s, [1]))).toBe("Live Photos and Anna");
    expect(show(removeAt(s, [2]))).toBe("Live Photos or HDR");
    expect(removeAt(S([T(live)]), [0])).toEqual(EMPTY_SEQ);
  });

  it("removeAt inside a group unwraps what is left, and drops an emptied group", () => {
    const s = S([T(anna), G(S([T(live), T(hdr)], "or"))], "and");
    expect(show(removeAt(s, [1, 0]))).toBe("Anna and HDR");
    expect(show(removeAt(s, [1]))).toBe("Anna");
  });

  it("removeLast takes the last tag in reading order", () => {
    const s = S([T(anna), G(S([T(live), T(hdr)], "or"))], "and");
    expect(show(removeLast(s))).toBe("Anna and Live Photos");
    expect(removeLast(EMPTY_SEQ)).toEqual(EMPTY_SEQ);
  });
});

describe("reading", () => {
  it("leaves lists tags in order with their path and whether not excludes them", () => {
    const s = S([T(anna), G(S([T(live), T(hdr, true)], "or"), true)], "and");
    expect(leaves(s).map((l) => [l.tag.label, l.not, l.path])).toEqual([
      ["Anna", false, [0]],
      ["Live Photos", true, [1, 0]],
      ["HDR", false, [1, 1]],
    ]);
    expect(flatTags(s).map((t) => t.label)).toEqual(["Anna", "Live Photos", "HDR"]);
  });

  it("describes a sequence in words", () => {
    expect(describeSeq(S([T(y2026), G(S([T(live), T(hdr)], "or")), T(shot, true)], "and", "and"))).toBe("2026 and (Live Photos or HDR), not Screenshot");
    expect(describeSeq(S([T(shot, true), T(live)], "or"))).toBe("not Screenshot or Live Photos");
    expect(describeSeq(EMPTY_SEQ)).toBe("");
  });
});

it("replaceLastTag swaps the last leaf, inside a group too, keeping its not", () => {
  const seq = repair({ items: [{ type: "tag", tag: live }, { type: "tag", tag: hdr, not: true }], joins: ["or"] });
  expect(describeSeq(replaceLastTag(seq, videos))).toBe("Live Photos or not Videos");
  expect(flatTags(replaceLastTag(EMPTY_SEQ, videos))).toEqual([]);
});
