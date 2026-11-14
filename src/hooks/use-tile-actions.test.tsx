import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { useTileActions } from "./use-tile-actions";

describe("rotating from the grid", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("rotate_assets", (a) => (a?.ids as number[]).map((id) => ({ assetId: id, recipe: null })));
    mockCommand("restore_recipes", () => null);
  });

  it("turns the photos a quarter and offers to undo it", async () => {
    const { result } = renderHook(() => useTileActions(), { wrapper: Providers });
    await act(() => result.current.rotate([4, 5], 1));
    expect(invokeMock).toHaveBeenCalledWith("rotate_assets", { ids: [4, 5], dir: 1 });
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("Rotated 2 photos");
    act(() => t.action?.run());
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("restore_recipes", { previous: [{ assetId: 4, recipe: null }, { assetId: 5, recipe: null }] }),
    );
  });

  it("offers Rotate left and right in the context menu", () => {
    const { result } = renderHook(() => useTileActions(), { wrapper: Providers });
    const more = result.current.menuItemsFor([4], { favorite: false }).find((i) => "label" in i && i.label === "More");
    const labels = more && "children" in more ? more.children.map((i) => ("label" in i ? i.label : null)) : [];
    expect(labels).toContain("Rotate left");
    expect(labels).toContain("Rotate right");
  });

  it("moves photos to Hidden from the context menu, with an undo", async () => {
    mockCommand("move_to_vault", (a) => a?.ids);
    mockCommand("move_from_vault", (a) => a?.ids);
    const { result } = renderHook(() => useTileActions(), { wrapper: Providers });
    const more = result.current.menuItemsFor([4, 5], { favorite: false }).find((i) => "label" in i && i.label === "More");
    const item = more && "children" in more ? more.children.find((i) => "label" in i && i.label === "Move to Hidden") : undefined;
    expect(item).toBeDefined();
    await act(async () => {
      if (item && "onSelect" in item) item.onSelect();
    });
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_to_vault", { ids: [4, 5] }));
    const t = toastStore.get().toasts.at(-1)!;
    expect(t.message).toBe("Moved 2 photos to Hidden");
    act(() => t.action?.run());
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_from_vault", { ids: [4, 5] }));
  });
});
