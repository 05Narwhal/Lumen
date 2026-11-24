import { describe, expect, it, vi } from "vitest";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { ExternalApp } from "@/types/generated/ExternalApp";
import { albumMenu, albumPageMenu, collectionMenu, deviceMenu, DIVIDER, labels, photoMenu, tidy, type MenuItem } from "./index";

const pick = (items: MenuItem[], label: string) => {
  const it = items.find((i) => "label" in i && i.label === label);
  if (!it || !("onSelect" in it)) throw new Error(`no item ${label} in ${labels(items).join(", ")}`);
  return it;
};

const subOf = (items: MenuItem[], label: string) => {
  const it = items.find((i) => "label" in i && i.label === label);
  if (!it || !("children" in it)) throw new Error(`no submenu ${label} in ${labels(items).join(", ")}`);
  return it.children;
};

const app = (id: string, name: string): ExternalApp => ({ id, name, kind: "editor", accepts: ["photo", "raw"], iconUrl: `photos://app-icon/${id}`, custom: false, send: "automatic", writes: "inPlace", addedBy: null });

const album = (over: Partial<AlbumSummary> = {}): AlbumSummary => ({
  id: 4,
  name: "Lisbon",
  kind: 0,
  parentId: null,
  coverAssetId: null,
  itemCount: 12,
  createdAt: 1,
  updatedAt: 1,
  query: null,
  hidesItems: false,
  collectionId: null,
  ...over,
} as AlbumSummary);

const photoActions = () => ({
  open: vi.fn(),
  favorite: vi.fn(),
  addToAlbum: vi.fn(),
  tagPeople: vi.fn(),
  rotate: vi.fn(),
  reveal: vi.fn(),
  copyTo: vi.fn(),
  moveToHidden: vi.fn(),
  trash: vi.fn(),
});

describe("menu factories", () => {
  it("tidies away empty entries and stray dividers", () => {
    expect(labels(tidy([DIVIDER, false, { label: "A", onSelect: () => {} }, DIVIDER, null, DIVIDER, { label: "B", onSelect: () => {} }, DIVIDER]))).toEqual(["A", "—", "B"]);
  });

  it("offers one photo everything, and the trash last", () => {
    const a = photoActions();
    const items = photoMenu([7], { favorite: false, inHidden: false, mac: false }, a);
    expect(labels(items)).toEqual(["Open", "Add to Favorites", "Add to album…", "—", "More", "—", "Move to Trash"]);
    expect(pick(items, "Move to Trash")).toMatchObject({ danger: true });
    pick(subOf(items, "More"), "Rotate right").onSelect();
    expect(a.rotate).toHaveBeenCalledWith([7], 1);
  });

  it("offers Develop right after Open for one photo, never for a selection", () => {
    const a = { ...photoActions(), develop: vi.fn() };
    const items = photoMenu([7], { favorite: false, inHidden: false, mac: false }, a);
    expect(labels(items).slice(0, 2)).toEqual(["Open", "Develop"]);
    pick(items, "Develop").onSelect();
    expect(a.develop).toHaveBeenCalledWith(7);
    expect(labels(photoMenu([7, 8], { favorite: false, inHidden: false, mac: false }, a))).not.toContain("Develop");
  });

  // Round 22 §9: Edit in <default> and Open with ▸, the rarer actions under More.
  it("puts the default app and Open with ▸ under Add to album, and the rest under More", () => {
    const a = { ...photoActions(), rate: vi.fn(), openWith: { editIn: vi.fn(), chooseApp: vi.fn() } };
    const openWith = { apps: [app("photoshop", "Photoshop"), app("gimp", "GIMP")], defaultApp: app("photoshop", "Photoshop"), mac: false };
    const items = photoMenu([7], { favorite: false, inHidden: false, mac: false, openWith }, a);
    expect(labels(items)).toEqual(["Rate", "Open", "Add to Favorites", "Add to album…", "—", "Edit in Photoshop", "Open with", "—", "More", "—", "Move to Trash"]);
    expect(pick(items, "Edit in Photoshop")).toMatchObject({ iconUrl: "photos://app-icon/photoshop", shortcut: "Mod+Shift+E" });
    expect(labels(subOf(items, "Open with"))).toEqual(["Photoshop", "GIMP", "—", "Show in Explorer", "Copy files to…", "—", "Choose another app…"]);
    expect(labels(subOf(items, "More"))).toEqual(["Rotate left", "Rotate right", "Tag people…", "Move to Hidden"]);
    pick(items, "Edit in Photoshop").onSelect();
    expect(a.openWith.editIn).toHaveBeenCalledWith([7], "photoshop");
    pick(subOf(items, "Open with"), "GIMP").onSelect();
    expect(a.openWith.editIn).toHaveBeenLastCalledWith([7], "gimp");
    pick(subOf(items, "Open with"), "Choose another app…").onSelect();
    expect(a.openWith.chooseApp).toHaveBeenCalledWith([7]);
    pick(subOf(items, "Open with"), "Show in Explorer").onSelect();
    expect(a.reveal).toHaveBeenCalledWith(7);
  });

  it("leaves Edit in out without a default, and Move to Hidden out of More inside Hidden", () => {
    const a = { ...photoActions(), openWith: { editIn: vi.fn(), chooseApp: vi.fn() } };
    const openWith = { apps: [app("gimp", "GIMP")], defaultApp: null, mac: true };
    const items = photoMenu([7], { favorite: false, inHidden: true, mac: true, openWith }, a);
    expect(labels(items)).toEqual(["Open", "Add to Favorites", "Add to album…", "—", "Open with", "—", "More", "—", "Move to Trash"]);
    expect(labels(subOf(items, "Open with"))).toEqual(["GIMP", "—", "Reveal in Finder", "Copy files to…", "—", "Choose another app…"]);
    expect(labels(subOf(items, "More"))).toEqual(["Rotate left", "Rotate right", "Tag people…"]);
  });

  it("acts on a whole selection when one of several selected tiles is right-clicked", () => {
    const a = photoActions();
    const items = photoMenu([1, 2, 3], { favorite: false, inHidden: true, mac: true }, a);
    expect(labels(items)).toEqual(["Add 3 to Favorites", "Add to album…", "—", "More", "—", "Move 3 to Trash"]);
    expect(labels(subOf(items, "More"))).toEqual(["Rotate left", "Rotate right"]);
    pick(items, "Move 3 to Trash").onSelect();
    expect(a.trash).toHaveBeenCalledWith([1, 2, 3]);
  });

  it("gives an album its menu, with the danger item last", () => {
    const a = { open: vi.fn(), rename: vi.fn(), editRules: vi.fn(), setCover: vi.fn(), setHides: vi.fn(), duplicate: vi.fn(), export: vi.fn(), delete: vi.fn() };
    const items = albumMenu(album(), a);
    expect(labels(items)).toEqual([
      "Open",
      "Rename…",
      "Set cover from…",
      "Hide its photos from the Library",
      "Duplicate album",
      "Export album…",
      "—",
      "Delete album",
    ]);
    pick(items, "Hide its photos from the Library").onSelect();
    expect(a.setHides).toHaveBeenCalledWith(true);
    // A smart album has rules instead of a name to edit, and no photos of its own to hide.
    expect(labels(albumMenu(album({ kind: 1 }), a))).toEqual(["Open", "Edit rules…", "Set cover from…", "Duplicate album", "Export album…", "—", "Delete album"]);
    // An empty album has no cover to pick and nothing to export.
    expect(labels(albumMenu(album({ itemCount: 0, hidesItems: true }), a))).toEqual(["Open", "Rename…", "Show its photos in the Library", "Duplicate album", "—", "Delete album"]);
  });

  // Julian's 2026-11-08 list, item 3: an icon cover, for an empty album too.
  it("offers Cover… on albums and collections that can change it", () => {
    const a = { open: vi.fn(), rename: vi.fn(), setHides: vi.fn(), delete: vi.fn(), setCover: vi.fn(), editCover: vi.fn() };
    expect(labels(albumMenu(album(), a)).slice(0, 4)).toEqual(["Open", "Rename…", "Set cover from…", "Cover…"]);
    expect(labels(albumMenu(album({ itemCount: 0 }), a))).toContain("Cover…");
    pick(albumMenu(album(), a), "Cover…").onSelect();
    expect(a.editCover).toHaveBeenCalled();
    // Distinct album covers (2026-11-09): new automatic prints, for an album showing photos.
    const s = { ...a, shuffleCover: vi.fn() };
    expect(labels(albumMenu(album(), s)).slice(0, 5)).toEqual(["Open", "Rename…", "Set cover from…", "Cover…", "Shuffle cover"]);
    pick(albumMenu(album(), s), "Shuffle cover").onSelect();
    expect(s.shuffleCover).toHaveBeenCalled();
    expect(labels(albumMenu(album({ itemCount: 1 }), s))).not.toContain("Shuffle cover");
    expect(labels(albumMenu(album({ coverStyle: { kind: "icon", shape: "box", icon: "star", color: "swatch-4" } }), s))).not.toContain("Shuffle cover");
    const c = { open: vi.fn(), newInside: vi.fn(), rename: vi.fn(), editCover: vi.fn(), move: vi.fn(), moveOut: vi.fn(), delete: vi.fn() };
    expect(labels(collectionMenu({ inside: false }, c))).toEqual(["Open", "New collection inside…", "Rename…", "Cover…", "Move to collection…", "—", "Delete collection"]);
  });

  it("moves an album into a collection or out of one", () => {
    const a = { open: vi.fn(), rename: vi.fn(), setHides: vi.fn(), delete: vi.fn(), moveToCollection: vi.fn(), removeFromCollection: vi.fn() };
    expect(labels(albumMenu(album({ itemCount: 0 }), a))).toEqual(["Open", "Rename…", "Hide its photos from the Library", "Move to collection…", "—", "Delete album"]);
    const inOne = albumMenu(album({ itemCount: 0, collectionId: 3 }), a);
    expect(labels(inOne)).toEqual(["Open", "Rename…", "Hide its photos from the Library", "Move to collection…", "Remove from collection", "—", "Delete album"]);
    pick(inOne, "Remove from collection").onSelect();
    expect(a.removeFromCollection).toHaveBeenCalled();
  });

  it("folds an album page's order and hiding switch into one menu", () => {
    const a = { setByDate: vi.fn(), setHides: vi.fn() };
    const manual = albumPageMenu(album(), { byDate: false, ...a });
    expect(labels(manual)).toEqual(["Sort by date", "Hide its photos from the Library"]);
    pick(manual, "Sort by date").onSelect();
    expect(a.setByDate).toHaveBeenCalledWith(true);
    pick(manual, "Hide its photos from the Library").onSelect();
    expect(a.setHides).toHaveBeenCalledWith(true);
    const dated = albumPageMenu(album({ hidesItems: true }), { byDate: true, ...a });
    expect(labels(dated)).toEqual(["Sort manually", "Show its photos in the Library"]);
    pick(dated, "Show its photos in the Library").onSelect();
    expect(a.setHides).toHaveBeenLastCalledWith(false);
  });

  it("gives a collection Open, New collection inside, Rename, Move and Delete", () => {
    const a = { open: vi.fn(), newInside: vi.fn(), rename: vi.fn(), move: vi.fn(), moveOut: vi.fn(), delete: vi.fn() };
    expect(labels(collectionMenu({ inside: true }, a))).toEqual(["Open", "New collection inside…", "Rename…", "Move to collection…", "Move to the top", "—", "Delete collection"]);
    expect(labels(collectionMenu({ inside: false }, a))).toEqual(["Open", "New collection inside…", "Rename…", "Move to collection…", "—", "Delete collection"]);
    pick(collectionMenu({ inside: false }, a), "Delete collection").onSelect();
    expect(a.delete).toHaveBeenCalled();
  });

  it("gives a library device Open, Rename and Hide", () => {
    const a = { open: vi.fn(), rename: vi.fn(), hide: vi.fn() };
    const items = deviceMenu(a);
    expect(labels(items)).toEqual(["Open", "Rename…", "—", "Hide device"]);
    pick(items, "Hide device").onSelect();
    expect(a.hide).toHaveBeenCalled();
  });
});
