import { describe, expect, it, vi } from "vitest";
import type { AssetFace } from "@/types/generated/AssetFace";
import type { AssetPet } from "@/types/generated/AssetPet";
import type { PetSummary } from "@/types/generated/PetSummary";
import type { MemorySummary } from "@/types/generated/MemorySummary";
import { faceMenu, importRowMenu, labels, memoryMenu, personMenu, petBoxMenu, petMenu, setMenu, slideshowMenu, type MenuItem } from "./index";

const run = (items: MenuItem[], label: string) => {
  const it = items.find((i) => "label" in i && i.label === label);
  if (!it || !("onSelect" in it)) throw new Error(`no item ${label} in ${labels(items).join(", ")}`);
  it.onSelect();
};

const memory = { id: 3, hidden: false } as MemorySummary;
const face = (over: Partial<AssetFace> = {}): AssetFace => ({ id: 9, personId: 2, name: "Anna", x: 0, y: 0, w: 0.1, h: 0.1, manual: false, ...over }) as AssetFace;

describe("library menus", () => {
  it("gives a memory Open, Export and Hide, and Play and Make slideshow only once they are handed over", () => {
    const a = { open: vi.fn(), exportPhotos: vi.fn(), setHidden: vi.fn() };
    expect(labels(memoryMenu(memory, a))).toEqual(["Open", "Export photos…", "—", "Hide memory"]);
    run(memoryMenu(memory, a), "Hide memory");
    expect(a.setHidden).toHaveBeenCalledWith(true);
    expect(labels(memoryMenu({ ...memory, hidden: true }, a)).at(-1)).toBe("Show memory again");
    const withFilms = { ...a, play: vi.fn(), makeSlideshow: vi.fn(), exportVideo: vi.fn() };
    expect(labels(memoryMenu(memory, withFilms))).toEqual(["Play", "Open", "Make slideshow", "Export video…", "Export photos…", "—", "Hide memory"]);
  });

  it("gives a person the People page's actions, the danger last", () => {
    const a = { open: vi.fn(), rename: vi.fn(), merge: vi.fn(), notAPerson: vi.fn(), remove: vi.fn() };
    const items = personMenu({ id: 2, name: null, faceCount: 3, photoCount: 3, coverFaceId: null }, a);
    expect(labels(items)).toEqual(["Open", "Add a name", "Merge with…", "—", "Not a person", "Remove from People"]);
    expect(items.at(-1)).toMatchObject({ danger: true });
    run(items, "Not a person");
    expect(a.notAPerson).toHaveBeenCalled();
  });

  it("names the person on a face box's own items, and leaves them off an unknown face", () => {
    const a = { tag: vi.fn(), newPerson: vi.fn(), untag: vi.fn(), useAsMain: vi.fn(), notAFace: vi.fn() };
    expect(labels(faceMenu(face(), a))).toEqual(["Tag as someone else…", "New person…", "Use as main photo of Anna", "—", "Not Anna", "Not a face"]);
    expect(labels(faceMenu(face({ personId: null, name: null }), a))).toEqual(["Tag…", "New person…", "—", "Not a face"]);
    run(faceMenu(face(), a), "Not Anna");
    expect(a.untag).toHaveBeenCalled();
  });

  it("gives a pet card the Pets page's actions, the danger last (Round 8B)", () => {
    const a = { open: vi.fn(), rename: vi.fn(), merge: vi.fn(), notAPet: vi.fn(), remove: vi.fn() };
    const pet = { id: 4, name: null, species: "dog", detectionCount: 3, photoCount: 3, coverDetectionId: null } as PetSummary;
    const items = petMenu(pet, a);
    expect(labels(items)).toEqual(["Open", "Add a name", "Merge with…", "—", "Not a pet", "Remove from Pets"]);
    expect(items.at(-1)).toMatchObject({ danger: true });
    expect(labels(petMenu({ ...pet, name: "Milo" }, { ...a, select: vi.fn(), canMerge: false }))).toEqual(["Open", "Select", "Rename", "Merge with…", "—", "Not a pet", "Remove from Pets"]);
    expect(petMenu(pet, { ...a, canMerge: false }).find((i) => "label" in i && i.label === "Merge with…")).toMatchObject({ disabled: true });
    run(items, "Not a pet");
    expect(a.notAPet).toHaveBeenCalled();
  });

  it("names the pet on a pet box's own items, and leaves them off an unknown pet (Round 8B)", () => {
    const a = { tag: vi.fn(), newPet: vi.fn(), untag: vi.fn(), useAsMain: vi.fn(), open: vi.fn(), notAPet: vi.fn() };
    const box = (over: Partial<AssetPet> = {}): AssetPet => ({ id: 7, petId: 4, name: "Milo", species: "dog", x: 0, y: 0, w: 0.2, h: 0.2, ...over });
    expect(labels(petBoxMenu(box(), a))).toEqual(["Open Milo", "Tag as another dog…", "New pet…", "Use as main photo of Milo", "—", "Not Milo", "Not a pet"]);
    expect(labels(petBoxMenu(box({ petId: null, name: null, species: "cat" }), a))).toEqual(["Tag as…", "New pet…", "—", "Not a pet"]);
    const { open: _open, ...viewer } = a;
    expect(labels(petBoxMenu(box(), viewer))[0]).toBe("Tag as another dog…");
    run(petBoxMenu(box(), a), "Not Milo");
    expect(a.untag).toHaveBeenCalled();
    run(petBoxMenu(box(), a), "Not a pet");
    expect(a.notAPet).toHaveBeenCalled();
  });

  it("gives a slideshow its row actions, with Delete last", () => {
    const a = { open: vi.fn(), duplicate: vi.fn(), rename: vi.fn(), delete: vi.fn() };
    expect(labels(slideshowMenu(a))).toEqual(["Open", "Duplicate", "Rename…", "—", "Delete"]);
    expect(labels(slideshowMenu({ ...a, play: vi.fn(), export: vi.fn() }))).toEqual(["Open", "Play", "Export…", "Duplicate", "Rename…", "—", "Delete"]);
  });

  it("offers an import row only what applies to it", () => {
    const dismiss = vi.fn();
    expect(labels(importRowMenu({ dismiss }))).toEqual(["Dismiss"]);
    expect(labels(importRowMenu({ showInFolder: vi.fn(), retryFailed: vi.fn(), copyErrors: vi.fn(), dismiss }))).toEqual([
      "Show in folder",
      "Retry failed",
      "Copy error text",
      "—",
      "Dismiss",
    ]);
  });

  it("gives a set of photos Open and what else it was handed", () => {
    expect(labels(setMenu({ open: vi.fn(), slideshow: vi.fn(), export: vi.fn() }))).toEqual(["Open", "Start slideshow", "Export…"]);
    expect(labels(setMenu({ open: vi.fn(), develop: vi.fn(), slideshow: vi.fn() }))).toEqual(["Open", "Open in Develop", "Start slideshow"]);
    expect(labels(setMenu({ open: vi.fn() }))).toEqual(["Open"]);
  });
});
