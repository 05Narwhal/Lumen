import { describe, expect, it, vi } from "vitest";
import { contributionItems, labels, photoMenu, viewerMenu, type MenuContribution, type MenuItem } from "./index";

const run = (items: MenuItem[], label: string) => {
  const it = items.find((i) => "label" in i && i.label === label);
  if (!it || !("onSelect" in it)) throw new Error(`no item ${label} in ${labels(items).join(", ")}`);
  it.onSelect();
};

const entry = (label: string, over: Partial<MenuContribution> = {}): MenuContribution => ({ id: `p:${label}`, label, icon: null, run: vi.fn(), ...over });

const photoActions = () => ({
  favorite: vi.fn(),
  addToAlbum: vi.fn(),
  rotate: vi.fn(),
  reveal: vi.fn(),
  copyTo: vi.fn(),
  moveToHidden: vi.fn(),
  trash: vi.fn(),
});
const photoCtx = { favorite: false, inHidden: false, mac: false };

const viewerActions = () => ({
  favorite: vi.fn(),
  rate: vi.fn(),
  addToAlbum: vi.fn(),
  rotate: vi.fn(),
  export: vi.fn(),
  reveal: vi.fn(),
  showInfo: vi.fn(),
  moveToHidden: vi.fn(),
  trash: vi.fn(),
});
const viewerCtx = { favorite: false, rating: 0, mac: false, inHidden: false, inAlbum: false, people: [] };

describe("contributionItems", () => {
  it("is_empty_without_contributions, and one item per entry otherwise", () => {
    expect(contributionItems(undefined)).toEqual([]);
    expect(contributionItems([])).toEqual([]);
    expect(labels(contributionItems([entry("A"), entry("B")]))).toEqual(["A", "B"]);
  });

  it("selecting_an_item_runs_its_entry", () => {
    const e = entry("A");
    run(contributionItems([e]), "A");
    expect(e.run).toHaveBeenCalledTimes(1);
  });
});

describe("photo menu contributions", () => {
  it("a_selection_menu_lists_them_before_More_and_Trash, set apart by dividers", () => {
    const l = labels(photoMenu([1, 2], { ...photoCtx, contributions: [entry("Check these")] }, photoActions()));
    const i = l.indexOf("Check these");
    expect(i).toBeGreaterThan(l.indexOf("Add to album…"));
    expect(l[i - 1]).toBe("—");
    expect(l[i + 1]).toBe("—");
    expect(i).toBeLessThan(l.indexOf("More"));
    expect(l[l.length - 1]).toMatch(/Trash/);
  });

  it("without_contributions_the_menu_is_what_it_was", () => {
    const base = labels(photoMenu([1], photoCtx, photoActions()));
    expect(labels(photoMenu([1], { ...photoCtx, contributions: [] }, photoActions()))).toEqual(base);
  });

  it("selecting_one_runs_it", () => {
    const e = entry("Check these");
    run(photoMenu([1], { ...photoCtx, contributions: [e] }, photoActions()), "Check these");
    expect(e.run).toHaveBeenCalledTimes(1);
  });
});

describe("viewer menu contributions", () => {
  it("lists_them_after_the_photo_actions_and_before_Show_info", () => {
    const l = labels(viewerMenu({ ...viewerCtx, contributions: [entry("Look closer"), entry("Other")] }, viewerActions()));
    expect(l.indexOf("Look closer")).toBeGreaterThan(l.indexOf("Show original in folder"));
    expect(l.indexOf("Other")).toBe(l.indexOf("Look closer") + 1);
    expect(l.indexOf("Other")).toBeLessThan(l.indexOf("Show info"));
    expect(l[l.length - 1]).toBe("Move to Trash");
  });

  it("without_contributions_the_menu_is_what_it_was", () => {
    const base = labels(viewerMenu(viewerCtx, viewerActions()));
    expect(labels(viewerMenu({ ...viewerCtx, contributions: [] }, viewerActions()))).toEqual(base);
  });
});

describe("the photos a menu entry works on", () => {
  it("a_photo_menu_hands_the_selected_ids_to_the_entry", () => {
    const e = entry("Check these");
    run(photoMenu([4, 5, 6], { ...photoCtx, contributions: [e] }, photoActions()), "Check these");
    expect(e.run).toHaveBeenCalledWith([4, 5, 6]);
  });
});
