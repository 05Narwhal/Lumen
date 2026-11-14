import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { ContextMenuHost } from "@/components/context-menu";
import { invokeMock, mockCommand } from "@/test/tauri-mock";
import { Sidebar, SHOW_DEVICES } from "./gui";

describe("Sidebar", () => {
  it("renders the three groups and marks the active route", () => {
    SHOW_DEVICES.value = true;
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
    render(
      <Providers><MemoryRouter initialEntries={["/library"]}>
        <Sidebar collapsed={false} onToggle={vi.fn()} devices={[{ id: 1, name: "iPhone 17 Pro" }]} />
      </MemoryRouter></Providers>,
    );
    for (const l of ["Home", "Library", "Albums", "Map", "Memories", "People & Pets", "Import", "Trash", "Settings", "iPhone 17 Pro"]) {
      expect(screen.getByRole("link", { name: l })).toBeInTheDocument();
    }
    expect(screen.getByRole("link", { name: "Library" })).toHaveAttribute("aria-current", "page");
    expect(document.querySelectorAll(".sidebar__indicator")).toHaveLength(1);
  });

  // Round 20: Devices are out of the sidebar, and Slideshows and Pets live under their merged entries.
  it("lights Memories on a slideshow and shows no Devices section", () => {
    SHOW_DEVICES.value = false;
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
    render(
      <Providers><MemoryRouter initialEntries={["/slideshows/3"]}>
        <Sidebar collapsed={false} onToggle={vi.fn()} devices={[{ id: 1, name: "iPhone 17 Pro" }]} />
      </MemoryRouter></Providers>,
    );
    expect(screen.getByRole("link", { name: "Memories" })).toHaveClass("active");
    expect(screen.queryByRole("link", { name: "Slideshows" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Devices" })).toBeNull();
    expect(screen.queryByRole("link", { name: "iPhone 17 Pro" })).toBeNull();
  });

  it("collapses to a rail with icon-only links", () => {
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
    render(
      <Providers><MemoryRouter>
        <Sidebar collapsed onToggle={vi.fn()} devices={[]} />
      </MemoryRouter></Providers>,
    );
    expect(screen.getByRole("navigation", { name: "Primary" })).toHaveAttribute("data-collapsed", "true");
    expect(screen.getByRole("link", { name: "Home" })).toBeInTheDocument();
  });
});

describe("Sidebar sections", () => {
  const devices = Array.from({ length: 12 }, (_, i) => ({ id: i + 1, name: `Camera ${i + 1}` }));
  const mount = () =>
    render(
      <Providers><MemoryRouter initialEntries={["/library"]}>
        <Sidebar collapsed={false} onToggle={vi.fn()} devices={devices} />
      </MemoryRouter></Providers>,
    );

  beforeEach(() => {
    SHOW_DEVICES.value = true;
    localStorage.clear();
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
  });

  it("folds a section away with its chevron and remembers it", async () => {
    const { unmount } = mount();
    const head = screen.getByRole("button", { name: "Devices" });
    expect(head).toHaveAttribute("aria-expanded", "true");
    await userEvent.click(head);
    expect(head).toHaveAttribute("aria-expanded", "false");
    await waitFor(() => expect(screen.queryByRole("link", { name: "Other devices" })).toBeNull());
    unmount();
    mount();
    expect(screen.getByRole("button", { name: "Devices" })).toHaveAttribute("aria-expanded", "false");
  });

  it("animates a section closed instead of dropping it", async () => {
    mount();
    const head = screen.getByRole("button", { name: "Devices" });
    const fold = screen.getByRole("link", { name: "Other devices" }).closest(".sidebar__fold");
    expect(fold, "the section's items sit in a fold that can animate").not.toBeNull();
    await userEvent.click(head);
    // Still there while it folds away, then gone.
    expect(screen.getByRole("link", { name: "Other devices" })).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("link", { name: "Other devices" })).toBeNull());
    await userEvent.click(head);
    expect(await screen.findByRole("link", { name: "Other devices" })).toBeInTheDocument();
  });

  it("caps a long device list so it scrolls on its own, counting the rows it shows", async () => {
    mount();
    // Twelve devices of one kind are one row until the group is expanded.
    const list = screen.getByRole("link", { name: "Other devices" }).closest(".sidebar__list");
    expect(list).not.toHaveAttribute("data-scroll");
    await userEvent.click(screen.getByRole("button", { name: "Show each of Other devices" }));
    expect(list).toHaveAttribute("data-scroll", "true");
  });

  it("keeps Import, Develop, Trash and Settings in the pinned footer", () => {
    mount();
    const footer = document.querySelector(".sidebar__footer-nav");
    for (const l of ["Import", "Develop", "Trash", "Settings"]) {
      expect(footer).toContainElement(screen.getByRole("link", { name: l }));
    }
    expect(document.querySelector(".sidebar__scroll")).toContainElement(screen.getByRole("link", { name: "Library" }));
  });
});

describe("Sidebar device groups", () => {
  beforeEach(() => {
    localStorage.clear();
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => []);
  });

  it("gathers two iPhones under one row that opens them all and expands to each", async () => {
    render(
      <Providers><MemoryRouter initialEntries={["/library"]}>
        <Sidebar
          collapsed={false}
          onToggle={vi.fn()}
          devices={[
            { id: 1, name: "iPhone 13", group: "iphone" },
            { id: 2, name: "NIKON Z 6II", group: "camera" },
            { id: 3, name: "iPhone 17 Pro", group: "iphone" },
          ]}
        />
      </MemoryRouter></Providers>,
    );
    expect(screen.getByRole("link", { name: "iPhones" })).toHaveAttribute("href", "/library/group/iphone");
    expect(screen.getByRole("link", { name: "NIKON Z 6II" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "iPhone 13" })).toBeNull();
    const expand = screen.getByRole("button", { name: "Show each of iPhones" });
    expect(expand).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(expand);
    expect(screen.getByRole("link", { name: "iPhone 13" })).toHaveAttribute("href", "/library/1");
    expect(screen.getByRole("link", { name: "iPhone 17 Pro" })).toBeInTheDocument();
  });

  it("duplicates an album from its right-click menu (Round 4G)", async () => {
    mockCommand("get_settings", () => null);
    mockCommand("list_albums", () => [
      { id: 4, name: "Lisbon", kind: 0, parentId: null, coverAssetId: null, itemCount: 3, createdAt: 1, updatedAt: 1, query: null, hidesItems: false },
    ]);
    mockCommand("duplicate_album", () => ({ id: 5, name: "Lisbon copy" }));
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library"]}>
          <Sidebar collapsed={false} onToggle={vi.fn()} devices={[]} />
        </MemoryRouter>
        <ContextMenuHost />
      </Providers>,
    );
    await userEvent.pointer({ keys: "[MouseRight]", target: await screen.findByRole("link", { name: "Lisbon" }) });
    await userEvent.click(await screen.findByRole("menuitem", { name: "Duplicate album" }, { timeout: 5000 }));
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("duplicate_album", { id: 4 }));
  });

  it("lists collections first and folds one open on its albums (Round 5A)", async () => {
    mockCommand("get_settings", () => null);
    const album = (id: number, name: string, collectionId: number | null) => ({ id, name, kind: 0, parentId: null, coverAssetId: null, itemCount: 1, createdAt: 1, updatedAt: 1, query: null, hidesItems: false, collectionId });
    mockCommand("list_albums", () => [album(1, "Lisbon", 7), album(2, "Family", null)]);
    mockCommand("list_album_collections", () => [{ id: 7, name: "Trips", parentId: null, createdAt: 1, updatedAt: 1 }]);
    render(
      <Providers>
        <MemoryRouter initialEntries={["/library"]}>
          <Sidebar collapsed={false} onToggle={vi.fn()} devices={[]} />
        </MemoryRouter>
      </Providers>,
    );
    const trips = await screen.findByRole("link", { name: "Trips" });
    expect(trips).toHaveAttribute("href", "/albums?c=7");
    expect(screen.getByRole("link", { name: "Family" })).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Lisbon" }), "inside its collection until it is opened").toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Show what is in Trips" }));
    expect(await screen.findByRole("link", { name: "Lisbon" })).toBeInTheDocument();
  });
});
