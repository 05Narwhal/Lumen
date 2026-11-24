import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { closePluginInstall, pluginInstallStore } from "@/components/plugin-install/store";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import { PluginPageRoute } from "./page";

const openUrl = vi.hoisted(() => vi.fn(() => Promise.resolve()));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl }));

const entry = (over: Partial<StoreEntry> = {}): StoreEntry => ({
  id: "jl.wayfinder",
  name: "Wayfinder",
  author: "JL Studios",
  jl: true,
  summary: "Find where photos without GPS were taken.",
  kinds: ["tools"],
  permissions: ["library.read"],
  icon: "photos://localhost/store-image/aa",
  homepage: "https://example.com/home",
  readme: "https://example.invalid/readme.md",
  screenshots: ["photos://localhost/store-image/s1", "photos://localhost/store-image/s2"],
  authorUrl: "https://example.com/jl",
  repository: "https://example.com/repo",
  latest: { version: "1.0.0", api: "1.1.0", url: "https://example.invalid/w.lumenplugin", blake3: "0", size: 2048, released: "2026-11-21" },
  ...over,
});
const installed = (over: Partial<PluginInfo> = {}) =>
  ({ id: "jl.wayfinder", name: "Wayfinder", version: "1.0.0", author: "JL Studios", description: "d", license: "MIT", trust: { type: "jlStudios" }, kinds: ["tools"], permissions: [], enabled: true, dev: false, state: "ready", iconUrl: null, settings: [], homepage: null, authorUrl: null, repository: null, screenshots: [], hasReadme: false, detectIds: [], assets: [], ...over }) as unknown as PluginInfo;

function mount(plugins: PluginInfo[] = []) {
  mockCommand("list_plugins", () => plugins);
  mockCommand("get_plugin_developer_mode", () => false);
  mockCommand("store_status", () => ({ offline: false, hasCache: true, checkedAt: null, autoUpdate: true, updates: [], updated: [] }));
  return render(
    <Providers>
      <MemoryRouter initialEntries={["/plugins/jl.wayfinder"]}>
        <Routes>
          <Route path="/plugins/:id" element={<PluginPageRoute />} />
          <Route path="/plugins" element={<div>store list</div>} />
        </Routes>
      </MemoryRouter>
    </Providers>,
  );
}

describe("Plugin page", () => {
  beforeEach(() => {
    invokeMock.mockClear();
    openUrl.mockClear();
    closePluginInstall();
    mockCommand("store_index", () => ({ version: 1, featured: null, plugins: [entry()] }));
    mockCommand("store_readme", () => "# Hello\n\nSome **words** and [a link](https://example.com/x).\n\n<script>alert(1)</script>");
  });

  it("shows the header, permissions and facts, and Get opens the install sheet", async () => {
    mockCommand("store_install", () => ({ path: "C:/tmp/w.lumenplugin", installed: false }));
    mount();
    expect(await screen.findByRole("heading", { name: "Wayfinder" })).toBeInTheDocument();
    expect(screen.getByText("By JL Studios")).toBeInTheDocument();
    expect(screen.getByText("1.0.0")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Get" }));
    await waitFor(() => expect(pluginInstallStore.get().path).toBe("C:/tmp/w.lumenplugin"));
  });

  it("links the author, the repository and the homepage through the opener", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "JL Studios" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.com/jl");
    await userEvent.click(screen.getByRole("button", { name: "Repository" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.com/repo");
  });

  it("renders the README as Markdown with raw HTML off and links through the opener", async () => {
    mount();
    expect(await screen.findByRole("heading", { name: "Hello" })).toBeInTheDocument();
    expect(document.querySelector("script")).toBeNull();
    await userEvent.click(screen.getByRole("link", { name: "a link" }));
    expect(openUrl).toHaveBeenCalledWith("https://example.com/x");
  });

  it("enlarges a screenshot on click and closes it again", async () => {
    mount();
    await userEvent.click(await screen.findByRole("button", { name: "Screenshot 1" }));
    const big = await screen.findByRole("img", { name: "Screenshot 1, enlarged" });
    expect(big).toHaveAttribute("src", "photos://localhost/store-image/s1");
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("img", { name: "Screenshot 1, enlarged" })).toBeNull());
  });

  it("an installed plugin shows Open and reads its own README", async () => {
    mockCommand("plugin_readme", () => "# Local readme");
    mount([installed({ hasReadme: true })]);
    expect(await screen.findByRole("button", { name: "Open" })).toBeEnabled();
    expect(await screen.findByRole("heading", { name: "Local readme" })).toBeInTheDocument();
    expect(invokeMock.mock.calls.some(([c]) => c === "store_readme")).toBe(false);
  });

  it("a plugin the store does not list still opens from its installed copy", async () => {
    mockCommand("store_index", () => ({ version: 1, featured: null, plugins: [] }));
    mount([installed({ name: "Local thing" })]);
    expect(await screen.findByRole("heading", { name: "Local thing" })).toBeInTheDocument();
  });

  it("says so for an unknown plugin and goes back to the store", async () => {
    mockCommand("store_index", () => ({ version: 1, featured: null, plugins: [] }));
    mount();
    expect(await screen.findByText("That plugin is not in the store.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back to plugins" }));
    expect(await screen.findByText("store list")).toBeInTheDocument();
  });
});
