// The Map screen's base map: MapLibre styles for detailed online maps, satellite imagery,
// and a bundled offline floor. Architecture spec §3.5: MapLibre GL with vector tiles.
//
// Online detail comes from OpenFreeMap (OpenMapTiles vector tiles, no key, no quota) and
// Esri World Imagery (satellite). When online detail is off or the servers cannot be
// reached, the map falls back to the Natural Earth data bundled in `src/assets/map/`, drawn
// in the theme's own colours — it is never an empty rectangle.

import type { FeatureCollection } from "geojson";
import type { LayerSpecification, StyleSpecification } from "maplibre-gl";
import { xToLon, yToLat } from "@/lib/mercator";
import type { Bounds } from "@/types/generated/Bounds";
import type { MapStyle } from "@/types/generated/MapStyle";

const OPENFREEMAP = "https://tiles.openfreemap.org/styles";
const ESRI_IMAGERY = "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
const ESRI_ATTRIBUTION = "Imagery © Esri, Maxar, Earthstar Geographics";

/**
 * MapLibre sizes the world in 512px tiles; the Rust clustering (and every marker size it
 * reasons about) uses 256px tiles. Same picture, one zoom level apart.
 */
export const clusterZoom = (mapZoom: number) => mapZoom + 1;

/** A MapLibre viewport as the catalogue's bounds: longitudes wrapped into -180..180. */
export function toBounds(b: { west: number; south: number; east: number; north: number }): Bounds {
  const lat = { minLat: b.south, maxLat: b.north };
  if (b.east - b.west >= 360) return { ...lat, minLon: -180, maxLon: 180 };
  const wrap = (lon: number) => ((((lon + 180) % 360) + 360) % 360) - 180;
  // A wrapped east below west is a viewport across the antimeridian; Rust splits it.
  return { ...lat, minLon: wrap(b.west), maxLon: wrap(b.east) };
}

/** Roads and borders over imagery: present enough to follow, faint enough to see through. */
export const SATELLITE_LINE_OPACITY = 0.45;

/**
 * Esri's imagery reaches zoom 17 (in 256px tiles) nearly everywhere; deeper tiles exist only
 * in some places and come back as a grey "Map data not yet available" square elsewhere.
 * Capping the source here makes MapLibre enlarge the last real tile instead of asking.
 */
export const SATELLITE_SOURCE_MAX_ZOOM = 17;
/** And the camera stops where that enlargement is still worth looking at (512px tiles). */
export const SATELLITE_MAX_ZOOM = 17.5;

/** Roads, borders and labels from the vector style, to draw over satellite imagery. */
export function satelliteOverlay(layers: LayerSpecification[]): LayerSpecification[] {
  const keep = new Set(["transportation", "boundary"]);
  return layers
    .filter((l) => {
      if (l.type === "symbol") return true;
      const sourceLayer = (l as { "source-layer"?: string })["source-layer"];
      return l.type === "line" && sourceLayer != null && keep.has(sourceLayer);
    })
    .map((l) => (l.type === "line" ? ({ ...l, paint: { ...l.paint, "line-opacity": SATELLITE_LINE_OPACITY } } as LayerSpecification) : l));
}

// ---- Offline floor --------------------------------------------------------------------

interface Encoded {
  q: number;
  land: number[][];
  lakes: number[][];
  borders: number[][];
}

function toLonLat(enc: number[], q: number, close: boolean): number[][] {
  const out: number[][] = [];
  let x = 0;
  let y = 0;
  for (let i = 0; i < enc.length; i += 2) {
    x = i === 0 ? enc[0] : x + enc[i];
    y = i === 0 ? enc[1] : y + enc[i + 1];
    out.push([xToLon(x / q), yToLat(y / q)]);
  }
  if (close && out.length > 0) out.push(out[0]);
  return out;
}

/** The bundled rings (delta-encoded Web Mercator, see scripts/build-map-data.py) as GeoJSON. */
export function offlineGeoJson(e: Encoded) {
  const polys = (rings: number[][]): FeatureCollection => ({
    type: "FeatureCollection",
    features: rings.map((r) => ({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [toLonLat(r, e.q, true)] } })),
  });
  return {
    land: polys(e.land),
    lakes: polys(e.lakes),
    borders: {
      type: "FeatureCollection",
      features: e.borders.map((r) => ({ type: "Feature", properties: {}, geometry: { type: "LineString", coordinates: toLonLat(r, e.q, false) } })),
    } as FeatureCollection,
  };
}

let offlineData: Promise<ReturnType<typeof offlineGeoJson>> | null = null;
const loadOffline = () => (offlineData ??= import("@/assets/map/world-50m.json").then((m) => offlineGeoJson(m.default as Encoded)));

export interface MapColours {
  water: string;
  land: string;
  coast: string;
  border: string;
}

async function offlineStyle(c: MapColours): Promise<StyleSpecification> {
  const data = await loadOffline();
  return {
    version: 8,
    sources: {
      land: { type: "geojson", data: data.land },
      lakes: { type: "geojson", data: data.lakes },
      borders: { type: "geojson", data: data.borders },
    },
    layers: [
      { id: "water", type: "background", paint: { "background-color": c.water } },
      { id: "land", type: "fill", source: "land", paint: { "fill-color": c.land, "fill-outline-color": c.coast } },
      { id: "lakes", type: "fill", source: "lakes", paint: { "fill-color": c.water, "fill-outline-color": c.coast } },
      { id: "borders", type: "line", source: "borders", paint: { "line-color": c.border, "line-width": 0.8 } },
      { id: "coast", type: "line", source: "land", paint: { "line-color": c.coast, "line-width": 1 } },
    ],
  };
}

// ---- Online styles --------------------------------------------------------------------

const vectorStyles = new Map<string, Promise<StyleSpecification>>();
function vectorStyle(name: string): Promise<StyleSpecification> {
  if (!vectorStyles.has(name)) {
    const p = fetch(`${OPENFREEMAP}/${name}`).then((r) => {
      if (!r.ok) throw new Error(`map style ${name}: ${r.status}`);
      return r.json() as Promise<StyleSpecification>;
    });
    // A failed fetch must not be cached, or the map stays offline after the network returns.
    p.catch(() => vectorStyles.delete(name));
    vectorStyles.set(name, p);
  }
  return vectorStyles.get(name)!;
}

async function satelliteStyle(): Promise<StyleSpecification> {
  // Labels and roads come from the light style: dark text with a pale halo reads on imagery.
  const base = await vectorStyle("liberty");
  return {
    ...base,
    sources: {
      imagery: { type: "raster", tiles: [ESRI_IMAGERY], tileSize: 256, maxzoom: SATELLITE_SOURCE_MAX_ZOOM, attribution: ESRI_ATTRIBUTION },
      ...base.sources,
    },
    layers: [{ id: "imagery", type: "raster", source: "imagery" }, ...satelliteOverlay(base.layers)],
  };
}

export interface StyleRequest {
  online: boolean;
  style: MapStyle;
  dark: boolean;
  colours: MapColours;
}

/**
 * The style to show. Online detail when allowed and reachable; otherwise the offline floor.
 * `offline` in the result says which one it is, so the caller can retry when the network
 * comes back.
 */
export async function buildStyle(req: StyleRequest): Promise<{ style: StyleSpecification; offline: boolean }> {
  if (req.online) {
    try {
      const style = req.style === "satellite" ? await satelliteStyle() : await vectorStyle(req.dark ? "fiord" : "liberty");
      return { style, offline: false };
    } catch {
      // No connection, or the tile server is down: the bundled world, never a blank map.
    }
  }
  return { style: await offlineStyle(req.colours), offline: true };
}
