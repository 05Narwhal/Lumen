import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { ReviewItem } from "@/types/generated/ReviewItem";
import type { ReviewState } from "@/types/generated/ReviewState";
import { PlaceReview } from "./gui";

vi.mock("maplibre-gl", async () => ({ Map: (await import("@/test/maplibre-mock")).FakeMap, setWorkerUrl: () => {}, addProtocol: () => {} }));

const item = (asset: number, group: string, state: ReviewItem["state"]): ReviewItem => ({
  asset,
  group,
  state,
  flag: null,
  candidates: [{ lat: 46.05, lon: 14.5, confidence: 0.6, label: "Ljubljana" }],
  status: "pending",
});

const state: ReviewState = {
  groups: [
    { id: "a", label: "12 Mar · Ljubljana", count: 2, check: 2 },
    { id: "b", label: "13 Mar · Bled", count: 1, check: 1 },
  ],
  items: [item(1, "a", "guess"), item(2, "a", "guess"), item(3, "b", "guess")],
  counts: { check: 3, moments: 2, done: 0, rejected: 0 },
};

describe("PlaceReview", () => {
  afterEach(cleanup);

  it("Enter on a moment confirms its photos and Write is disabled with nothing done", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_plugins", () => []);
    mockCommand("get_plugin_developer_mode", () => false);
    mockCommand("places_review", () => state);
    mockCommand("places_review_event", () => ({ ...state, counts: { ...state.counts, check: 1, done: 2 } }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/review/places?plugin=jl.wayfinder"]}>
          <Routes>
            <Route path="/review/places" element={<PlaceReview />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("12 Mar · Ljubljana", { selector: "h2" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Write 0 locations/ })).toBeDisabled();
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("places_review_event", { pluginId: "jl.wayfinder", event: { kind: "confirm", assets: [1, 2] } }),
    );
  });

  it("says there is nothing to review when the plugin suggested nothing", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("list_plugins", () => []);
    mockCommand("places_review", () => ({ groups: [], items: [], counts: { check: 0, moments: 0, done: 0, rejected: 0 } }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/review/places?plugin=jl.wayfinder"]}>
          <Routes>
            <Route path="/review/places" element={<PlaceReview />} />
          </Routes>
        </MemoryRouter>
      </Providers>,
    );
    expect(await screen.findByText("Nothing to review.")).toBeInTheDocument();
  });
});
