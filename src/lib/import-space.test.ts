import { describe, expect, it } from "vitest";
import type { ImportPlan } from "@/types/generated/ImportPlan";
import { roomQuestion } from "./import-space";

const plan = (x: Partial<ImportPlan>): ImportPlan => ({
  count: 2310,
  bytes: 52_000_000_000,
  freeBytes: 61_200_000_000,
  fitsCount: 2310,
  fitsBytes: 52_000_000_000,
  buffer: 20_000_000_000,
  ...x,
});

describe("roomQuestion", () => {
  it("asks nothing when everything fits", () => {
    expect(roomQuestion(plan({}))).toEqual({ kind: "all" });
    expect(roomQuestion(plan({ freeBytes: null }))).toEqual({ kind: "all" });
  });

  it("offers the newest that fit, saying how much and why", () => {
    const q = roomQuestion(plan({ fitsCount: 1850, fitsBytes: 41_200_000_000 }));
    expect(q).toEqual({
      kind: "some",
      title: "Not enough space for everything",
      body: "1,850 of 2,310 items (41 GB) fit and keep 20 GB free on the library's drive. The newest come first; the rest wait until you free up space.",
      confirmLabel: "Import 1,850",
      limitBytes: 41_200_000_000,
    });
  });

  it("says there is no room when nothing fits", () => {
    expect(roomQuestion(plan({ freeBytes: 18_000_000_000, fitsCount: 0, fitsBytes: 0 }))).toEqual({
      kind: "none",
      message: "There is no room to import while keeping 20 GB free on the library's drive (18 GB free).",
    });
  });
});
