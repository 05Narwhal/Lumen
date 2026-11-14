import { describe, expect, it } from "vitest";
import type { RepairResult } from "@/types/generated/RepairResult";
import { repairImportedMessage } from "./repair-progress";

const result = (r: Partial<RepairResult>): RepairResult => ({
  imported: 0,
  repaired: 0,
  sidecars: 0,
  skipped: 0,
  batchId: null,
  repairedIds: [],
  notes: [],
  outcomes: [],
  cancelled: false,
  ...r,
});

describe("repairImportedMessage", () => {
  it("names all four counts in the spec's order", () => {
    expect(repairImportedMessage(result({ imported: 1204, repaired: 3, sidecars: 1 }))).toBe("1,204 imported · 3 repaired · 1 sidecar · 0 skipped");
  });

  it("says when it stopped early", () => {
    expect(repairImportedMessage(result({ imported: 2, cancelled: true }))).toBe("Stopped · 2 imported · 0 repaired · 0 sidecars · 0 skipped");
  });
});
