import { describe, expect, it } from "vitest";
import { DEFAULT_DEVELOP, hasGeometry, mergePanels, signed, ZERO_ADJUST } from "./develop";

describe("mergePanels", () => {
  it("takes exactly the chosen panels, as Rust does", () => {
    const mine = { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, contrast: 0.3 }, auto: true };
    const warm = { ...DEFAULT_DEVELOP, basic: { ...DEFAULT_DEVELOP.basic, exposure: 0.5 }, effects: { ...DEFAULT_DEVELOP.effects, grain: { amount: 0.4, size: 0.2 } } };
    const effects = mergePanels(mine, warm, ["effects"]);
    expect(effects.basic.contrast).toBe(0.3);
    expect(effects.auto).toBe(true);
    expect(effects.effects.grain.amount).toBe(0.4);
    const basic = mergePanels(mine, warm, ["basic"]);
    expect([basic.basic.exposure, basic.basic.contrast, basic.auto]).toEqual([0.5, 0, false]);
    expect(basic.effects).toEqual(DEFAULT_DEVELOP.effects);
  });

  it("treats masks as a panel of their own", () => {
    const mask = { id: 1, name: "Sky", shapes: [], invert: false, opacity: 1, visible: true, adjust: { ...ZERO_ADJUST, exposure: 1 } };
    const theirs = { ...DEFAULT_DEVELOP, masks: [mask], effects: { ...DEFAULT_DEVELOP.effects, grain: { amount: 0.4, size: 0.2 } } };
    const m = mergePanels(DEFAULT_DEVELOP, theirs, ["masks"]);
    expect(m.masks).toEqual([mask]);
    expect(m.effects).toEqual(DEFAULT_DEVELOP.effects);
    expect(mergePanels(DEFAULT_DEVELOP, theirs, ["effects"]).masks ?? []).toEqual([]);
  });
});

describe("signed", () => {
  it("writes a real minus sign and no sign for zero", () => {
    expect(signed(12)).toBe("+12");
    expect(signed(-0.304, 2)).toBe("−0.30");
    expect(signed(-0.001, 2)).toBe("0.00");
  });
});

describe("outputSize", () => {
  it("turns and crops as Rust does", async () => {
    const { outputSize } = await import("./develop");
    const { IDENTITY } = await import("@/hooks/use-edit-session");
    expect(outputSize(IDENTITY, 6000, 4000)).toEqual([6000, 4000]);
    expect(outputSize({ ...IDENTITY, rotate: 90 }, 6000, 4000)).toEqual([4000, 6000]);
    expect(outputSize({ ...IDENTITY, rotate: 270, crop: { x: 0, y: 0, w: 0.5, h: 0.25 } }, 6000, 4000)).toEqual([2000, 1500]);
  });
});

describe("hasGeometry", () => {
  it("is true for a crop, a turn, a flip or a straighten and false for the identity", async () => {
    const { IDENTITY } = await import("@/hooks/use-edit-session");
    expect(hasGeometry(IDENTITY)).toBe(false);
    expect(hasGeometry({ ...IDENTITY, crop: { x: 0, y: 0, w: 0.5, h: 0.5 } })).toBe(true);
    expect(hasGeometry({ ...IDENTITY, rotate: 90 })).toBe(true);
    expect(hasGeometry({ ...IDENTITY, straighten: 1.5 })).toBe(true);
    expect(hasGeometry({ ...IDENTITY, flip_h: true })).toBe(true);
    expect(hasGeometry({ ...IDENTITY, flip_v: true })).toBe(true);
  });
});
