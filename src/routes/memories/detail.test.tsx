import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { toastStore } from "@/components/toast/store";
import { filmStore } from "@/lib/film";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { MemoriesRoute } from "./gui";

function Where() {
  const { pathname, search } = useLocation();
  return <p data-testid="where">{pathname + search}</p>;
}

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/memories/7"]}>
        <Routes>
          <Route path="/memories/:id" element={<><MemoriesRoute /><Where /></>} />
          <Route path="/slideshows/:id" element={<Where />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

// jsdom lays nothing out; the grid only asks for a layout once it has a width.
const SIZES = ["clientWidth", "clientHeight", "offsetWidth", "offsetHeight"] as const;
afterEach(() => {
  for (const p of SIZES) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[p];
});

beforeEach(() => {
  for (const p of SIZES) Object.defineProperty(HTMLElement.prototype, p, { configurable: true, get: () => (p.endsWith("Width") ? 1000 : 800) });
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  mockCommand("get_memory", () => ({ id: 7, kind: 0, title: "Trip to Lisbon", subtitle: "12–19 March 2024", startAt: 100, endAt: 900, coverAssetId: 3, itemCount: 40, curatedCount: 10, hidden: false }));
  mockCommand("memory_curated_ids", () => [3, 4, 5]);
  toastStore.set({ toasts: [] });
  filmStore.set(null);
});

describe("Memory detail", () => {
  it("shows the highlights, and all photos of the period on request", async () => {
    mount();
    expect(await screen.findByRole("heading", { level: 1, name: "Trip to Lisbon" })).toBeInTheDocument();
    expect(screen.getByText("12–19 March 2024")).toBeInTheDocument();
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ memoryId: 7 }) })));
    await userEvent.click(screen.getByRole("radio", { name: "All photos (40)" }));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ scope: expect.objectContaining({ memoryId: null, query: expect.objectContaining({ dateFrom: 100, dateTo: 900 }) }) })),
    );
    expect(screen.getByRole("radio", { name: "Highlights (10)" })).toBeInTheDocument();
  });

  it("hides with Undo", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Hide" }));
    expect(invokeMock).toHaveBeenCalledWith("set_memory_hidden", { id: 7, hidden: true });
    await waitFor(() => expect(toastStore.get().toasts.at(-1)?.message).toBe("Memory hidden"));
    expect(toastStore.get().toasts.at(-1)?.action?.label).toBe("Undo");
  });

  it("exports its film as a video to a folder asked for, with no slideshow made (Round 6)", async () => {
    vi.mocked(openDialog).mockResolvedValueOnce("D:/Films");
    mockCommand("export_memory_film", () => null);
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Export video" }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("export_memory_film", { memoryId: 7, destDir: "D:/Films" }));
    expect(invokeMock).not.toHaveBeenCalledWith("save_memory_film", expect.anything());
  });

  it("plays the memory as a film, and no longer offers Make slideshow (Round 4F)", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Play" }));
    expect(filmStore.get()).toEqual({ memoryId: 7, title: "Trip to Lisbon", coverAssetId: 3 });
    expect(screen.queryByRole("button", { name: "Make slideshow" })).not.toBeInTheDocument();
  });
});
