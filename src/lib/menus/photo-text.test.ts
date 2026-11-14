import { describe, expect, it, vi } from "vitest";
import { labels } from "./parts";
import { photoTextMenu } from "./photo-text";

const actions = () => ({ copy: vi.fn(), copyAll: vi.fn(), search: vi.fn() });

describe("photoTextMenu", () => {
  it("offers Copy, Copy all and a search for a line", () => {
    const a = actions();
    const items = photoTextMenu({ text: "Gostilna pri Mostu" }, a);
    expect(labels(items)).toEqual(["Copy", "Copy all", "—", "Search for “Gostilna pri Mostu”"]);
    const search = items[3];
    if ("onSelect" in search) search.onSelect();
    expect(a.search).toHaveBeenCalledWith("Gostilna pri Mostu");
  });

  it("offers only Copy all away from a line", () => {
    expect(labels(photoTextMenu({ text: null }, actions()))).toEqual(["Copy all"]);
  });
});
