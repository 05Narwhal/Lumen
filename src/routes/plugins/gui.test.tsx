import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it } from "vitest";
import { Providers } from "@/app";
import { closePluginInstall, pluginInstallStore } from "@/components/plugin-install/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import { PluginsRoute } from "./gui";

const entry = (over: Partial<StoreEntry> = {}): StoreEntry => ({
  id: "jl.wayfinder",
  name: "Wayfinder",
  author: "JL Studios",
  jl: true,
  summary: "Find where photos without GPS were taken.",
  kinds: ["tools"],
  permissions: ["library.read"],
  icon: null,
  homepage: null,
  readme: null,
  screenshots: [],
  authorUrl: null,
  repository: null,
  latest: { version: "1.0.0", api: "1.1.0", url: "https://example.invalid/w.lumenplugin", blake3: "0", size: 2048, released: "2026-11-21" },
  ...over,
});

const status = (over = {}) => ({ offline: false, hasCache: true, checkedAt: null, autoUpdate: true, updates: [], updated: [], ...over });
const calls = (name: string) => invokeMock.mock.calls.filter(([c]) => c === name);

function mount(path = "/plugins", st: object = status()) {
  mockCommand("list_plugins", () => []);
  mockCommand("get_plugin_developer_mode", () => false);
  mockCommand("store_status", () => st);
  return render(
    <Providers>
      <MemoryRouter initialEntries={[path]}>
        <PluginsRoute />
      </MemoryRouter>
    </Providers>,
  );
}

describe("Plugins page (the store)", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    closePluginInstall();
  });

  it("shows the offline screen with nothing cached, and Try again asks for the index again", async () => {
    mockCommand("store_index", () => {
      throw { code: "store_offline", message: "offline" };
    });
    mount();
    expect(await screen.findByText("The plugin store needs the internet.")).toBeInTheDocument();
    const before = calls("store_index").length;
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(calls("store_index").length).toBeGreaterThan(before));
  });

  it("lists the store, marks JL's plugins, and Get opens the install sheet on the download", async () => {
    mockCommand("store_index", () => ({ version: 1, plugins: [entry(), entry({ id: "x.warm", name: "Warm looks", author: "Ada", jl: false, kinds: ["presets"] })] }));
    mockCommand("store_install", () => ({ path: "C:/tmp/w.lumenplugin", installed: false }));
    mount();
    expect((await screen.findAllByText("Wayfinder")).length).toBeGreaterThan(0);
    expect(screen.getAllByText("Warm looks").length).toBeGreaterThan(0);
    await userEvent.click(screen.getAllByRole("button", { name: "Get Wayfinder" })[0]);
    await waitFor(() => expect(pluginInstallStore.get().path).toBe("C:/tmp/w.lumenplugin"));
    expect(calls("store_install")[0][1]).toEqual({ id: "jl.wayfinder" });
  });

  it("says so when a search matches nothing, and Clear search brings the list back", async () => {
    mockCommand("store_index", () => ({ version: 1, plugins: [entry()] }));
    mount();
    await screen.findAllByText("Wayfinder");
    await userEvent.type(screen.getByRole("searchbox"), "zzz");
    expect(await screen.findByText("No plugins match.")).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("status")).getByRole("button", { name: "Clear search" }));
    expect((await screen.findAllByText("Wayfinder")).length).toBeGreaterThan(0);
  });
});

const daysAgo = (n: number) => new Date(Date.now() - n * 86_400_000).toISOString().slice(0, 10);
const rel = (released: string) => ({ version: "1.0.0", api: "1.1.0", url: "https://example.invalid/p.lumenplugin", blake3: "0", size: 1, released });
const shot = "photos://localhost/store-image/aaa.png";

describe("Discover (Round 25 P2)", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    closePluginInstall();
  });
  const index = (featured: string | null = "jl.wayfinder") => ({
    version: 1,
    featured,
    plugins: [
      entry({ screenshots: [shot], icon: "photos://localhost/store-image/icon.png", latest: rel(daysAgo(200)) }),
      entry({ id: "x.warm", name: "Warm looks", author: "Ada", jl: false, kinds: ["presets"], latest: rel(daysAgo(3)) }),
      entry({ id: "x.old", name: "Old theme", author: "Bo", jl: false, kinds: ["theme"], latest: rel(daysAgo(100)) }),
    ],
  });

  it("leads with the featured plugin: its first screenshot, its icon and a Get button", async () => {
    mockCommand("store_index", () => index());
    mount();
    const hero = await screen.findByRole("region", { name: "Featured" });
    expect(within(hero).getByRole("heading", { name: "Wayfinder" })).toBeInTheDocument();
    expect(within(hero).getByAltText("Wayfinder screenshot")).toHaveAttribute("src", shot);
    expect(within(hero).getByRole("button", { name: "Get Wayfinder" })).toBeInTheDocument();
  });

  it("has no hero when the index names none", async () => {
    mockCommand("store_index", () => index(null));
    mount();
    await screen.findByRole("heading", { name: "By JL Studios" });
    expect(screen.queryByRole("region", { name: "Featured" })).toBeNull();
  });

  it("sorts plugins into Updates, New and By JL Studios, then one row per kind, each plugin once", async () => {
    mockCommand("store_index", () => index());
    mount("/plugins", status({ updates: [{ id: "x.old", name: "Old theme", from: "0.9.0", to: "1.0.0", needsReview: false }] }));
    const section = async (name: string) => within(await screen.findByRole("region", { name }));
    expect((await section("By JL Studios")).getByText("Wayfinder")).toBeInTheDocument();
    const fresh = await section("New");
    expect(fresh.getByText("Warm looks")).toBeInTheDocument();
    expect(fresh.queryByText("Old theme")).toBeNull();
    const updated = await section("Updates");
    await waitFor(() => expect(updated.getByText("Old theme")).toBeInTheDocument());
    // Claimed above, so their kind rows are left out: no plugin shows twice outside the hero.
    expect(screen.queryByRole("region", { name: "Themes" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Looks" })).toBeNull();
    expect(screen.getAllByText("Warm looks")).toHaveLength(1);
  });

  it("See all opens a section's flat grid, and Back returns to Discover", async () => {
    mockCommand("store_index", () => index());
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "See all By JL Studios" }));
    expect(await screen.findByRole("heading", { name: "By JL Studios" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Featured" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Back to Discover" }));
    expect(await screen.findByRole("region", { name: "Featured" })).toBeInTheDocument();
  });

  it("says so in a section that has nothing", async () => {
    mockCommand("store_index", () => ({ version: 1, featured: null, plugins: [entry({ jl: false, latest: rel(daysAgo(200)) })] }));
    mount();
    expect(await screen.findByText("Nothing new in the last 30 days.")).toBeInTheDocument();
    expect(screen.getByText("Everything you have is up to date.")).toBeInTheDocument();
  });

  it("filters by kind chips instead of a select, and an empty kind has an empty state", async () => {
    mockCommand("store_index", () => index());
    mount();
    await screen.findByRole("region", { name: "Featured" });
    expect(screen.queryByRole("combobox")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Looks" }));
    expect(screen.getByRole("button", { name: "Looks" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByText("Warm looks")).toBeInTheDocument();
    expect(screen.queryByText("Old theme")).toBeNull();
    expect(screen.queryByRole("region", { name: "Featured" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Providers" }));
    expect(await screen.findByText("No providers in the store yet.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "All" }));
    expect(await screen.findByRole("region", { name: "Featured" })).toBeInTheDocument();
  });

  it("collapses to one flat list while searching", async () => {
    mockCommand("store_index", () => index());
    mount();
    await screen.findByRole("region", { name: "Featured" });
    await userEvent.type(screen.getByRole("searchbox"), "warm");
    expect(screen.getAllByText("Warm looks")).toHaveLength(1);
    expect(screen.queryByRole("region", { name: "Featured" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "By JL Studios" })).toBeNull();
  });

  it("a card shows its icon and a screenshot thumbnail and opens the plugin page", async () => {
    mockCommand("store_index", () => index());
    mount();
    const jl = within(await screen.findByRole("region", { name: "By JL Studios" }));
    expect(jl.getByAltText("Wayfinder screenshot")).toHaveAttribute("src", shot);
    expect(jl.getByRole("button", { name: "Details of Wayfinder" })).toBeInTheDocument();
  });
});
