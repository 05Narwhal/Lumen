import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { MemoriesRoute } from "./gui";

const memory = (id: number, kind: number, title: string, hidden = false) => ({
  id,
  kind,
  title,
  subtitle: "12–19 March 2024",
  startAt: 0,
  endAt: 0,
  coverAssetId: id,
  itemCount: 20,
  curatedCount: 8,
  hidden,
});

function mount() {
  render(
    <Providers>
      <MemoryRouter initialEntries={["/memories"]}>
        <Routes>
          <Route path="/memories" element={<MemoriesRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("Memories index", () => {
  it("groups memories by kind and shows hidden ones on request", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_memories", (a) => [
      memory(1, 2, "On this day"),
      memory(2, 0, "Trip to Lisbon"),
      memory(3, 1, "Ljubljana"),
      ...(a?.includeHidden ? [memory(4, 0, "Trip to Rome", true)] : []),
    ]);
    mount();
    for (const heading of ["On this day", "Trips", "Events"]) {
      expect(await screen.findByRole("heading", { level: 2, name: heading })).toBeInTheDocument();
    }
    expect(screen.getByRole("link", { name: /Trip to Lisbon/ })).toHaveAttribute("href", "/memories/2");
    expect(screen.queryByText("Trip to Rome")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "Show hidden" }));
    expect(invokeMock).toHaveBeenCalledWith("list_memories", { includeHidden: true });
    expect(await screen.findByRole("link", { name: /Trip to Rome/ })).toBeInTheDocument();
  });

  it("has one sentence and no button when there are none", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mount();
    expect(await screen.findByText("Memories appear as your library grows.")).toBeInTheDocument();
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
