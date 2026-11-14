import { beforeEach, describe, expect, it } from "vitest";
import { clearViewerSequence, dropFromViewerSequence, sequenceNeighbors, setViewerSequence } from "./viewer-sequence";

describe("viewer sequence", () => {
  beforeEach(() => clearViewerSequence());

  it("pages only through the photos it was opened with", () => {
    setViewerSequence([4, 9, 2]);
    expect(sequenceNeighbors(4)).toEqual({ prev: null, next: 9, index: 0, total: 3 });
    expect(sequenceNeighbors(9)).toEqual({ prev: 4, next: 2, index: 1, total: 3 });
    expect(sequenceNeighbors(2)).toEqual({ prev: 9, next: null, index: 2, total: 3 });
  });

  it("does not apply to a photo outside it, so the timeline takes over", () => {
    setViewerSequence([4, 9]);
    expect(sequenceNeighbors(77)).toBeNull();
    clearViewerSequence();
    expect(sequenceNeighbors(4)).toBeNull();
  });

  it("closes the gap when a photo in it is trashed", () => {
    setViewerSequence([4, 9, 2]);
    dropFromViewerSequence(9);
    expect(sequenceNeighbors(4)?.next).toBe(2);
    expect(sequenceNeighbors(2)?.total).toBe(2);
  });
});
