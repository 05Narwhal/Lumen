import { cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import { ReviewRoute } from "./gui";

vi.mock("maplibre-gl", async () => ({ Map: (await import("@/test/maplibre-mock")).FakeMap, setWorkerUrl: () => {}, addProtocol: () => {} }));

const at = (path: string) =>
  render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <Routes>
          <Route path="/review/:kind" element={<ReviewRoute />} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );

describe("review route", () => {
  afterEach(cleanup);

  it("serves the place review under /review/places for any plugin", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("list_plugins", () => []);
    mockCommand("get_plugin_developer_mode", () => false);
    mockCommand("places_review", () => ({ groups: [], items: [], counts: { check: 0, moments: 0, done: 0, rejected: 0 } }));
    at("/review/places?plugin=acme.finder");
    expect(await screen.findByText("Place review")).toBeInTheDocument();
  });

  it("an unknown review kind is an empty state with a way back, not a crash", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("list_plugins", () => []);
    mockCommand("get_plugin_developer_mode", () => false);
    at("/review/weather?plugin=acme.finder");
    expect(await screen.findByText("Nothing to review here.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Back" })).toBeInTheDocument();
  });

  it("serves each kind a plugin can propose under /review/<kind>, beside the place review", async () => {
    mockLibraryDefaults();
    mockCommand("get_settings", () => null);
    mockCommand("list_plugins", () => []);
    mockCommand("get_plugin_developer_mode", () => false);
    mockCommand("proposals_list", () => ({ items: [], nextCursor: null }));
    for (const [kind, heading] of [
      ["location", "Review places"],
      ["date", "Review dates"],
      ["tags", "Review tags"],
      ["label", "Review labels"],
      ["person", "Review names"],
      ["title", "Review titles"],
      ["caption", "Review captions"],
    ] as const) {
      at(`/review/${kind}?plugin=acme.finder`);
      expect(await screen.findByRole("heading", { name: heading })).toBeInTheDocument();
      cleanup();
    }
  });
});
