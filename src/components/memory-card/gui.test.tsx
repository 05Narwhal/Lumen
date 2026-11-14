import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { exportSheetStore } from "@/components/export-sheet";
import { toastStore } from "@/components/toast/store";
import { filmStore } from "@/lib/film";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { MemoryCard } from "./gui";

const memory = { id: 3, title: "Lisbon", subtitle: "May 2026", coverAssetId: null, hidden: false } as unknown as MemorySummary;

beforeEach(() => {
  filmStore.set(null);
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
});

function mount() {
  render(
    <Providers>
      <MemoryRouter>
        <Routes>
          <Route path="/" element={<MemoryCard memory={memory} />} />
          <Route path="/slideshows/:id" element={<p>Slideshow editor</p>} />
        </Routes>
      </MemoryRouter>
      <ContextMenuHost />
    </Providers>,
  );
}

describe("MemoryCard", () => {
  it("links to the memory and plays its film from the card (Round 4F)", async () => {
    mount();
    expect(screen.getByRole("link")).toHaveAttribute("href", "/memories/3");
    await userEvent.click(screen.getByRole("button", { name: "Play Lisbon" }));
    expect(filmStore.get()).toEqual({ memoryId: 3, title: "Lisbon", coverAssetId: null });
  });

  it("plays the film from its right-click menu", async () => {
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("Lisbon") });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Play" }, { timeout: 5000 }));
    expect(filmStore.get()?.memoryId).toBe(3);
  });

  it("makes a slideshow from the film and opens it", async () => {
    mockCommand("save_memory_film", () => 12);
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("Lisbon") });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Make slideshow" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("save_memory_film", { memoryId: 3 }));
    expect(await screen.findByText("Slideshow editor")).toBeInTheDocument();
  });

  it("hides the memory from its right-click menu, with Undo", async () => {
    mockCommand("set_memory_hidden", () => null);
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("Lisbon") });
    expect(await screen.findByRole("menuitem", { name: "Open" }, { timeout: 5000 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("menuitem", { name: "Hide memory" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_memory_hidden", { id: 3, hidden: true }));
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Memory hidden"));
    act(() => toastStore.get().toasts.at(-1)?.action?.run());
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("set_memory_hidden", { id: 3, hidden: false }));
  });

  it("exports the memory's highlights", async () => {
    mockCommand("memory_curated_ids", () => [11, 12]);
    mount();
    await userEvent.pointer({ keys: "[MouseRight]", target: screen.getByText("Lisbon") });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Export photos…" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("memory_curated_ids", { id: 3 }));
    await waitFor(() => expect(exportSheetStore.get().ids).toEqual([11, 12]));
  });
});
