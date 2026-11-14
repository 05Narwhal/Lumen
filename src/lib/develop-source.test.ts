import { beforeEach, describe, expect, it, vi } from "vitest";
import { addToDevelop, clearDevelop, DEFAULT_DEVELOP_SOURCE, developSourceStore, isEmptySource, openDevelop, removeFromDevelop, restoreDevelop, withFilters } from "./develop-source";

describe("develop source", () => {
  beforeEach(() => developSourceStore.set(DEFAULT_DEVELOP_SOURCE));

  it("starts empty, for the user to choose photos, in capture order", () => {
    expect(developSourceStore.get()).toMatchObject({ only: [], sort: "captured" });
    expect(isEmptySource(developSourceStore.get())).toBe(true);
  });

  it("adds chosen photos without repeating any", () => {
    addToDevelop([3, 1]);
    addToDevelop([1, 7]);
    expect(developSourceStore.get()).toMatchObject({ only: [3, 1, 7], label: "Selection" });
  });

  it("opens on a selection at a photo, keeping the chosen sort", () => {
    developSourceStore.set((s) => ({ ...s, sort: "rating" }));
    const navigate = vi.fn();
    openDevelop(navigate, { only: [4, 2, 9], label: "Selection" }, 2);
    expect(navigate).toHaveBeenCalledWith("/develop/2");
    expect(developSourceStore.get()).toMatchObject({ only: [4, 2, 9], label: "Selection", sort: "rating" });
    openDevelop(navigate);
    expect(navigate).toHaveBeenLastCalledWith("/develop");
    expect(isEmptySource(developSourceStore.get()), "nothing chosen: Develop asks").toBe(true);
    // Opened on one photo with no set: just that photo, never the whole library.
    openDevelop(navigate, {}, 5);
    expect(developSourceStore.get()).toMatchObject({ only: [5] });
  });

  it("applies the top bar's filters on the scope's query", () => {
    const s = withFilters(DEFAULT_DEVELOP_SOURCE, { flag: 1, colorLabels: [3] });
    expect(s.scope.query).toMatchObject({ flag: 1, colorLabels: [3], text: null });
    expect(withFilters(s, { flag: null }).scope.query).toMatchObject({ flag: null, colorLabels: [3] });
  });

  it("clears the filmstrip back to choosing photos, and Undo brings the same ones back", () => {
    developSourceStore.set((s) => ({ ...s, only: [4, 2, 9], label: "Selection", sort: "rating" }));
    const prev = clearDevelop();
    expect(isEmptySource(developSourceStore.get())).toBe(true);
    expect(developSourceStore.get().sort).toBe("rating");
    restoreDevelop(prev);
    expect(developSourceStore.get()).toMatchObject({ only: [4, 2, 9], label: "Selection" });
  });

  it("removes photos from the strip, keeping the order of the rest", () => {
    developSourceStore.set((s) => ({ ...s, only: [1, 2, 3], label: "Selection" }));
    const prev = removeFromDevelop([2], [1, 2, 3]);
    expect(developSourceStore.get().only).toEqual([1, 3]);
    expect(prev.only).toEqual([1, 2, 3]);
  });

  it("removing from a whole album keeps the others as an explicit list", () => {
    developSourceStore.set((s) => ({ ...s, only: null, label: "Trip" }));
    removeFromDevelop([5], [4, 5, 6]);
    expect(developSourceStore.get()).toMatchObject({ only: [4, 6], label: "Trip" });
  });
});
