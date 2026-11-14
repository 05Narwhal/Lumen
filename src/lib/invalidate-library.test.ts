import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { invalidateLibraryData } from "./events";

describe("invalidateLibraryData", () => {
  it("refetches Develop's history, so a snapshot an AI assistant saved shows without reopening the photo", async () => {
    const qc = new QueryClient();
    qc.setQueryData(["develop-history", 7], []);
    await invalidateLibraryData(qc);
    expect(qc.getQueryState(["develop-history", 7])?.isInvalidated).toBe(true);
  });
});
