import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Providers } from "@/app";
import { clearSelection, selectAll, selectionStore } from "@/hooks/use-selection";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { SelectionBar } from "./gui";

describe("SelectionBar", () => {
  it("appears with the count and clears on Deselect", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    clearSelection();
    render(
      <Providers>
        <MemoryRouter>
          <SelectionBar />
        </MemoryRouter>
      </Providers>,
    );
    expect(screen.queryByRole("toolbar")).toBeNull();
    act(() => selectAll([1, 2]));
    // By label, not text: a narrow window hides the word "selected" and keeps the number.
    expect(await screen.findByLabelText("2 selected")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Explorer|Finder/ })).toBeNull();
    act(() => selectAll([5]));
    expect(await screen.findByRole("button", { name: /Explorer|Finder/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Trash" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("trash_assets", { ids: [5] }));
    await waitFor(() => expect(screen.queryByRole("toolbar")).toBeNull());
  });

  it("offers Move to Hidden in the library and Move to Library in the Hidden folder", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("move_from_vault", (a) => a?.ids);
    clearSelection();
    const { unmount } = render(
      <Providers>
        <MemoryRouter initialEntries={["/library"]}>
          <SelectionBar />
        </MemoryRouter>
      </Providers>,
    );
    act(() => selectAll([1, 2]));
    expect(await screen.findByRole("button", { name: "Move to Hidden" })).toBeInTheDocument();
    unmount();
    clearSelection();
    render(
      <Providers>
        <MemoryRouter initialEntries={["/hidden"]}>
          <SelectionBar />
        </MemoryRouter>
      </Providers>,
    );
    act(() => selectAll([3]));
    await userEvent.click(await screen.findByRole("button", { name: "Move to Library" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("move_from_vault", { ids: [3] }));
    expect(screen.queryByRole("button", { name: "Move to Hidden" })).toBeNull();
  });

  function mountBar(path = "/library") {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    clearSelection();
    render(
      <Providers>
        <MemoryRouter initialEntries={[path]}>
          <SelectionBar />
        </MemoryRouter>
      </Providers>,
    );
  }

  it("keeps the selection after a rotate, so a second press makes a half turn", async () => {
    mountBar();
    mockCommand("rotate_assets", () => [{ assetId: 1, recipe: null }]);
    act(() => selectAll([1, 2]));
    await userEvent.click(await screen.findByRole("button", { name: "Rotate right" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("rotate_assets", expect.anything()));
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("clears the selection after opening it in Develop", async () => {
    mountBar();
    act(() => selectAll([1, 2]));
    await userEvent.click(await screen.findByRole("button", { name: "Develop" }));
    await waitFor(() => expect(selectionStore.get().ids.size).toBe(0));
  });

  it("keeps the selection after Favorite, so it can be marked again", async () => {
    mountBar();
    act(() => selectAll([1, 2]));
    await userEvent.click(await screen.findByRole("button", { name: "Favorite" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_favorite", expect.anything()));
    expect(selectionStore.get().ids.size).toBe(2);
  });

  it("clears the selection after copying the files out", async () => {
    mountBar();
    vi.mocked(openDialog).mockResolvedValueOnce("C:/Out");
    mockCommand("copy_assets_to", () => 2);
    act(() => selectAll([1, 2]));
    await userEvent.click(await screen.findByRole("button", { name: "Copy files to…" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("copy_assets_to", { ids: [1, 2], dir: "C:/Out" }));
    await waitFor(() => expect(selectionStore.get().ids.size).toBe(0));
  });
});
