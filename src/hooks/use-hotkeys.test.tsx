import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { paletteStore } from "@/components/command-palette/store";
import { toast } from "@/components/toast";
import { toastStore } from "@/components/toast/store";
import { noteRadarGrid } from "@/lib/radar-view";
import type { ViewerScope } from "@/lib/viewer-scope";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { clearSelection, selectionStore } from "./use-selection";
import { useGlobalHotkeys } from "./use-hotkeys";

function Probe() {
  useGlobalHotkeys();
  const l = useLocation();
  return <p data-testid="path">{l.pathname}</p>;
}

describe("useGlobalHotkeys", () => {
  it("navigates with digits and opens the palette with ctrl+k", async () => {
    mockCommand("get_settings", () => null);
    paletteStore.set({ open: false });
    render(
      <Providers>
        <MemoryRouter>
          <Routes>
            <Route path="*" element={<Probe />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.keyboard("3");
    expect(screen.getByTestId("path")).toHaveTextContent("/albums");
    await userEvent.keyboard("{Control>}k{/Control}");
    expect(paletteStore.get().open).toBe(true);
  });

  it("ignores digits typed into inputs", async () => {
    mockCommand("get_settings", () => null);
    render(
      <Providers>
        <MemoryRouter>
          <Routes>
            <Route
              path="*"
              element={
                <>
                  <Probe />
                  <input aria-label="x" />
                </>
              }
            />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    await userEvent.click(screen.getByRole("textbox", { name: "x" }));
    await userEvent.keyboard("2");
    expect(screen.getByTestId("path")).toHaveTextContent("/");
  });

  // Round 4J: in Develop, Ctrl+= / Ctrl+− / Ctrl+0 zoom the photo, not the library's tiles.
  it("leaves Ctrl+=, Ctrl+− and Ctrl+0 to Develop", () => {
    mockCommand("get_settings", () => null);
    const at = (path: string) =>
      render(
        <Providers>
          <MemoryRouter initialEntries={[path]}>
            <Routes>
              <Route path="*" element={<Probe />} />
            </Routes>
          </MemoryRouter>
        </Providers>,
      );
    const press = (key: string) => {
      const e = new KeyboardEvent("keydown", { key, ctrlKey: true, cancelable: true, bubbles: true });
      window.dispatchEvent(e);
      return e.defaultPrevented;
    };
    const { unmount } = at("/develop/3");
    expect(["=", "-", "0"].map(press)).toEqual([false, false, false]);
    unmount();
    at("/library");
    expect(press("=")).toBe(true);
  });

  it("turns the selection with ctrl+[ and ctrl+]", async () => {
    mockCommand("get_settings", () => null);
    mockCommand("rotate_assets", (a) => (a?.ids as number[]).map((id) => ({ assetId: id, recipe: null })));
    invokeMock.mockClear();
    render(
      <Providers>
        <MemoryRouter>
          <Routes>
            <Route path="*" element={<Probe />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    selectionStore.set({ ids: new Set([7, 8]), anchor: 7 });
    await userEvent.keyboard("{Control>}]{/Control}");
    expect(invokeMock).toHaveBeenCalledWith("rotate_assets", { ids: [7, 8], dir: 1 });
    await userEvent.keyboard("{Control>}[[{/Control}");
    expect(invokeMock).toHaveBeenCalledWith("rotate_assets", { ids: [7, 8], dir: -1 });
    clearSelection();
  });

  // Round 10: keys that act on where the user is.
  function mountAt(path: string) {
    mockCommand("get_settings", () => null);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/", path]}>
          <Routes>
            <Route path="*" element={<Probe />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
  }

  it("selects every photo of the grid on screen with ctrl+a, not only the loaded rows, and none with ctrl+d", async () => {
    const grid: ViewerScope = { width: 900, rowHeight: 200, gap: 4, scope: {} as ViewerScope["scope"], groupBy: "month" };
    mockCommand("set_face_radar_view", () => null);
    mockCommand("layout_ids", () => Array.from({ length: 5000 }, (_, i) => i + 1));
    noteRadarGrid(grid);
    clearSelection();
    mountAt("/library");
    await userEvent.keyboard("{Control>}a{/Control}");
    await vi.waitFor(() => expect(selectionStore.get().ids.size).toBe(5000));
    expect(invokeMock).toHaveBeenCalledWith("layout_ids", grid);
    await userEvent.keyboard("{Control>}d{/Control}");
    expect(selectionStore.get().ids.size).toBe(0);
    noteRadarGrid(null);
    await userEvent.keyboard("{Control>}a{/Control}");
    expect(selectionStore.get().ids.size).toBe(0);
  });

  it("undoes the newest action with ctrl+z", async () => {
    toastStore.set({ toasts: [] });
    const first = vi.fn();
    const second = vi.fn();
    toast({ message: "Moved to Trash", undo: first });
    toast({ message: "Added to Favorites", undo: second });
    mountAt("/library");
    await userEvent.keyboard("{Control>}z{/Control}");
    expect(second).toHaveBeenCalledOnce();
    expect(first).not.toHaveBeenCalled();
    expect(toastStore.get().toasts.map((t) => t.message)).toEqual(["Moved to Trash"]);
  });

  it("goes back and forward with alt+arrows", async () => {
    mountAt("/albums");
    await userEvent.keyboard("{Alt>}{ArrowLeft}{/Alt}");
    expect(screen.getByTestId("path")).toHaveTextContent(/^\/$/);
    await userEvent.keyboard("{Alt>}{ArrowRight}{/Alt}");
    expect(screen.getByTestId("path")).toHaveTextContent("/albums");
  });
});
