import { describe, expect, it } from "vitest";
import { DEFAULT_DEVELOP, ZERO_ADJUST, type CopiedSettings } from "./develop";
import { MAX_CLIPBOARD_CHARS, parseSettings, serializeSettings } from "./develop-clipboard";
import type { Mask } from "@/types/generated/Mask";
import type { MaskShape } from "@/types/generated/MaskShape";

const mask = (id: number, shapes: MaskShape[]): Mask => ({ id, name: `Mask ${id}`, shapes, invert: false, opacity: 1, adjust: ZERO_ADJUST, visible: true });
const linear: MaskShape = { kind: "linear", add: true, from: [0.1, 0.2], to: [0.3, 0.9] };
const radial: MaskShape = { kind: "radial", add: true, center: [0.5, 0.5], radius: [0.2, 0.3], angle: 0, feather: 0.4, inside: true };
const brush: MaskShape = { kind: "brush", add: true, radius: 0.05, feather: 0.5, flow: 1, points: [[0.1, 0.1]] };
const ai: MaskShape = { kind: "ai", add: true, target: "person", instance: 1, raster: "abc123" };

const settings = (over: Partial<CopiedSettings["develop"]> = {}): CopiedSettings => ({
  develop: { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, exposure: 0.7, contrast: -0.25 }, ...over },
  panels: ["basic", "curve", "masks"],
});

describe("develop clipboard", () => {
  it("round_trip", () => {
    const s = settings({ masks: [mask(1, [linear, radial])] });
    const out = parseSettings(serializeSettings(s, "26.2.0-alpha"));
    expect(out?.settings).toEqual(s);
    expect(out).toMatchObject({ droppedBrush: 0, newer: false });
  });

  it("writes the envelope readable", () => {
    const text = serializeSettings(settings(), "26.2.0-alpha");
    expect(text).toContain('\n  "lumen": "develop-settings"');
    expect(JSON.parse(text)).toMatchObject({ lumen: "develop-settings", version: 1, app: "26.2.0-alpha" });
  });

  it("clamps_out_of_range", () => {
    const s = settings();
    s.develop.basic.exposure = 99;
    s.develop.basic.temperature = -500;
    s.develop.basic.contrast = 3;
    s.develop.masks = [{ ...mask(1, [linear]), opacity: 4, adjust: { ...ZERO_ADJUST, exposure: -40 } }];
    const out = parseSettings(serializeSettings(s, "x"))?.settings.develop;
    expect(out?.basic.exposure).toBe(5);
    expect(out?.basic.temperature).toBe(-100);
    expect(out?.basic.contrast).toBe(1);
    expect(out?.masks?.[0].opacity).toBe(1);
    expect(out?.masks?.[0].adjust.exposure).toBe(-5);
  });

  it("drops_brush_masks", () => {
    const s = settings({ masks: [mask(1, [brush]), mask(2, [linear, brush])] });
    const out = parseSettings(serializeSettings(s, "x"));
    expect(out?.droppedBrush).toBe(2);
    expect(out?.settings.develop.masks?.map((m) => [m.id, m.shapes.length])).toEqual([[2, 1]]);
  });

  it("keeps_shapes_and_ai_kinds", () => {
    const s = settings({ masks: [mask(1, [linear, radial, ai])] });
    const out = parseSettings(serializeSettings(s, "x"))?.settings.develop.masks?.[0].shapes;
    expect(out?.[0]).toEqual(linear);
    expect(out?.[1]).toEqual(radial);
    // An AI mask travels as its kind; the raster belongs to the photo it was found on.
    expect(out?.[2]).toMatchObject({ kind: "ai", target: "person", instance: 1, raster: "" });
  });

  it("newer_version_flagged", () => {
    const doc = JSON.parse(serializeSettings(settings(), "x"));
    doc.version = 7;
    doc.settings.develop.somethingNew = { a: 1 };
    const out = parseSettings(JSON.stringify(doc));
    expect(out?.newer).toBe(true);
    expect(out?.settings.develop.basic.exposure).toBe(0.7);
    expect(out?.settings.develop).not.toHaveProperty("somethingNew");
  });

  it("foreign_json_ignored", () => {
    expect(parseSettings('{"a":1}')).toBeNull();
    expect(parseSettings("[1,2]")).toBeNull();
    expect(parseSettings("not json")).toBeNull();
    expect(parseSettings("")).toBeNull();
    expect(parseSettings(JSON.stringify({ lumen: "develop-settings", version: 1 }))).toBeNull();
    expect(parseSettings(JSON.stringify({ lumen: "develop-settings", version: 1, settings: { develop: "no" } }))).toBeNull();
  });

  it("rejects_over_1mb", () => {
    const big = serializeSettings(settings(), "x").replace('"app": "x"', `"app": "${"x".repeat(MAX_CLIPBOARD_CHARS)}"`);
    expect(parseSettings(big)).toBeNull();
  });

  it("falls back to every panel when none are listed", () => {
    const doc = JSON.parse(serializeSettings(settings(), "x"));
    doc.settings.panels = ["nonsense"];
    expect(parseSettings(JSON.stringify(doc))?.settings.panels).toEqual(["basic", "curve", "hsl", "grading", "detail", "effects"]);
  });
});
