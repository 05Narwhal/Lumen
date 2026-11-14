import { describe, expect, it } from "vitest";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { AutoResult } from "@/types/generated/AutoResult";
import { autoMessage } from "./auto";

const result = (r: Partial<AutoResult>): AutoResult => ({ basic: DEFAULT_DEVELOP.basic, usedModel: true, scene: null, source: "classic", model: null, look: null, ...r });

describe("autoMessage", () => {
  it("reads the scene", () => {
    expect(autoMessage(result({ scene: "sunset" }))).toBe("Auto — read as a sunset");
    expect(autoMessage(result({}))).toBe("Auto");
  });

  it("names the trained model it started from", () => {
    const model = { source: "model", model: "Lumen Ray-high 1.0", look: { curve: DEFAULT_DEVELOP.curve, hsl: DEFAULT_DEVELOP.hsl, grading: DEFAULT_DEVELOP.grading } };
    expect(autoMessage(result({ ...model, scene: "sunset" }))).toBe("Auto with Lumen Ray-high 1.0 — read as a sunset");
    expect(autoMessage(result(model))).toBe("Auto with Lumen Ray-high 1.0");
    // The image model still downloading: the trained model's look was still applied.
    expect(autoMessage(result({ ...model, usedModel: false }))).toBe("Auto with Lumen Ray-high 1.0");
  });

  it("says when smart features are off", () => {
    expect(autoMessage(result({ usedModel: false }))).toBe("Auto measured the photo; smart features are off or still downloading.");
  });
});
