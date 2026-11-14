import { describe, expect, it } from "vitest";
import { fitBounds, MAX_LATITUDE, pan, project, unproject, viewportBounds, wrapLon, zoomAround, type Viewport } from "./mercator";

const view = (over: Partial<Viewport> = {}): Viewport => ({ lat: 0, lon: 0, zoom: 2, width: 800, height: 600, ...over });

describe("web mercator", () => {
  it("puts the view's centre in the middle of the screen", () => {
    const v = view({ lat: 38.72, lon: -9.14 });
    const p = project(v.lat, v.lon, v);
    expect(p.x).toBeCloseTo(400, 6);
    expect(p.y).toBeCloseTo(300, 6);
  });

  it("round-trips a coordinate through project and unproject", () => {
    const v = view({ lat: 46.05, lon: 14.5, zoom: 7 });
    const p = project(51.5, -0.12, v);
    const back = unproject(p.x, p.y, v);
    expect(back.lat).toBeCloseTo(51.5, 6);
    expect(back.lon).toBeCloseTo(-0.12, 6);
  });

  it("clamps the poles, which Mercator cannot show", () => {
    const v = view();
    expect(project(89.9, 0, v).y).toBeCloseTo(project(MAX_LATITUDE, 0, v).y, 6);
    expect(project(-89.9, 0, v).y).toBeCloseTo(project(-MAX_LATITUDE, 0, v).y, 6);
  });

  it("keeps north up and east right", () => {
    const v = view();
    expect(project(10, 0, v).y).toBeLessThan(project(-10, 0, v).y);
    expect(project(0, 10, v).x).toBeGreaterThan(project(0, -10, v).x);
  });

  it("pans by exactly the drag distance", () => {
    const v = view({ lat: 20, lon: 30, zoom: 5 });
    const moved = pan(v, 100, 0);
    // Dragging the map left moves the centre east.
    expect(moved.lon).toBeGreaterThan(v.lon);
    const p = project(v.lat, v.lon, moved);
    expect(p.x).toBeCloseTo(400 - 100, 4);
  });

  it("keeps the point under the cursor fixed while zooming", () => {
    const v = view({ lat: 40, lon: -3, zoom: 4 });
    const anchor = unproject(200, 150, v);
    const zoomed = zoomAround(v, 1, 200, 150);
    expect(zoomed.zoom).toBe(5);
    const after = project(anchor.lat, anchor.lon, zoomed);
    expect(after.x).toBeCloseTo(200, 3);
    expect(after.y).toBeCloseTo(150, 3);
  });

  it("does not zoom past its limits", () => {
    expect(zoomAround(view({ zoom: 18 }), 1, 0, 0).zoom).toBe(18);
    expect(zoomAround(view({ zoom: 1 }), -1, 0, 0).zoom).toBe(1);
  });

  it("reports the bounds the viewport covers", () => {
    const b = viewportBounds(view({ lat: 0, lon: 0, zoom: 3 }));
    expect(b.minLat).toBeLessThan(0);
    expect(b.maxLat).toBeGreaterThan(0);
    expect(b.minLon).toBeGreaterThanOrEqual(-180);
    expect(b.maxLon).toBeLessThanOrEqual(180);
    expect(b.minLat).toBeLessThan(b.maxLat);
  });

  it("wraps longitude across the antimeridian", () => {
    expect(wrapLon(190)).toBeCloseTo(-170, 6);
    expect(wrapLon(-190)).toBeCloseTo(170, 6);
    expect(wrapLon(45)).toBeCloseTo(45, 6);
    expect(wrapLon(-180)).toBeCloseTo(-180, 6);
  });

  it("fits a box into the viewport", () => {
    const lisbon = { minLat: 38.6, maxLat: 38.8, minLon: -9.25, maxLon: -9.05 };
    const v = fitBounds(lisbon, 800, 600);
    expect(v.lat).toBeCloseTo(38.7, 3);
    expect(v.lon).toBeCloseTo(-9.15, 3);
    // Everything in the box must land inside the viewport.
    for (const [lat, lon] of [
      [lisbon.minLat, lisbon.minLon],
      [lisbon.maxLat, lisbon.maxLon],
    ]) {
      const p = project(lat, lon, v);
      expect(p.x).toBeGreaterThanOrEqual(0);
      expect(p.x).toBeLessThanOrEqual(800);
      expect(p.y).toBeGreaterThanOrEqual(0);
      expect(p.y).toBeLessThanOrEqual(600);
    }
    // A single point does not produce an infinite zoom.
    const dot = fitBounds({ minLat: 10, maxLat: 10, minLon: 20, maxLon: 20 }, 800, 600);
    expect(Number.isFinite(dot.zoom)).toBe(true);
    expect(dot.zoom).toBeLessThanOrEqual(18);
  });
});
