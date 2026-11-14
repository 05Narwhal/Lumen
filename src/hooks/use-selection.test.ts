import { waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { clearSelection, selectAll, selectionStore, selectOnly, selectRange, toggleSelection } from "./use-selection";

describe("selection store", () => {
  beforeEach(() => clearSelection());

  it("toggles and tracks the anchor", () => {
    toggleSelection(3);
    toggleSelection(5);
    expect([...selectionStore.get().ids]).toEqual([3, 5]);
    expect(selectionStore.get().anchor).toBe(5);
    toggleSelection(3);
    expect([...selectionStore.get().ids]).toEqual([5]);
  });

  it("extends a range from the anchor in grid order, both directions", () => {
    const order = [10, 20, 30, 40, 50];
    selectOnly(20);
    selectRange(40, order);
    expect([...selectionStore.get().ids].sort()).toEqual([20, 30, 40]);
    selectRange(10, order);
    expect([...selectionStore.get().ids].sort((a, b) => a - b)).toEqual([10, 20, 30, 40]);
    expect(selectionStore.get().anchor).toBe(20);
  });

  // Round 5: the grid only holds the rows it has loaded; a range reaching beyond them asks
  // Rust for the whole run from the cached layout.
  it("asks the layout for a range that reaches beyond the loaded rows", async () => {
    const layout = { width: 1000, rowHeight: 200, gap: 4, scope: { albumId: 3 }, groupBy: "month" };
    mockCommand("layout_range_ids", () => [20, 25, 30, 35, 9000]);
    selectOnly(20);
    selectRange(9000, [10, 20, 30], layout);
    await waitFor(() => expect(selectionStore.get().ids.size).toBe(5));
    expect(invokeMock).toHaveBeenCalledWith("layout_range_ids", { ...layout, from: 20, to: 9000 });
    expect(selectionStore.get().anchor).toBe(20);
  });

  it("selects the target alone when there is no anchor and clears", () => {
    selectRange(30, [10, 20, 30]);
    expect([...selectionStore.get().ids]).toEqual([30]);
    selectAll([1, 2, 3]);
    expect(selectionStore.get().ids.size).toBe(3);
    clearSelection();
    expect(selectionStore.get().ids.size).toBe(0);
    expect(selectionStore.get().anchor).toBeNull();
  });
});
