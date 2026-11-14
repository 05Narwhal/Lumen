import { describe, expect, it, vi } from "vitest";
import type { AiMaskInfo } from "@/types/generated/AiMaskInfo";
import type { AiMaskList } from "@/types/generated/AiMaskList";
import type { Mask } from "@/types/generated/Mask";
import { addAiShape, aiLabel, missingRasters, newAiMask, parseRaster, retarget } from "./ai-masks";
import { aiMaskMenu } from "./menus/masks";
import { labels } from "./menus/parts";

const info = (kind: AiMaskInfo["kind"], instance = 0, hash = `${kind}${instance}`.padEnd(32, "0")): AiMaskInfo => ({ hash, kind, instance, model: "facet@1", score: 0.97, width: 4, height: 3 });
const list = (masks: AiMaskInfo[], scanned = true): AiMaskList => ({ available: true, scanned, masks });

describe("AI masks", () => {
  it("names people from one, the rest by kind", () => {
    expect(aiLabel(info("sky"))).toBe("Sky");
    expect(aiLabel(info("person", 2))).toBe("Person 3");
  });

  it("makes a mask of one AI shape, named after it", () => {
    const m = newAiMask([], info("subject"));
    expect(m.name).toBe("Subject");
    expect(m.shapes).toEqual([{ kind: "ai", add: true, target: "subject", instance: 0, raster: info("subject").hash }]);
    const two = addAiShape([m], m.id, info("person", 1), false);
    expect(two[0].shapes[1]).toEqual({ kind: "ai", add: false, target: "person", instance: 1, raster: info("person", 1).hash });
  });

  it("points a mask's AI shapes at this photo's rasters of the same target", () => {
    const pasted: Mask = { ...newAiMask([], info("sky", 0, "a".repeat(32))), shapes: [{ kind: "ai", add: true, target: "sky", instance: 0, raster: "a".repeat(32) }, { kind: "linear", add: true, from: [0, 0], to: [1, 1] }] };
    const here = retarget(pasted, list([info("sky")]));
    expect(here.shapes[0]).toMatchObject({ raster: info("sky").hash });
    expect(here.shapes[1]).toBe(pasted.shapes[1]);
    // Nothing of that target here: left as it was, still missing.
    expect(retarget(pasted, list([info("subject")]))).toBe(pasted);
  });

  it("lists the AI shapes whose raster this photo doesn't have", () => {
    const m = newAiMask([], info("sky"));
    expect(missingRasters(m, { [info("sky").hash]: "missing" })).toEqual([0]);
    expect(missingRasters(m, { [info("sky").hash]: { width: 1, height: 1, weights: [9] } })).toEqual([]);
    // Still loading is not missing.
    expect(missingRasters(m, {})).toEqual([]);
  });

  it("reads the route's size header", () => {
    const bytes = new Uint8Array([3, 0, 0, 0, 2, 0, 0, 0, 1, 2, 3, 4, 5, 6]);
    const r = parseRaster(bytes.buffer);
    expect(r && [r.width, r.height, Array.from(r.weights)]).toEqual([3, 2, [1, 2, 3, 4, 5, 6]]);
    expect(parseRaster(new Uint8Array([3, 0, 0, 0, 2, 0, 0, 0, 1]).buffer)).toBeNull();
  });
});

describe("aiMaskMenu", () => {
  const a = { pick: vi.fn(), find: vi.fn(), again: vi.fn() };

  it("before a scan, offers each target and finds on select", () => {
    const items = aiMaskMenu(list([], false), a);
    expect(labels(items)).toEqual(["Sky", "Subject", "Background", "People"]);
    const sky = items[0];
    if ("onSelect" in sky) sky.onSelect();
    expect(a.find).toHaveBeenCalledWith("sky");
  });

  it("after a scan, lists what was found and says what wasn't", () => {
    const items = aiMaskMenu(list([info("subject"), info("background"), info("person", 0), info("person", 1)]), a);
    expect(labels(items)).toEqual(["No sky found", "Subject", "Background", "People", "—", "Detect again"]);
    expect(items[0]).toMatchObject({ disabled: true });
    const people = items[3];
    expect("children" in people && labels(people.children)).toEqual(["Person 1", "Person 2"]);
    if ("children" in people) {
      const p2 = people.children[1];
      if ("onSelect" in p2) p2.onSelect();
    }
    expect(a.pick).toHaveBeenCalledWith(info("person", 1));
  });

  it("says when nobody was found", () => {
    const items = aiMaskMenu(list([]), a);
    expect(labels(items)).toEqual(["No sky found", "No subject found", "No background found", "No people found", "—", "Detect again"]);
  });
});
