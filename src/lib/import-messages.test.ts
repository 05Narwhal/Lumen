import { describe, expect, it } from "vitest";
import type { ImportSummary } from "@/types/generated/ImportSummary";
import { importButtonLabel, importFinishedMessage, interruptedImports, noteImportFinished } from "./import-messages";

const s = (x: Partial<ImportSummary>): ImportSummary => ({ batchId: 1, imported: 0, skipped: 0, failed: 0, cancelled: false, interrupted: false, label: "Julian's iPhone", skipRecords: 0, ...x });

describe("importFinishedMessage", () => {
  it("counts what happened", () => {
    expect(importFinishedMessage(s({ imported: 1204, skipped: 3 }))).toBe("Import finished: 1,204 imported, 3 skipped");
    expect(importFinishedMessage(s({ imported: 2, cancelled: true }))).toBe("Import cancelled: 2 imported");
  });

  it("says a device was unplugged and how to finish, not that a file failed", () => {
    expect(importFinishedMessage(s({ imported: 1204, failed: 1, interrupted: true }))).toBe(
      "Julian's iPhone was disconnected after 1,204 imported. Plug it back in to import the rest.",
    );
  });
});

describe("resuming an interrupted import", () => {
  it("offers to resume a device whose last import was cut short, until one completes", () => {
    interruptedImports.set(new Set());
    expect(importButtonLabel(1106, false)).toBe("Import 1,106 new");
    noteImportFinished(s({ interrupted: true }));
    expect(interruptedImports.get().has("Julian's iPhone")).toBe(true);
    expect(importButtonLabel(1106, interruptedImports.get().has("Julian's iPhone"))).toBe("Resume import (1,106 left)");
    noteImportFinished(s({ imported: 1106 }));
    expect(interruptedImports.get().has("Julian's iPhone")).toBe(false);
    expect(importButtonLabel(0, false)).toBe("Up to date");
  });
});
