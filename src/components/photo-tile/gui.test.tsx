import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Tile } from "@/types/generated/Tile";
import { rgbaToThumbHash } from "thumbhash";
import { Providers } from "@/app";
import { mockCommand } from "@/test/tauri-mock";
import { PhotoTile, thumbSize } from "./gui";

const base: Tile = { id: 7, x: 10, w: 200, h: 150, thumbhash: null, color: "#336699", mediaType: 0, durationMs: null, renderable: true, extension: "jpg", isFavorite: false, trashedAt: null, rating: 0, hdr: 0, raw: false, colorLabel: 0, flag: 0 };

describe("thumbSize", () => {
  it("chooses on the longest side, so a landscape M tile is not an upscaled 256", () => {
    // An M row is 200 px tall; a 3:2 landscape tile is 300 px wide. Picking on height
    // alone left it on the 256 px thumbnail, stretched across 300 px.
    expect(thumbSize(300, 200)).toBe(1024);
    expect(thumbSize(180, 120)).toBe(256); // S stays cheap
    expect(thumbSize(214, 320)).toBe(1024); // L portrait
  });

  it("accounts for device pixel ratio", () => {
    const original = window.devicePixelRatio;
    Object.defineProperty(window, "devicePixelRatio", { value: 2, configurable: true });
    expect(thumbSize(180, 120)).toBe(1024); // 360 physical px needs the larger derivative
    Object.defineProperty(window, "devicePixelRatio", { value: original, configurable: true });
  });
});

describe("PhotoTile", () => {
  it("asks again after a failed load instead of keeping the format label", () => {
    vi.useFakeTimers();
    try {
      render(<PhotoTile tile={base} top={0} selected={false} selecting={false} onOpen={vi.fn()} onToggle={vi.fn()} />);
      const first = document.querySelector("img")!;
      fireEvent.error(first);
      expect(document.querySelector(".photo-tile__format")).toBeNull();
      act(() => void vi.advanceTimersByTime(2000));
      const second = document.querySelector("img")!;
      expect(second).not.toBe(first);
      fireEvent.load(second);
      expect(document.querySelector(".photo-tile")).toHaveAttribute("data-loading", "false");
      expect(document.querySelector(".photo-tile__format")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows the format label only after every retry failed", () => {
    vi.useFakeTimers();
    try {
      render(<PhotoTile tile={base} top={0} selected={false} selecting={false} onOpen={vi.fn()} onToggle={vi.fn()} />);
      for (const wait of [2000, 8000, 30000]) {
        fireEvent.error(document.querySelector("img")!);
        expect(document.querySelector(".photo-tile__format")).toBeNull();
        act(() => void vi.advanceTimersByTime(wait));
      }
      fireEvent.error(document.querySelector("img")!);
      expect(document.querySelector(".photo-tile__format")).toHaveTextContent("JPG");
      expect(document.querySelector("img")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("renders the 256px thumbnail and opens on click", async () => {
    const onOpen = vi.fn();
    const onToggle = vi.fn();
    render(<PhotoTile tile={base} top={44} selected={false} selecting={false} onOpen={onOpen} onToggle={onToggle} />);
    const img = document.querySelector("img")!;
    expect(img.getAttribute("src")).toContain("thumb/7?s=256");
    expect(document.querySelector(".photo-tile")).toHaveAttribute("data-loading", "true");
    fireEvent.load(img);
    expect(document.querySelector(".photo-tile")).toHaveAttribute("data-loading", "false");
    fireEvent.click(screen.getByRole("button", { name: "Item 7" }));
    expect(onOpen).not.toHaveBeenCalled();
    await new Promise((r) => setTimeout(r, 260));
    expect(onOpen).toHaveBeenCalledWith(7);
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("favorites on double-click without opening, and shows the star", async () => {
    const onOpen = vi.fn();
    const onFavorite = vi.fn();
    const { rerender } = render(<PhotoTile tile={base} top={0} selected={false} selecting={false} onOpen={onOpen} onToggle={vi.fn()} onFavorite={onFavorite} />);
    const hit = screen.getByRole("button", { name: "Item 7" });
    fireEvent.click(hit);
    fireEvent.click(hit);
    fireEvent.doubleClick(hit, { clientX: 40, clientY: 50 });
    await new Promise((r) => setTimeout(r, 260));
    // Only the id: the burst is aimed at the tile, never at where the pointer landed.
    expect(onFavorite).toHaveBeenCalledWith(7);
    expect(onOpen).not.toHaveBeenCalled();
    rerender(<PhotoTile tile={base} top={0} selected={false} selecting={false} favorite onOpen={onOpen} onToggle={vi.fn()} onFavorite={onFavorite} />);
    expect(screen.getByLabelText("Favorite")).toBeInTheDocument();
  });

  it("toggles with ctrl-click and via the checkmark", () => {
    const onOpen = vi.fn();
    const onToggle = vi.fn();
    render(<PhotoTile tile={base} top={0} selected={false} selecting={false} onOpen={onOpen} onToggle={onToggle} />);
    fireEvent.click(screen.getByRole("button", { name: "Item 7" }), { ctrlKey: true });
    expect(onToggle).toHaveBeenCalledWith(7, { ctrl: true, shift: false });
    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    expect(onToggle).toHaveBeenCalledTimes(2);
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("shows a duration badge for video and a format label when not renderable", () => {
    render(
      <PhotoTile
        tile={{ ...base, id: 8, mediaType: 1, durationMs: 65_000, renderable: false, extension: "mov" }}
        top={0}
        selected={false}
        selecting={false}
        onOpen={vi.fn()}
        onToggle={vi.fn()}
      />,
    );
    expect(screen.getByText("1:05")).toBeInTheDocument();
    expect(screen.getByText("MOV")).toBeInTheDocument();
    expect(document.querySelector("img")).toBeNull();
  });

  it("tags RAW, HDR and Live Photos, and keeps them on narrow tiles (CSS steps them down)", () => {
    const props = { top: 0, selected: false, selecting: false, onOpen: vi.fn(), onToggle: vi.fn() };
    const { rerender } = render(<PhotoTile tile={{ ...base, raw: true, hdr: 2, extension: "nef" }} {...props} />);
    expect(screen.getByText("RAW")).toBeInTheDocument();
    expect(screen.getByText("HDR")).toBeInTheDocument();
    rerender(<PhotoTile tile={{ ...base, mediaType: 2 }} {...props} />);
    expect(screen.getByText("LIVE")).toBeInTheDocument();
    expect(screen.queryByText("RAW")).toBeNull();
    rerender(<PhotoTile tile={{ ...base, raw: true, w: 70 }} {...props} />);
    expect(screen.getByText("RAW")).toBeInTheDocument();
  });

  it("uses the 1024px thumbnail for tall rows", () => {
    render(<PhotoTile tile={{ ...base, h: 320 }} top={0} selected selecting onOpen={vi.fn()} onToggle={vi.fn()} />);
    expect(document.querySelector("img")!.getAttribute("src")).toContain("s=1024");
    expect(document.querySelector(".photo-tile")).toHaveAttribute("data-selected", "true");
  });
});

describe("tilePropsEqual", () => {
  /** A grid refetch hands every tile a new but identical object; that must not re-render it. */
  it("treats a new but identical tile as unchanged, and anything else as a change", async () => {
    const { tilePropsEqual } = await import("./gui");
    const tile = { id: 1, x: 0, y: 0, w: 100, h: 80, mediaType: 0, thumbhash: "abc", dominantColor: "#fff", isFavorite: false, durationMs: null, rating: 0, trashedAt: null } as never;
    const onOpen = () => {};
    const base = { tile, top: 0, selected: false, selecting: false, onOpen, onToggle: onOpen };
    expect(tilePropsEqual(base, { ...base, tile: { ...(tile as object) } as never })).toBe(true);
    expect(tilePropsEqual(base, { ...base, tile: { ...(tile as object), thumbhash: "abd" } as never })).toBe(false);
    expect(tilePropsEqual(base, { ...base, selected: true })).toBe(false);
    expect(tilePropsEqual(base, { ...base, onOpen: () => {} })).toBe(false);
  });
});

describe("a Years square", () => {
  const props = { top: 0, selected: false, selecting: false, onOpen: vi.fn(), onToggle: vi.fn(), compact: true };

  it("waits for its image while the view flies past, and keeps it once started", () => {
    const { rerender } = render(<PhotoTile tile={base} {...props} hold />);
    expect(document.querySelector("img"), "held: its colour shows").toBeNull();
    rerender(<PhotoTile tile={base} {...props} hold={false} />);
    expect(document.querySelector("img")).not.toBeNull();
    rerender(<PhotoTile tile={base} {...props} hold />);
    expect(document.querySelector("img"), "a started image is never dropped").not.toBeNull();
  });

  it("renders no badges, and its checkbox only while selecting", () => {
    const fav = { ...base, isFavorite: true, mediaType: 1, durationMs: 4000, raw: true, w: 200 };
    const { rerender } = render(<PhotoTile tile={fav} {...props} />);
    expect(document.querySelector(".photo-tile__badge, .photo-tile__star, .photo-tile__tags")).toBeNull();
    expect(screen.queryByRole("button", { name: "Select" })).toBeNull();
    rerender(<PhotoTile tile={fav} {...props} selecting />);
    expect(screen.getByRole("button", { name: "Select" })).toBeInTheDocument();
  });
});

describe("a tile passed while the scrubber is dragged", () => {
  // A real ThumbHash of a 4×4 grey square, so the placeholder decodes.
  const hash = btoa(String.fromCharCode(...rgbaToThumbHash(4, 4, new Uint8Array(64).fill(128))));
  const props = { top: 0, selected: false, selecting: false, onOpen: vi.fn(), onToggle: vi.fn() };
  const tile = { ...base, thumbhash: hash };
  const tileEl = () => document.querySelector(".photo-tile") as HTMLElement;

  it("shows its colour, not its ThumbHash, until the drag ends", () => {
    const { rerender } = render(<PhotoTile tile={tile} {...props} hold />);
    expect(tileEl().style.backgroundImage).toBe("");
    rerender(<PhotoTile tile={tile} {...props} hold={false} />);
    expect(tileEl().style.backgroundImage).toContain("data:image/png");
  });

  // Round 22 lane C: the app a copy was made for, where the RAW tag sits.
  it("shows the app's icon on a photo made for an editor, and a plain icon once it is gone", async () => {
    mockCommand("get_settings", () => null);
    mockCommand("list_external_apps", () => [{ id: "photoshop", name: "Adobe Photoshop", kind: "editor", accepts: ["photo"], iconUrl: "photos://app-icon/3", custom: false, send: "automatic" }]);
    mockCommand("get_default_apps", () => ({ photo: null, raw: null, video: null }));
    const props = { top: 0, selected: false, selecting: false, onOpen: vi.fn(), onToggle: vi.fn() };
    const { rerender } = render(
      <Providers>
        <PhotoTile tile={{ ...base, editedIn: "photoshop" }} {...props} />
      </Providers>,
    );
    const badge = await screen.findByTitle("Edited in Adobe Photoshop");
    await waitFor(() => expect(badge.querySelector("img")).toHaveAttribute("src", "photos://app-icon/3"));
    rerender(
      <Providers>
        <PhotoTile tile={{ ...base, editedIn: "gone" }} {...props} />
      </Providers>,
    );
    expect(await screen.findByTitle("Edited in another app")).toBeInTheDocument();
    expect(screen.queryByTitle(/Edited in Adobe/)).toBeNull();
  });
});
