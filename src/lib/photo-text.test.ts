import { describe, expect, it } from "vitest";
import type { Settings } from "@/types/generated/Settings";
import type { Recipe } from "@/types/generated/Recipe";
import type { ShownTextLine } from "@/types/generated/ShownTextLine";
import { allText, hasTextTag, HAS_TEXT_TAG, joinSentences, readTextOn, searchLabel, shownAspectOf } from "./photo-text";

const settings = (enabled: boolean, readText: boolean) => ({ smart: { enabled, experimental: { readText } } }) as unknown as Settings;
const line = (text: string, ord: number) => ({ ord, text, quad: [0, 0, 1, 0, 1, 1, 0, 1] });

describe("photo text helpers", () => {
  it("reads text only with both smart features and the Glyph switch on", () => {
    expect(readTextOn(undefined)).toBe(false);
    expect(readTextOn({} as Settings)).toBe(false);
    expect(readTextOn({ smart: { enabled: true } } as unknown as Settings)).toBe(false);
    expect(readTextOn(settings(true, false))).toBe(false);
    expect(readTextOn(settings(false, true))).toBe(false);
    expect(readTextOn(settings(true, true))).toBe(true);
  });

  it("copies every line, one per line, in reading order", () => {
    expect(allText([line("Gostilna", 0), line("Kosilo 12 €", 1)])).toBe("Gostilna\nKosilo 12 €");
  });

  it("cuts a long search label to 32 characters", () => {
    expect(searchLabel("  Hotel   Union  ")).toBe("Hotel Union");
    const long = searchLabel("a".repeat(40));
    expect(long.endsWith("…")).toBe(true);
    expect(long).toHaveLength(33);
  });

  it("finds the Has text tag among a page's tags", () => {
    expect(hasTextTag([])).toBe(false);
    expect(hasTextTag([HAS_TEXT_TAG])).toBe(true);
    expect(HAS_TEXT_TAG.value).toEqual({ type: "hasText" });
  });

  it("gives the shown photo's aspect after a turn and a crop", () => {
    const r = (p: Partial<Recipe>) => ({ rotate: 0, crop: null, straighten: 0, flip_h: false, flip_v: false, ...p }) as Recipe;
    expect(shownAspectOf(1.5, null)).toBe(1.5);
    expect(shownAspectOf(1.5, r({ rotate: 90 }))).toBeCloseTo(1 / 1.5);
    expect(shownAspectOf(2, r({ crop: { x: 0, y: 0, w: 0.5, h: 1 } }))).toBeCloseTo(1);
    expect(shownAspectOf(2, r({ rotate: 270, crop: { x: 0, y: 0, w: 1, h: 0.5 } }))).toBeCloseTo(1);
  });

  describe("joinSentences", () => {
    /** A level line `h` tall whose top is at `y`, as the quad the model returns. */
    const at = (text: string, y: number, h = 0.05) => ({ text, quad: [0.1, y, 0.9, y, 0.9, y + h, 0.1, y + h] as ShownTextLine["quad"] });

    it("runs lines on until one ends a sentence", () => {
      const lines = [at("Gostilna pri", 0.1), at("Mostu je odprta.", 0.16), at("Kosilo 12 €", 0.22)];
      expect(joinSentences(lines)).toEqual(["Gostilna pri Mostu je odprta.", "Kosilo 12 €"]);
    });

    it("breaks after ! and ? and …", () => {
      expect(joinSentences([at("Stop!", 0.1), at("Why?", 0.16), at("Wait…", 0.22), at("then go", 0.28)])).toEqual(["Stop!", "Why?", "Wait…", "then go"]);
    });

    it("breaks where a block ends: a gap taller than the line", () => {
      const lines = [at("Hotel Union", 0.1), at("Ljubljana", 0.16), at("ODPRTO", 0.5)];
      expect(joinSentences(lines)).toEqual(["Hotel Union Ljubljana", "ODPRTO"]);
    });

    it("rejoins a word split by a hyphen at the end of a line", () => {
      expect(joinSentences([at("Gostil-", 0.1), at("na pri Mostu", 0.16)])).toEqual(["Gostilna pri Mostu"]);
    });

    it("keeps a hyphen before a capital or a digit", () => {
      expect(joinSentences([at("Ljubljana-", 0.1), at("Bled 2024", 0.16)])).toEqual(["Ljubljana- Bled 2024"]);
    });

    it("ignores blank lines, and gives nothing for no text", () => {
      expect(joinSentences([])).toEqual([]);
      expect(joinSentences([at("  ", 0.1), at("A", 0.16)])).toEqual(["A"]);
    });
  });
});
