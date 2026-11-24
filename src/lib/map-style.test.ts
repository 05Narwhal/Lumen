import type { Polygon } from "geojson";
import { describe, expect, it } from "vitest";
import { clusterZoom, offlineGeoJson, SATELLITE_LINE_OPACITY, SATELLITE_MAX_ZOOM, SATELLITE_SOURCE_MAX_ZOOM, satelliteOverlay, satelliteSource, toBounds } from "./map-style";

describe("clusterZoom", () => {
  it("converts MapLibre's 512px-tile zoom to the 256px-tile zoom the clustering uses", () => {
    expect(clusterZoom(3)).toBe(4);
    expect(clusterZoom(10.5)).toBe(11.5);
  });
});

describe("toBounds", () => {
  it("passes an ordinary viewport through", () => {
    expect(toBounds({ west: -9.5, south: 38, east: 12.5, north: 48 })).toEqual({ minLat: 38, maxLat: 48, minLon: -9.5, maxLon: 12.5 });
  });

  it("wraps a viewport that has panned past the antimeridian", () => {
    // MapLibre reports 170°E → 190°E; the catalogue speaks -180..180.
    expect(toBounds({ west: 170, south: -10, east: 190, north: 10 })).toEqual({ minLat: -10, maxLat: 10, minLon: 170, maxLon: -170 });
  });

  it("asks for the whole world when more than a world is visible", () => {
    expect(toBounds({ west: -300, south: -85, east: 300, north: 85 })).toEqual({ minLat: -85, maxLat: 85, minLon: -180, maxLon: 180 });
  });
});

describe("satelliteOverlay", () => {
  it("keeps roads, borders and labels from the vector style and drops its fills", () => {
    const layers = [
      { id: "background", type: "background" },
      { id: "water", type: "fill", source: "openmaptiles", "source-layer": "water" },
      { id: "building", type: "fill", source: "openmaptiles", "source-layer": "building" },
      { id: "road_primary", type: "line", source: "openmaptiles", "source-layer": "transportation" },
      { id: "boundary_2", type: "line", source: "openmaptiles", "source-layer": "boundary" },
      { id: "waterway", type: "line", source: "openmaptiles", "source-layer": "waterway" },
      { id: "place_city", type: "symbol", source: "openmaptiles", "source-layer": "place" },
      { id: "hillshade", type: "raster", source: "ne2_shaded" },
    ] as never[];
    const kept = satelliteOverlay(layers);
    expect(kept.map((l: { id: string }) => l.id)).toEqual(["road_primary", "boundary_2", "place_city"]);
  });

  it("lets the imagery show through the roads and borders", () => {
    const layers = [
      { id: "road", type: "line", source: "openmaptiles", "source-layer": "transportation", paint: { "line-color": "#fff", "line-opacity": 1 } },
      { id: "label", type: "symbol", source: "openmaptiles", "source-layer": "place", paint: { "text-color": "#000" } },
    ] as never[];
    const [road, label] = satelliteOverlay(layers) as { paint: Record<string, unknown> }[];
    expect(road.paint["line-opacity"]).toBe(SATELLITE_LINE_OPACITY);
    expect(road.paint["line-color"]).toBe("#fff");
    expect(SATELLITE_LINE_OPACITY).toBeLessThan(0.6);
    expect(label.paint["text-color"]).toBe("#000");
  });
});

describe("satelliteSource", () => {
  const SECRET = "AAPK-own-key 123";

  it("is EOX Sentinel-2 cloudless without a key", () => {
    const s = satelliteSource();
    expect(s.tiles).toEqual(["https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless-2025_3857/default/GoogleMapsCompatible/{z}/{y}/{x}.jpg"]);
    expect(s.attribution).toContain("EOxCloudless");
    expect(s.attribution).toContain("EOX IT Services GmbH");
    expect(s.attribution).toContain("Contains modified Copernicus Sentinel data 2025");
    expect(s.maxzoom).toBe(14);
    expect(s.mapMaxZoom).toBeGreaterThanOrEqual(14);
    expect(s.mapMaxZoom).toBeLessThanOrEqual(15);
    expect(JSON.stringify(s)).not.toContain("arcgis");
  });

  it("treats empty, null, undefined and whitespace-only keys as none", () => {
    const eox = satelliteSource();
    for (const key of [undefined, null, "", "   ", " \t\n "]) expect(satelliteSource(key)).toEqual(eox);
  });

  it("uses Esri's keyed World Imagery with the key as token, at the old zoom limits", () => {
    const s = satelliteSource(`  ${SECRET} `);
    expect(s.tiles).toEqual([
      `https://ibasemaps-api.arcgis.com/arcgis/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}?token=${encodeURIComponent(SECRET)}`,
    ]);
    expect(s.maxzoom).toBe(SATELLITE_SOURCE_MAX_ZOOM);
    expect(s.mapMaxZoom).toBe(SATELLITE_MAX_ZOOM);
    expect(s.attribution).toContain("Powered by Esri");
    expect(s.attribution).toContain("Esri, Maxar, Earthstar Geographics");
    expect(JSON.stringify(s)).not.toContain("eox");
  });

  it("never puts the key in the attribution", () => {
    expect(satelliteSource(SECRET).attribution).not.toContain("AAPK");
    expect(satelliteSource(SECRET).attribution).not.toContain(SECRET);
  });
});

describe("offlineGeoJson", () => {
  it("turns the bundled rings back into longitude and latitude polygons", () => {
    const q = 1 << 20;
    // One square: (0.25, 0.25) → (0.75, 0.25) → (0.75, 0.75) → (0.25, 0.75) of the world.
    const ring = [q / 4, q / 4, q / 2, 0, 0, q / 2, -q / 2, 0];
    const fc = offlineGeoJson({ q, land: [ring], lakes: [], borders: [] }).land;
    const [first] = fc.features;
    expect(first.geometry.type).toBe("Polygon");
    const coords = (first.geometry as Polygon).coordinates[0];
    expect(coords[0][0]).toBeCloseTo(-90);
    expect(coords[1][0]).toBeCloseTo(90);
    expect(coords[0][1]).toBeCloseTo(66.51, 1); // y 0.25 in Web Mercator
    expect(coords.at(-1)).toEqual(coords[0]); // closed
  });
});
