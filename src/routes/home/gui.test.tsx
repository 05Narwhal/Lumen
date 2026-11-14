import { leaves } from "@/lib/search-expr";
import { parseTags } from "@/lib/search-tags";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { DEFAULT_DEVELOP_SOURCE, developSourceStore } from "@/lib/develop-source";
import { mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { HomeSummary } from "@/types/generated/HomeSummary";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { HomeRoute } from "./gui";
import { pickHero } from "./pick-hero";

const NOTHING: HomeSummary = { drafts: [], draftCount: 0, stripLabel: null, unreadable: 0, peopleToReview: 0, places: [] };

const memory = (id: number, kind: number, title: string): MemorySummary => ({
  id,
  kind,
  title,
  subtitle: "12–19 March 2024",
  startAt: 1_710_000_000,
  endAt: 1_710_500_000,
  coverAssetId: id + 100,
  itemCount: 40,
  curatedCount: 10,
  hidden: false,
});

/** A library with 40 photos and nothing else; a test overrides what it cares about. */
function setup() {
  mockCommand("get_settings", () => null);
  mockLibraryDefaults();
  mockCommand("get_library_stats", () => ({ items: 40, videos: 0, bytes: 1 }));
  mockCommand("home_summary", () => NOTHING);
  mockCommand("count_duplicate_sets", () => [0, 0]);
  mockCommand("list_iphones", () => ({ usbmuxdAvailable: true, devices: [] }));
}

function renderHome() {
  render(
    <Providers>
      <MemoryRouter>
        <HomeRoute />
      </MemoryRouter>
    </Providers>,
  );
}

describe("HomeRoute", () => {
  it("shows the latest import with its thumbnails and the stats line", async () => {
    setup();
    mockCommand("get_library_stats", () => ({ items: 1248, videos: 12, bytes: 5 * 1024 ** 3 }));
    mockCommand("list_import_batches", () => [
      { id: 2, sourceKind: 0, sourceLabel: "Screenshots", sourcePath: null, startedAt: 1, finishedAt: 2, itemCount: 80, skippedCount: 0, failedCount: 0, state: 1 },
    ]);
    mockCommand("get_recent_asset_ids", () => [11, 12, 13]);
    mockCommand("list_devices", () => [{ id: 1, displayName: "iPhone 17 Pro", kind: 1, icon: null, itemCount: 812, lastImportAt: 1, hidden: false }]);
    renderHome();
    const card = await screen.findByRole("article", { name: "Latest import" });
    expect(within(card).getByRole("heading", { name: "Screenshots" })).toBeInTheDocument();
    expect(await within(card).findAllByRole("button", { name: /Open item/ })).toHaveLength(3);
    expect(screen.getByLabelText("Library stats")).toHaveTextContent("1,248 items · 12 videos · 5.0 GB");
    expect(await screen.findByRole("link", { name: /iPhone 17 Pro/ })).toHaveAttribute("href", "/library/1");
  });

  it("shows the empty card when there is nothing yet", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    renderHome();
    expect(await screen.findByRole("status")).toHaveTextContent(/empty/i);
    expect(screen.getByText("Import a folder, a card or a phone to get started.")).toBeInTheDocument();
  });

  it("draws neither the hero nor the Develop card when there are no memories and no drafts", async () => {
    setup();
    renderHome();
    expect(await screen.findByLabelText("Library stats")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Memories" })).not.toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Featured memory" })).not.toBeInTheDocument();
    expect(screen.queryByRole("article", { name: "In Develop" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Continue" })).not.toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Needs attention" })).not.toBeInTheDocument();
  });

  it("shows the picked memory as the hero, the others beside it", async () => {
    setup();
    const memories = [memory(7, 0, "Trip to Lisbon"), memory(8, 1, "Ljubljana"), memory(9, 1, "Bled")];
    mockCommand("list_memories", () => memories);
    renderHome();
    const hero = await screen.findByRole("group", { name: "Featured memory" });
    const picked = pickHero(memories, new Date())!;
    expect(within(hero).getByRole("heading", { name: picked.title })).toBeInTheDocument();
    expect(within(hero).getByRole("link", { name: `Open ${picked.title}` })).toHaveAttribute("href", `/memories/${picked.id}`);
    expect(within(hero).getByRole("button", { name: "Play" })).toBeInTheDocument();
    const region = screen.getByRole("region", { name: "Memories" });
    for (const other of memories.filter((m) => m.id !== picked.id)) {
      expect(within(region).getByRole("link", { name: new RegExp(other.title) })).toHaveAttribute("href", `/memories/${other.id}`);
    }
    expect(within(region).getByRole("link", { name: "See all" })).toHaveAttribute("href", "/memories");
  });

  it("lists what needs attention, and the Develop drafts", async () => {
    setup();
    mockCommand("home_summary", () => ({ ...NOTHING, drafts: [5, 6], draftCount: 9, unreadable: 3 }));
    mockCommand("count_duplicate_sets", () => [4, 7]);
    mockCommand("list_volumes", () => [{ mount: "E:\\", label: "EOS R6", key: "k", totalItems: 200, newItems: 124, removable: true }]);
    renderHome();
    const strip = await screen.findByRole("navigation", { name: "Needs attention" });
    expect(await within(strip).findByRole("link", { name: "4 duplicate sets" })).toHaveAttribute("href", "/duplicates");
    expect(within(strip).getByRole("link", { name: "Import 124 new · EOS R6" })).toBeInTheDocument();
    expect(within(strip).getByRole("link", { name: "3 files Lumen can't open" })).toHaveAttribute("href", "/import?mode=repair");
    const develop = await screen.findByRole("article", { name: "In Develop" });
    expect(within(develop).getByRole("heading", { name: "In Develop · 9" })).toBeInTheDocument();
    expect(within(develop).getAllByRole("button", { name: /Develop photo/ })).toHaveLength(2);
  });

  it("opens every photo with Develop work from the card when no filmstrip is remembered", async () => {
    setup();
    developSourceStore.set(DEFAULT_DEVELOP_SOURCE);
    mockCommand("list_draft_ids", () => [5, 6, 7, 8, 9, 10, 11, 12, 13]);
    mockCommand("home_summary", () => ({ ...NOTHING, drafts: [5, 6], draftCount: 9 }));
    renderHome();
    const develop = await screen.findByRole("article", { name: "In Develop" });
    await userEvent.click(within(develop).getByRole("button", { name: "Develop photo 6" }));
    await vi.waitFor(() => expect(developSourceStore.get().only).toEqual([5, 6, 7, 8, 9, 10, 11, 12, 13]));
  });

  it("shows the remembered filmstrip's label on the card and opens exactly that set", async () => {
    setup();
    developSourceStore.set({ ...DEFAULT_DEVELOP_SOURCE, only: [20, 21, 22], label: "Selection" });
    mockCommand("home_summary", () => ({ ...NOTHING, drafts: [21, 20, 22], draftCount: 3, stripLabel: "Selection" }));
    renderHome();
    const develop = await screen.findByRole("article", { name: "In Develop" });
    expect(within(develop).getByText("Selection")).toBeInTheDocument();
    await userEvent.click(within(develop).getByRole("button", { name: "Open Develop" }));
    await vi.waitFor(() => expect(developSourceStore.get().label).toBe("Selection"));
    expect(developSourceStore.get().only).toEqual([20, 21, 22]);
  });

  it("links a place to the Library narrowed to it", async () => {
    setup();
    mockCommand("home_summary", () => ({
      ...NOTHING,
      places: [{ place: { kind: "city", name: "Idrija", country: "Slovenia" }, label: "Idrija, Slovenia", photoCount: 31, lat: 46, lon: 14, coverAssetId: 1 }],
    }));
    renderHome();
    const link = await screen.findByRole("link", { name: /Idrija, Slovenia/ });
    const href = link.getAttribute("href") ?? "";
    expect(href.startsWith("/library?tags=")).toBe(true);
    const read = leaves(parseTags(new URLSearchParams(href.split("?")[1]))).map((l) => l.tag);
    expect(read).toEqual([
      { label: "Idrija, Slovenia", value: { type: "place", place: { kind: "city", name: "Idrija", country: "Slovenia" } } },
    ]);
  });

  describe("devices", () => {
    const device = (id: number, lastImportAt: number | null) => ({
      id,
      displayName: `Camera ${id}`,
      kind: 2,
      icon: null,
      itemCount: 10,
      lastImportAt,
      hidden: false,
      group: "camera",
    });
    const renderDevices = (count: number) => {
      setup();
      // Device n was last used at time n, so the newest are the highest ids.
      mockCommand("list_devices", () => Array.from({ length: count }, (_, i) => device(i + 1, i + 1)));
      renderHome();
    };

    it("shows the 8 most recently used, and the rest behind Show all", async () => {
      renderDevices(45);
      const region = await screen.findByRole("region", { name: "Devices" });
      await within(region).findByRole("link", { name: /Camera 45/ });
      expect(within(region).getAllByRole("link")).toHaveLength(8);
      expect(within(region).queryByRole("link", { name: /^Camera 3710 items/ })).not.toBeInTheDocument();
      expect(within(region).getByRole("link", { name: /^Camera 3810 items/ })).toBeInTheDocument();
      await userEvent.click(within(region).getByRole("button", { name: "Show all 45" }));
      expect(within(region).getAllByRole("link")).toHaveLength(45);
      expect(within(region).getByRole("button", { name: "Show fewer" })).toBeInTheDocument();
    });

    it("has no Show all with 8 or fewer", async () => {
      renderDevices(6);
      const region = await screen.findByRole("region", { name: "Devices" });
      await within(region).findByRole("link", { name: /Camera 6/ });
      expect(within(region).getAllByRole("link")).toHaveLength(6);
      expect(within(region).queryByRole("button", { name: /Show all/ })).not.toBeInTheDocument();
    });
  });
});
