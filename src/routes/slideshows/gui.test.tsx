import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { toastStore } from "@/components/toast/store";
import { selectionStore } from "@/hooks/use-selection";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { MemoriesRoute } from "@/routes/memories";

const show = (id: number, name: string) => ({
  id,
  name,
  clipCount: 3,
  durationMs: 10_800,
  coverAssetId: 4,
  updatedAt: Math.floor(Date.now() / 1000) - 86_400 * 3,
});

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/memories?tab=slideshows"]}>
        <Routes>
          <Route
            path="/memories"
            element={
              <>
                <MemoriesRoute />
                <Where />
              </>
            }
          />
          <Route path="/slideshows/:id" element={<Where />} />
        </Routes>
      </MemoryRouter>
      <ContextMenuHost />
    </Providers>,
  );
}

beforeEach(() => {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  toastStore.set({ toasts: [] });
  selectionStore.set({ ids: new Set(), anchor: null });
});


/**
 * These two tests open a Mantine menu, and on a loaded run they fail roughly one time in
 * five: the dropdown's item is in the DOM but its `display: none` has not lifted. The menu
 * needs the full pointer sequence to open — a bare `fireEvent.click` never opens it at all —
 * and the row it hangs off re-renders as the queries settle, which unmounts an open menu.
 * Waiting on `aria-expanded`, or clicking again, both made it worse, because the captured
 * trigger node is the one that was replaced. A generous timeout is the best of the options
 * tried; the flake is in the widget's mechanics under jsdom, not in what is being tested.
 */
describe("Slideshows tab of Memories", () => {
  it("lists slideshows with their length", async () => {
    mockCommand("list_slideshows", () => [show(1, "Trip to Lisbon"), show(2, "On this day")]);
    mount();
    expect(await screen.findByRole("link", { name: /Trip to Lisbon/ })).toHaveAttribute("href", "/slideshows/1");
    expect(screen.getAllByText(/3 photos · 0:11 · edited 3 days ago/)).toHaveLength(2);
  });

  it("offers one action when there is nothing yet", async () => {
    mockCommand("list_slideshows", () => []);
    mount();
    expect(await screen.findByText("No slideshows yet")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "New slideshow" }).length).toBeGreaterThan(0);
  });

  it("makes a slideshow from the photos picked and opens it", async () => {
    mockCommand("list_slideshows", () => []);
    mockCommand("create_slideshow", () => 12);
    mount();
    await userEvent.click((await screen.findAllByRole("button", { name: "New slideshow" }))[0]);
    selectionStore.set({ ids: new Set([5, 6, 7]), anchor: 7 });
    await userEvent.click(await screen.findByRole("button", { name: /Make slideshow/ }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_slideshow", { name: "New slideshow", assetIds: [5, 6, 7] }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/slideshows/12"));
  });

  it("makes one from a memory", async () => {
    mockCommand("list_slideshows", () => []);
    mockCommand("list_memories", () => [
      { id: 3, kind: 0, title: "Trip to Lisbon", subtitle: "12–19 March 2024", startAt: 1, endAt: 2, coverAssetId: 4, itemCount: 40, curatedCount: 10, hidden: false },
    ]);
    mockCommand("create_slideshow_from_memory", () => 20);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "From memory" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /Trip to Lisbon/ }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_slideshow_from_memory", { memoryId: 3 }));
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/slideshows/20"));
  });

  it("deletes a slideshow with Undo", async () => {
    mockCommand("list_slideshows", () => [show(1, "Trip to Lisbon")]);
    mockCommand("delete_slideshow", () => ({ id: 1, name: "Trip to Lisbon", doc: { version: 1, resolution: [1920, 1080], fps: 30, audio: null, clips: [] } }));
    mockCommand("create_slideshow", () => 9);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "More for Trip to Lisbon" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: /Delete/ }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("delete_slideshow", { id: 1 }));
    // Hold on to the action as soon as it exists: a toast dismisses itself after six
    // seconds, and reading the store a second time is a race on a slow run.
    let undo: (() => void) | undefined;
    await waitFor(() => {
      const toast = toastStore.get().toasts.at(-1);
      expect(toast?.message).toBe("Slideshow deleted");
      undo = toast?.action?.run;
      expect(undo).toBeTypeOf("function");
    });

    undo?.();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_slideshow", expect.objectContaining({ name: "Trip to Lisbon" })));
  });

  it("exports a slideshow from its right-click menu and its ⋯ button", async () => {
    mockCommand("list_slideshows", () => [show(1, "Trip to Lisbon")]);
    vi.mocked(openDialog).mockResolvedValueOnce("D:/Films").mockResolvedValueOnce("E:/Out");
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: await screen.findByText("Trip to Lisbon") });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Export…" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_slideshow", { id: 1, destDir: "D:/Films" }));
    await userEvent.click(await screen.findByRole("button", { name: "More for Trip to Lisbon" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Export…" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_slideshow", { id: 1, destDir: "E:/Out" }));
  });

  it("duplicates a slideshow from its row's right-click menu, with Undo", async () => {
    const doc = { version: 1, resolution: [1920, 1080], fps: 30, audio: null, clips: [{ asset_id: 4, extra: [5] }] };
    mockCommand("list_slideshows", () => [show(1, "Trip to Lisbon")]);
    mockCommand("get_slideshow", () => ({ id: 1, name: "Trip to Lisbon", doc }));
    mockCommand("create_slideshow", () => 7);
    mockCommand("save_slideshow", () => null);
    mockCommand("delete_slideshow", () => null);
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: await screen.findByText("Trip to Lisbon") });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Duplicate" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("create_slideshow", { name: "Trip to Lisbon copy", assetIds: [4, 5], arrange: false }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("save_slideshow", { id: 7, name: "Trip to Lisbon copy", doc }));
    let undo: (() => void) | undefined;
    await waitFor(() => {
      const toast = toastStore.get().toasts.at(-1);
      expect(toast?.message).toBe("Made “Trip to Lisbon copy”");
      undo = toast?.action?.run;
    });
    undo?.();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("delete_slideshow", { id: 7 }));
  });
});
