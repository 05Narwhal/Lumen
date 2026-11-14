import { describe, expect, it } from "vitest";
import { DEFAULT_DEVELOP } from "@/lib/develop";
import type { RetouchResult } from "@/types/generated/RetouchResult";
import { retouchMessage } from "./retouch";

const result = (r: Partial<RetouchResult>): RetouchResult => ({ develop: DEFAULT_DEVELOP, crop: null, straighten: 0, usedModel: true, summary: [], model: null, ...r });

describe("retouchMessage", () => {
  it("names what the model changed", () => {
    expect(retouchMessage(result({ summary: ["tone", "colour", "crop"] }))).toBe("Retouch — tone, colour, crop");
    expect(retouchMessage(result({}))).toBe("Retouch");
  });

  it("names the model when it has a name", () => {
    expect(retouchMessage(result({ model: "Lumen Ray-high 1.0", summary: ["tone"] }))).toBe("Retouch with Lumen Ray-high 1.0 — tone");
    expect(retouchMessage(result({ model: "Lumen Ray-high 1.0" }))).toBe("Retouch with Lumen Ray-high 1.0");
  });

  it("says when it fell back to Auto", () => {
    expect(retouchMessage(result({ usedModel: false, model: null }))).toBe("Retouch model isn't ready — used Auto instead.");
  });
});
