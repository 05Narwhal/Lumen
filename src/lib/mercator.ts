// Web Mercator, the projection every slippy map uses. Pure functions so the awkward
// parts — the pole clamp, the antimeridian, zoom around a point — are unit-testable.

import type { Bounds } from "@/types/generated/Bounds";

/** Mercator cannot represent the poles; every map clamps at this latitude. */
export const MAX_LATITUDE = 85.05112878;

export interface Viewport {
  /** Centre of the view. */
  lat: number;
  lon: number;
  /** Tile zoom: the world is `256 * 2^zoom` pixels wide. */
  zoom: number;
  width: number;
  height: number;
}

export const worldSize = (zoom: number): number => 256 * 2 ** zoom;

/** Longitude to a fraction of the world, 0 at -180 and 1 at +180. */
export const lonToX = (lon: number): number => (lon + 180) / 360;

/** Latitude to a fraction of the world, 0 at the top. */
export function latToY(lat: number): number {
  const clamped = Math.max(-MAX_LATITUDE, Math.min(MAX_LATITUDE, lat));
  const rad = (clamped * Math.PI) / 180;
  return 0.5 - Math.log(Math.tan(Math.PI / 4 + rad / 2)) / (2 * Math.PI);
}

export const xToLon = (x: number): number => x * 360 - 180;

export function yToLat(y: number): number {
  const n = Math.PI * (1 - 2 * y);
  return (180 / Math.PI) * Math.atan(Math.sinh(n));
}

/** Screen position of a coordinate, in pixels from the top-left of the viewport. */
export function project(lat: number, lon: number, v: Viewport): { x: number; y: number } {
  const size = worldSize(v.zoom);
  const cx = lonToX(v.lon) * size;
  const cy = latToY(v.lat) * size;
  return { x: lonToX(lon) * size - cx + v.width / 2, y: latToY(lat) * size - cy + v.height / 2 };
}

/** The coordinate under a screen position. */
export function unproject(x: number, y: number, v: Viewport): { lat: number; lon: number } {
  const size = worldSize(v.zoom);
  const cx = lonToX(v.lon) * size;
  const cy = latToY(v.lat) * size;
  return { lat: yToLat((y - v.height / 2 + cy) / size), lon: xToLon((x - v.width / 2 + cx) / size) };
}

/** What the viewport currently covers, for the cluster query. */
export function viewportBounds(v: Viewport): Bounds {
  const topLeft = unproject(0, 0, v);
  const bottomRight = unproject(v.width, v.height, v);
  return {
    minLat: Math.min(topLeft.lat, bottomRight.lat),
    maxLat: Math.max(topLeft.lat, bottomRight.lat),
    minLon: Math.max(-180, Math.min(topLeft.lon, bottomRight.lon)),
    maxLon: Math.min(180, Math.max(topLeft.lon, bottomRight.lon)),
  };
}

/**
 * Move the *view* by `(dx, dy)` pixels: the content slides the opposite way, as when you
 * drag a map. Panning right by 100 shows what was 100 px to the right.
 */
export function pan(v: Viewport, dx: number, dy: number): Viewport {
  const centre = project(v.lat, v.lon, v);
  const moved = unproject(centre.x + dx, centre.y + dy, v);
  return { ...v, lat: Math.max(-MAX_LATITUDE, Math.min(MAX_LATITUDE, moved.lat)), lon: wrapLon(moved.lon) };
}

/** Zoom by `delta` steps while keeping the coordinate under (x, y) in place. */
export function zoomAround(v: Viewport, delta: number, x: number, y: number, minZoom = 1, maxZoom = 18): Viewport {
  const zoom = Math.max(minZoom, Math.min(maxZoom, v.zoom + delta));
  if (zoom === v.zoom) return v;
  const anchor = unproject(x, y, v);
  const zoomed: Viewport = { ...v, zoom };
  const after = project(anchor.lat, anchor.lon, zoomed);
  return pan(zoomed, after.x - x, after.y - y);
}

/** Longitude wrapped into [-180, 180). */
export function wrapLon(lon: number): number {
  const wrapped = ((lon + 180) % 360 + 360) % 360 - 180;
  return wrapped;
}

/** The view that shows `bounds` with a little room around it. */
export function fitBounds(bounds: Bounds, width: number, height: number, padding = 48): Viewport {
  const lat = (bounds.minLat + bounds.maxLat) / 2;
  const lon = (bounds.minLon + bounds.maxLon) / 2;
  const spanX = Math.abs(lonToX(bounds.maxLon) - lonToX(bounds.minLon)) || 1e-6;
  const spanY = Math.abs(latToY(bounds.minLat) - latToY(bounds.maxLat)) || 1e-6;
  const usableW = Math.max(1, width - padding * 2);
  const usableH = Math.max(1, height - padding * 2);
  // The zoom where the span fills the usable box, whichever axis binds first.
  const zoomX = Math.log2(usableW / (256 * spanX));
  const zoomY = Math.log2(usableH / (256 * spanY));
  const zoom = Math.max(1, Math.min(18, Math.min(zoomX, zoomY)));
  return { lat, lon, zoom, width, height };
}
