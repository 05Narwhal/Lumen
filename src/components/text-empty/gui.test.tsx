import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { afterEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { jobsStore } from "@/lib/jobs-store";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { TextEmptyState } from "./gui";

function Where() {
  return <p data-testid="where">{useLocation().pathname + useLocation().search}</p>;
}

function mount(readText: boolean) {
  mockLibraryDefaults();
  mockCommand("get_settings", () => ({ appearance: { reduceMotion: false }, smart: { enabled: true, experimental: { readText } } }));
  render(
    <Providers>
      <MemoryRouter initialEntries={["/library"]}>
        <Routes>
          <Route path="*" element={<><TextEmptyState /><Where /></>} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

afterEach(() => jobsStore.set({ jobs: [] }));

describe("TextEmptyState", () => {
  it("offers to turn reading text on while it is off", async () => {
    mount(false);
    expect(screen.getByText("No text found in these photos yet")).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("button", { name: "Turn on reading text" }));
    expect(screen.getByTestId("where")).toHaveTextContent("/settings?section=experimental");
  });

  it("says how many photos are still to read while the scan runs", async () => {
    mount(true);
    act(() => jobsStore.set({ jobs: [{ id: "ocr", kind: "maintenance", label: "Reading text", progress: 0.5, state: "running", done: 796, total: 2000 }] }));
    expect(await screen.findByText("Still reading text · 1,204 left")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Turn on reading text" })).toBeNull();
  });

  it("says nothing was found once every photo is read", async () => {
    mount(true);
    expect(await screen.findByText("Lumen found no words in them.")).toBeInTheDocument();
  });
});
