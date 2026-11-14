import { describe, expect, it, vi } from "vitest";
import type { ExternalApp } from "@/types/generated/ExternalApp";
import { developCanvasMenu, labels, viewerMenu, type MenuItem } from "./index";

const find = (items: MenuItem[], label: string) => items.find((i) => "label" in i && i.label === label);
const run = (items: MenuItem[], label: string) => {
  const it = find(items, label);
  if (!it || !("onSelect" in it)) throw new Error(`no item ${label} in ${labels(items).join(", ")}`);
  it.onSelect();
};
const sub = (items: MenuItem[], label: string) => {
  const it = find(items, label);
  if (!it || !("children" in it)) throw new Error(`no submenu ${label}`);
  return it.children;
};

const viewerActions = () => ({
  favorite: vi.fn(),
  rate: vi.fn(),
  addToAlbum: vi.fn(),
  rotate: vi.fn(),
  copyImage: vi.fn(),
  export: vi.fn(),
  develop: vi.fn(),
  reveal: vi.fn(),
  setAlbumCover: vi.fn(),
  useAsMainPhoto: vi.fn(),
  tagPeople: vi.fn(),
  showInfo: vi.fn(),
  moveToHidden: vi.fn(),
  trash: vi.fn(),
});
const ctx = { favorite: false, rating: 2, mac: false, inHidden: false, inAlbum: false, people: [] };

describe("viewer and Develop menus", () => {
  it("offers Set as album cover only when the viewer came from an album", () => {
    const a = viewerActions();
    expect(labels(viewerMenu(ctx, a))).not.toContain("Set as album cover");
    const inAlbum = viewerMenu({ ...ctx, inAlbum: true }, a);
    run(inAlbum, "Set as album cover");
    expect(a.setAlbumCover).toHaveBeenCalled();
  });

  it("copies the photo, rates it and makes someone's main photo from it", () => {
    const a = viewerActions();
    const items = viewerMenu({ ...ctx, people: [{ personId: 3, name: "Anna", faceId: 30 }] }, a);
    expect(labels(items)).toEqual([
      "Rate",
      "Add to Favorites",
      "Add to album…",
      "Rotate left",
      "Rotate right",
      "—",
      "Copy image",
      "Export…",
      "Develop",
      "Show original in folder",
      "Use as main photo of",
      "Tag people…",
      "Show info",
      "Move to Hidden",
      "—",
      "Move to Trash",
    ]);
    run(items, "Copy image");
    expect(a.copyImage).toHaveBeenCalled();
    const rate = items.find((i) => "onRate" in i && i.label === "Rate");
    if (rate && "onRate" in rate) rate.onRate(5);
    expect(a.rate).toHaveBeenCalledWith(5);
    run(sub(items, "Use as main photo of"), "Anna");
    expect(a.useAsMainPhoto).toHaveBeenCalledWith(3, 30);
    // A video has no still to copy or develop.
    const video = viewerMenu(ctx, { ...a, copyImage: undefined, develop: undefined });
    expect(labels(video)).not.toContain("Copy image");
    expect(labels(video)).not.toContain("Develop");
  });

  it("drives Develop's canvas through the session's own functions", () => {
    const a = {
      setCompare: vi.fn(),
      setClipping: vi.fn(),
      zoom: vi.fn(),
      copySettings: vi.fn(),
      pasteSettings: vi.fn(),
      reset: vi.fn(),
      flag: vi.fn(),
      label: vi.fn(),
      export: vi.fn(),
    };
    const items = developCanvasMenu({ compare: "split", clipping: false, labels: [{ value: 1, name: "Red" }] }, a);
    expect(labels(items)).toEqual(["Before / after", "Show clipping", "Zoom", "—", "Copy settings…", "Paste settings", "Reset develop", "—", "Flag", "Label", "Export…"]);
    // The mode on now is ticked, and choosing it again turns it off.
    run(sub(items, "Before / after"), "Split ✓");
    expect(a.setCompare).toHaveBeenCalledWith("off");
    run(sub(items, "Before / after"), "Side by side");
    expect(a.setCompare).toHaveBeenCalledWith("side");
    run(items, "Show clipping");
    expect(a.setClipping).toHaveBeenCalledWith(true);
    run(sub(items, "Zoom"), "100 %");
    expect(a.zoom).toHaveBeenCalledWith(1);
    run(sub(items, "Flag"), "Reject");
    expect(a.flag).toHaveBeenCalledWith(-1);
    run(sub(items, "Label"), "Red");
    expect(a.label).toHaveBeenCalledWith(1);
    run(items, "Paste settings");
    expect(a.pasteSettings).toHaveBeenCalled();
  });

  // Round 22 §9: both menus take Edit in <default> and Open with ▸ from the one shared factory.
  it("gives the viewer's and Develop's menus the same two app entries", () => {
    const ps: ExternalApp = { id: "ps", name: "Photoshop", kind: "editor", accepts: ["photo"], iconUrl: "photos://app-icon/1", custom: false, send: "automatic", writes: "inPlace", addedBy: null };
    const openWith = { apps: [ps], defaultApp: ps, mac: false };
    const actions = { editIn: vi.fn(), chooseApp: vi.fn() };
    const viewer = viewerMenu({ ...ctx, openWith }, { ...viewerActions(), openWith: actions });
    const develop = developCanvasMenu({ compare: "off", clipping: false, labels: [], openWith }, {
      setCompare: vi.fn(), setClipping: vi.fn(), zoom: vi.fn(), copySettings: vi.fn(), pasteSettings: vi.fn(), reset: vi.fn(), flag: vi.fn(), label: vi.fn(), export: vi.fn(), openWith: actions,
    });
    for (const items of [viewer, develop]) {
      expect(labels(items)).toEqual(expect.arrayContaining(["Edit in Photoshop", "Open with"]));
      run(items, "Edit in Photoshop");
      expect(actions.editIn).toHaveBeenLastCalledWith("ps");
      run(sub(items, "Open with"), "Choose another app…");
    }
    expect(actions.chooseApp).toHaveBeenCalledTimes(2);
    // Nothing installed or no context: the menus are as they were.
    expect(labels(viewerMenu(ctx, viewerActions()))).not.toContain("Open with");
  });
});
