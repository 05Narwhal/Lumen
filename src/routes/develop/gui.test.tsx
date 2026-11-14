import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { DevelopRoute } from "./gui";

function Where() {
  return <p data-testid="where">{useLocation().pathname}</p>;
}

function mount(url: string) {
  render(
    <Providers>
      <MemoryRouter initialEntries={[url]}>
        <Routes>
          <Route
            path="/develop/:id?"
            element={
              <>
                <Where />
                <DevelopRoute />
              </>
            }
          />
          <Route path="*" element={<Where />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("DevelopRoute", () => {
  beforeEach(() => {
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("develop_preview", () => ({ token: "5", width: 300, height: 200, superseded: false, histogram: null }));
  });

  it("starts empty and asks for photos, without reading the library", async () => {
    mount("/develop");
    expect(await screen.findByText("Choose photos to develop")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Choose photos…" })).toBeInTheDocument();
    expect(invokeMock).not.toHaveBeenCalledWith("strip_ids", expect.anything());
  });

  it("says so when a chosen set has nothing to develop", async () => {
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: null, label: "Album" });
    mockCommand("strip_ids", () => []);
    mount("/develop");
    expect(await screen.findByText("Nothing to develop here")).toBeInTheDocument();
  });

  it("opens the filmstrip's first photo and renders it", async () => {
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [42, 43], label: "Selection" });
    mockCommand("strip_ids", () => [42, 43]);
    mount("/develop");
    await waitFor(() => expect(screen.getByTestId("where")).toHaveTextContent("/develop/42"));
    // Its cached rendition stands in until the render arrives, then the render replaces it.
    await waitFor(() => expect(screen.getByRole("img", { name: "The photo as developed" }).getAttribute("src")).toContain("edit-preview/5"));
    expect(invokeMock).toHaveBeenCalledWith("strip_ids", { scope: DEFAULT_DEVELOP_SOURCE.scope, only: [42, 43], sort: "captured" });
  });

  it("names its source in the top bar", async () => {
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [9], label: "Selection" });
    mount("/develop/9");
    expect(await screen.findByText("Selection")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Undo" })).toBeDisabled();
  });
});
