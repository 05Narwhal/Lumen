import { render, screen, waitFor } from "@testing-library/react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { Providers } from "@/app";
import { invokeMock, mockCommand, mockLibraryDefaults } from "@/test/tauri-mock";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import { LIBRARY_SCOPE } from "@/hooks/use-layout";
import userEvent from "@testing-library/user-event";
import { clearViewerScope, viewerScopeStore } from "@/lib/viewer-scope";
import { PhotoGrid } from "./gui";

const tile = (id: number, x: number) => ({ id, x, w: 300, h: 200, thumbhash: null, color: null, mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 });
const rows: LayoutRow[] = [
  { index: 0, kind: "header", y: 0, height: 44, section: 0, title: "September 2026", tiles: [] },
  { index: 1, kind: "tiles", y: 44, height: 200, section: 0, title: null, tiles: [tile(1, 0), tile(2, 304)] },
  { index: 2, kind: "header", y: 248, height: 44, section: 1, title: "August 2026", tiles: [] },
  { index: 3, kind: "tiles", y: 292, height: 200, section: 1, title: null, tiles: [tile(3, 0)] },
];

describe("PhotoGrid", () => {
  beforeAll(() => {
    // jsdom has no layout; give the grid a width so the layout query runs.
    // The scroll element is 1000 px wide; the inner track (minus padding and gutter) is 952.
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get(this: HTMLElement) {
        return this.classList.contains("photo-grid__inner") ? 952 : 1000;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 800 });
    Object.defineProperty(HTMLElement.prototype, "offsetWidth", { configurable: true, get: () => 1000 });
    Object.defineProperty(HTMLElement.prototype, "offsetHeight", { configurable: true, get: () => 800 });
    HTMLElement.prototype.getBoundingClientRect = () =>
      ({ width: 1000, height: 800, top: 0, left: 0, right: 1000, bottom: 800, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
  });

  it("renders month headers and tiles from the Rust layout", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_layout", () => ({
      totalHeight: 492,
      itemCount: 3,
      rowHeights: [44, 200, 44, 200],
      sections: [
        { index: 0, year: 2026, title: "September 2026", y: 0, height: 248, firstRow: 0, rowCount: 2, itemCount: 2 },
        { index: 1, year: 2026, title: "August 2026", y: 248, height: 244, firstRow: 2, rowCount: 2, itemCount: 1 },
      ],
    }));
    mockCommand("get_layout_rows", (a) => rows.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    const onOpen = vi.fn();
    render(
      <Providers>
        <PhotoGrid rowHeight={200} scope={LIBRARY_SCOPE} onOpen={onOpen} />
      </Providers>,
    );
    expect(await screen.findByRole("heading", { name: "September 2026" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "August 2026" })).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: /^Item \d+$/ })).toHaveLength(3);
    expect(screen.getByRole("grid")).toHaveAttribute("aria-rowcount", "4");
    expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.objectContaining({ width: 952 }));
  });

  it("a dated grid taller than the view has the timeline scrubber in place of a scrollbar", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    // Rows 1 and 3 are 2,000 px tall, so the grid is far taller than its 800 px view.
    const tall = rows.map((r) => (r.kind === "tiles" ? { ...r, height: 2000 } : r));
    mockCommand("get_layout", () => ({
      totalHeight: 4088,
      itemCount: 3,
      rowHeights: [44, 2000, 44, 2000],
      sections: [
        { index: 0, year: 2026, month: 9, title: "September 2026", y: 0, height: 2044, firstRow: 0, rowCount: 2, itemCount: 2 },
        { index: 1, year: 2025, month: 8, title: "August 2025", y: 2044, height: 2044, firstRow: 2, rowCount: 2, itemCount: 1 },
      ],
    }));
    mockCommand("get_layout_rows", (a) => tall.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    render(
      <Providers>
        <PhotoGrid rowHeight={200} scope={LIBRARY_SCOPE} onOpen={vi.fn()} />
      </Providers>,
    );
    const bar = await screen.findByRole("scrollbar");
    expect(bar).toHaveAttribute("aria-valuetext", "September 2026");
    expect(screen.getByRole("button", { name: "2025" })).toBeInTheDocument();
  });

  it("opening a photo tells the viewer which grid it came from", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_layout", () => ({ totalHeight: 492, itemCount: 3, rowHeights: [44, 200, 44, 200], sections: [
      { index: 0, year: 2026, title: "September 2026", y: 0, height: 248, firstRow: 0, rowCount: 2, itemCount: 2 },
      { index: 1, year: 2026, title: "August 2026", y: 248, height: 244, firstRow: 2, rowCount: 2, itemCount: 1 },
    ] }));
    mockCommand("get_layout_rows", (a) => rows.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    const onOpen = vi.fn();
    const album = { ...LIBRARY_SCOPE, albumId: 7 };
    clearViewerScope();
    const { unmount } = render(
      <Providers>
        <PhotoGrid rowHeight={200} scope={album} onOpen={onOpen} />
      </Providers>,
    );
    await userEvent.click((await screen.findAllByRole("button", { name: "Item 2" }))[0]);
    await waitFor(() => expect(onOpen).toHaveBeenCalledWith(2, expect.any(Number)));
    expect(viewerScopeStore.get()).toEqual({ width: 952, rowHeight: 200, gap: expect.any(Number), scope: album, groupBy: "month" });
    // Once the grid is gone, a viewer opened from elsewhere must not page through it.
    unmount();
    expect(viewerScopeStore.get()).toBeNull();
  });

  it("asks Rust where a photo sits and reports when it has scrolled there", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    mockCommand("get_layout", () => ({
      totalHeight: 492,
      itemCount: 3,
      rowHeights: [44, 200, 44, 200],
      sections: [
        { index: 0, year: 2026, title: "September 2026", y: 0, height: 248, firstRow: 0, rowCount: 2, itemCount: 2 },
        { index: 1, year: 2026, title: "August 2026", y: 248, height: 244, firstRow: 2, rowCount: 2, itemCount: 1 },
      ],
    }));
    mockCommand("get_layout_rows", (a) => rows.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    mockCommand("find_layout_row", () => 3);
    const onFocused = vi.fn();
    render(
      <Providers>
        <PhotoGrid rowHeight={120} scope={LIBRARY_SCOPE} groupBy="day" focusAsset={3} onFocused={onFocused} onOpen={vi.fn()} />
      </Providers>,
    );
    await waitFor(() => expect(onFocused).toHaveBeenCalled());
    expect(invokeMock).toHaveBeenCalledWith("find_layout_row", expect.objectContaining({ assetId: 3, groupBy: "day", rowHeight: 120 }));
  });
});

describe("PhotoGrid after photos are removed", () => {
  /**
   * Removing photos re-packs every row after them. Rows cut for the old arrangement stayed on
   * screen at the new one's offsets until refetched: they overlapped, the shift-click order
   * mixed both, and photos nobody selected were added to an album.
   */
  it("never draws rows cut for the old arrangement", async () => {
    const { layoutFingerprint } = await import("@/hooks/use-layout");
    const before = { totalHeight: 492, itemCount: 3, rowHeights: [44, 200, 44, 200], sections: [] };
    const after = { totalHeight: 448, itemCount: 2, rowHeights: [44, 160, 44, 200], sections: [] };
    expect(layoutFingerprint(before)).not.toBe(layoutFingerprint(after));
    expect(layoutFingerprint(before)).toBe(layoutFingerprint({ ...before, rowHeights: [...before.rowHeights] }));

    const { QueryClient } = await import("@tanstack/react-query");
    const { invalidateLibraryData } = await import("@/lib/events");
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    let current = before;
    let currentRows = rows;
    mockCommand("get_layout", () => current);
    // The new rows arrive only after a delay, as from a busy backend.
    let release: () => void = () => {};
    const gate = new Promise<void>((r) => (release = r));
    mockCommand("get_layout_rows", async (a) => {
      if (current === after) await gate;
      return currentRows.slice(Number(a?.start), Number(a?.start) + Number(a?.count));
    });
    const qc = new QueryClient();
    const { useQueryClient } = await import("@tanstack/react-query");
    let client: typeof qc | null = null;
    function Grab() {
      client = useQueryClient();
      return null;
    }
    render(
      <Providers>
        <Grab />
        <PhotoGrid rowHeight={200} scope={LIBRARY_SCOPE} onOpen={vi.fn()} />
      </Providers>,
    );
    await waitFor(() => expect(screen.getAllByRole("button", { name: /^Item \d+$/ })).toHaveLength(3));

    // Photo 2 is removed: row 1 re-packs to a new height.
    current = after;
    currentRows = [rows[0], { ...rows[1], height: 160, tiles: [tile(1, 0)] }, { ...rows[2], y: 204 }, { ...rows[3], y: 248 }];
    void invalidateLibraryData(client!);
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith("get_layout", expect.anything()));
    // While the new rows load, the old row 1 (with photo 2) is not drawn at the new offsets.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Item 2" })).toBeNull());
    release();
    await waitFor(() => expect(screen.getAllByRole("button", { name: /^Item \d+$/ })).toHaveLength(2));
  });
});

describe("PhotoGrid without sections", () => {
  it("gives every row of photos the same gap, the first included", async () => {
    mockCommand("get_settings", () => null);
    mockLibraryDefaults();
    // An unsectioned scope: one untitled section whose first row is photos.
    mockCommand("get_layout", () => ({
      totalHeight: 608,
      itemCount: 3,
      rowHeights: [200, 200, 200],
      sections: [{ index: 0, year: 2026, title: "", y: 0, height: 608, firstRow: 0, rowCount: 3, itemCount: 3 }],
    }));
    const plain: LayoutRow[] = [0, 1, 2].map((i) => ({ index: i, kind: "tiles", y: i * 204, height: 200, section: 0, title: null, tiles: [tile(10 + i, 0)] }));
    mockCommand("get_layout_rows", (a) => plain.slice(Number(a?.start), Number(a?.start) + Number(a?.count)));
    render(
      <Providers>
        <PhotoGrid rowHeight={200} scope={{ ...LIBRARY_SCOPE, memoryId: 4 }} onOpen={vi.fn()} />
      </Providers>,
    );
    await waitFor(() => expect(document.querySelectorAll(".photo-grid__row")).toHaveLength(3));
    const tops = [...document.querySelectorAll<HTMLElement>(".photo-grid__row")].map((r) => parseFloat(r.style.top));
    expect(tops[1] - tops[0]).toBe(tops[2] - tops[1]);
    expect(tops[1] - tops[0]).toBeGreaterThan(200);
  });
});
