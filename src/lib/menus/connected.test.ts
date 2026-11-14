import { describe, expect, it, vi } from "vitest";
import { connectedMenu, labels, mapMenu } from "./index";

const actions = () => ({ open: vi.fn(), browse: vi.fn(), importNew: vi.fn(), importAgain: vi.fn(), eject: vi.fn(), forget: vi.fn() });

describe("device and map menus", () => {
  it("gives Eject and Forget only to cards and drives", () => {
    expect(labels(connectedMenu("volume", actions()))).toEqual(["Open", "Browse…", "Import new", "Import everything again", "—", "Eject", "Forget this device"]);
    expect(labels(connectedMenu("iphone", actions()))).toEqual(["Open", "Browse…", "Import new"]);
    expect(labels(connectedMenu("mac", actions()))).toEqual(["Open", "Browse…"]);
  });

  it("offers a group of pins a slideshow, and one photo no slideshow of one", () => {
    const a = { open: vi.fn(), slideshow: vi.fn(), addToAlbum: vi.fn(), zoomToFit: vi.fn() };
    expect(labels(mapMenu(false, a))).toEqual(["Open photos here", "Start slideshow of these", "Add to album…", "Zoom to fit"]);
    expect(labels(mapMenu(true, a))).toEqual(["Open photo", "Add to album…", "Zoom to fit"]);
  });
});
