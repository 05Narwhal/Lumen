import { describe, expect, it } from "vitest";
import { notesFor } from "./release.mjs";

describe("notesFor", () => {
  const log = "# Changelog\n\n## 26.1.1-alpha\n\n- Faster grid.\n- Fixes.\n\n## 26.1.0-alpha\n\n- First alpha.\n";

  it("takes this version's section and nothing after it", () => {
    expect(notesFor(log, "26.1.1-alpha")).toBe("- Faster grid.\n- Fixes.");
    expect(notesFor(log, "26.1.0-alpha")).toBe("- First alpha.");
  });

  it("is empty for a version with no entry", () => {
    expect(notesFor(log, "26.2.0-alpha")).toBe("");
  });
});
