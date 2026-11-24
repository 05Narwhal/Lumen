// The Map screen's base map: MapLibre styles for detailed online maps, satellite imagery,
// and a bundled offline floor. Architecture spec §3.5: MapLibre GL with vector tiles.
//
// Online detail comes from OpenFreeMap (OpenMapTiles vector tiles, no key, no quota) and,
// for satellite, EOX Sentinel-2 cloudless (free for non-commercial use with attribution) or,
// only with a person's own key, Esri World Imagery. When online detail is off or the servers
// cannot be reached, the map falls back to the Natural Earth data bundled in
// `src/assets/map/`, drawn in the theme's own colours — it is never an empty rectangle.

import type { FeatureCollection } from "geojson";
import type { LayerSpecification, StyleSpecification } from "maplibre-gl";
import { xToLon, yToLat } from "@/lib/mercator";
import type { Bounds } from "@/types/generated/Bounds";
import type { MapStyle } from "@/types/generated/MapStyle";

const OPENFREEMAP = "https://tiles.openfreemap.org/styles";

// Satellite imagery sources, read at dev time (2026-11-14); the app asks for tiles only under
// the "online map" switch:
//
// EOX: the newest yearly Web Mercator layer in
//   https://tiles.maps.eox.at/wmts/1.0.0/WMTSCapabilities.xml is `s2cloudless-2025_3857`; its
//   ResourceURL template is `.../s2cloudless-2025_3857/default/{TileMatrixSet}/{TileMatrix}/
//   {TileRow}/{TileCol}.jpg` with TileMatrixSet `GoogleMapsCompatible` (256px tiles, matrices
//   0-21, EPSG:3857). A real tile (z14 over Ljubljana, row 5825 col 8852) came back as a
//   256x256 JPEG of the old town and castle hill, so it is plain XYZ `{z}/{y}/{x}` despite
//   EOX's metatiles. Licence CC BY-NC-SA 4.0; the attribution text is the layer's own.
// Esri: https://developers.arcgis.com/maplibre-gl-js/maps/raster-tile-basemaps/display-multiple-basemap-layers/
//   uses `https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/
//   {z}/{y}/{x}?token=<API key>`; the key needs a Location Platform account with the static
//   basemap tiles privilege. Without a key Lumen never asks Esri.
const EOX_YEAR = 2025;
const EOX_IMAGERY = `https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-${EOX_YEAR}_3857/default/GoogleMapsCompatible/{z}/{y}/{x}.jpg`;
const EOX_ATTRIBUTION = `EOxCloudless https://cloudless.eox.at by EOX IT Services GmbH (Contains modified Copernicus Sentinel data ${EOX_YEAR})`;
const ESRI_IMAGERY = "https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";
const ESRI_ATTRIBUTION = "Powered by Esri | Imagery © Esri, Maxar, Earthstar Geographics";

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
 * With a person's own key, Esri's imagery reaches zoom 17 (in 256px tiles) nearly everywhere; deeper tiles exist only
 * in some places and come back as a grey "Map data not yet available" square elsewhere.
 * Capping the source here makes MapLibre enlarge the last real tile instead of asking.
 */
export const SATELLITE_SOURCE_MAX_ZOOM = 17;
/** And the camera stops where that enlargement is still worth looking at (512px tiles). */
export const SATELLITE_MAX_ZOOM = 17.5;

/**
 * EOX's layer is Sentinel-2, about 10 m per pixel. Its capabilities list matrices down to 21,
 * but past zoom 14 they only enlarge the same pixels, so MapLibre enlarges the last real tile
 * instead of asking for blurrier ones.
 */
export const EOX_SOURCE_MAX_ZOOM = 14;
/** And the camera stops where that enlargement is still worth looking at (512px tiles): measured over Ljubljana, 14 is sharp, 15 soft but readable, 16 mush. */
export const EOX_MAX_ZOOM = 15;

export interface SatelliteSource {
  tiles: string[];
  maxzoom: number;
  attribution: string;
  /** The camera's zoom cap while this source is shown. */
  mapMaxZoom: number;
}

/**
 * The satellite imagery to draw: EOX Sentinel-2 cloudless, or Esri World Imagery when the
 * person has entered their own key (a blank or whitespace-only key is no key). The key is
 * used only as the tile URL's token; it is never part of the attribution.
 */
export function satelliteSource(esriKey?: string | null): SatelliteSource {
  const key = esriKey?.trim();
  if (key) {
    return {
      tiles: [`${ESRI_IMAGERY}?token=${encodeURIComponent(key)}`],
      maxzoom: SATELLITE_SOURCE_MAX_ZOOM,
      attribution: ESRI_ATTRIBUTION,
      mapMaxZoom: SATELLITE_MAX_ZOOM,
    };
  }
  return { tiles: [EOX_IMAGERY], maxzoom: EOX_SOURCE_MAX_ZOOM, attribution: EOX_ATTRIBUTION, mapMaxZoom: EOX_MAX_ZOOM };
}

/** The camera's zoom cap for the vector map and the offline floor. */
export const VECTOR_MAX_ZOOM = 19;

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

async function satelliteStyle(source: SatelliteSource): Promise<StyleSpecification> {
  // Labels and roads come from the light style: dark text with a pale halo reads on imagery.
  const base = await vectorStyle("liberty");
  return {
    ...base,
    sources: {
      imagery: { type: "raster", tiles: source.tiles, tileSize: 256, maxzoom: source.maxzoom, attribution: source.attribution },
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
  /** The person's own Esri key, if any; satellite imagery is EOX's without one. */
  esriKey?: string | null;
}

/**
 * The style to show. Online detail when allowed and reachable; otherwise the offline floor.
 * `offline` in the result says which one it is, so the caller can retry when the network
 * comes back.
 */
export async function buildStyle(req: StyleRequest): Promise<{ style: StyleSpecification; offline: boolean; maxZoom: number }> {
  if (req.online) {
    try {
      if (req.style === "satellite") {
        const source = satelliteSource(req.esriKey);
        return { style: await satelliteStyle(source), offline: false, maxZoom: source.mapMaxZoom };
      }
      return { style: await vectorStyle(req.dark ? "fiord" : "liberty"), offline: false, maxZoom: VECTOR_MAX_ZOOM };
    } catch {
      // No connection, or the tile server is down: the bundled world, never a blank map.
    }
  }
  return { style: await offlineStyle(req.colours), offline: true, maxZoom: VECTOR_MAX_ZOOM };
}
