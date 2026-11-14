import { describe, expect, it } from "vitest";
import { decodeLayer, reliefFade, worldCopies, worldToScreen } from "./basemap";
import { latToY, lonToX, project, type Viewport } from "./mercator";

const view: Viewport = { lat: 20, lon: 10, zoom: 3, width: 800, height: 600 };

describe("decodeLayer", () => {
  it("turns delta-encoded integer rings into absolute world fractions with bounds", () => {
    const q = 1000;
    const [ring] = decodeLayer([[100, 200, 50, 0, 0, 50, -50, 0]], q);
    expect(Array.from(ring.points)).toEqual([0.1, 0.2, 0.15, 0.2, 0.15, 0.25, 0.1, 0.25]);
    expect(ring.bounds).toEqual([0.1, 0.2, 0.15, 0.25]);
  });
});

describe("worldToScreen", () => {
  it("agrees with the projection the markers use", () => {
    const t = worldToScreen(view);
    for (const [lat, lon] of [
      [38.72, -9.14],
      [-33.87, 151.21],
      [64.1, -21.9],
    ]) {
      const p = project(lat, lon, view);
      expect(lonToX(lon) * t.scale + t.x).toBeCloseTo(p.x, 6);
      expect(latToY(lat) * t.scale + t.y).toBeCloseTo(p.y, 6);
    }
  });

  it("stays exact at street zoom, where float32 path coordinates would drift", () => {
    const deep: Viewport = { ...view, lat: 46.05, lon: 14.5, zoom: 18 };
    const t = worldToScreen(deep);
    expect(lonToX(14.5) * t.scale + t.x).toBeCloseTo(deep.width / 2, 3);
  });
});

describe("worldCopies", () => {
  it("draws one copy of the world when the view sits inside it", () => {
    expect(worldCopies(view)).toEqual([0]);
  });

  it("adds the neighbouring copy when the view straddles the antimeridian", () => {
    expect(worldCopies({ ...view, lon: 179, zoom: 3 })).toEqual([0, 1]);
    expect(worldCopies({ ...view, lon: -179, zoom: 3 })).toEqual([-1, 0]);
  });

  it("covers a window wider than the whole world at the lowest zoom", () => {
    expect(worldCopies({ ...view, zoom: 1, width: 1600 })).toEqual([-2, -1, 0, 1, 2]);
  });
});

describe("reliefFade", () => {
  it("shows the relief at world scale and drops it before it blurs at city scale", () => {
    expect(reliefFade(2)).toBe(1);
    expect(reliefFade(4)).toBe(1);
    expect(reliefFade(5.5)).toBeCloseTo(0.5);
    expect(reliefFade(7)).toBe(0);
    expect(reliefFade(12)).toBe(0);
  });
});
