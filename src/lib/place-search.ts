// Place search for the Map. With online detail on it asks Photon — OpenStreetMap's
// search-as-you-type geocoder, no key — which knows villages, streets and landmarks. The
// bundled GeoNames atlas (towns of 15,000 and more) answers offline, with online detail off,
// or when the service does not.
import { invoke } from "@/lib/ipc";
import type { Bounds } from "@/types/generated/Bounds";
import type { PlaceHit } from "@/types/generated/PlaceHit";

const PHOTON = "https://photon.komoot.io/api/";
const PHOTON_LANGS = new Set(["en", "de", "fr", "it"]);

/** What a result is, coarsely: enough to pick an icon and to tell a town from a stream. */
export type PlaceKind = "country" | "region" | "city" | "town" | "village" | "water" | "mountain" | "nature" | "road" | "airport" | "landmark" | "address" | "place";

const PLACE_VALUES: Record<string, PlaceKind> = {
  country: "country",
  state: "region",
  region: "region",
  province: "region",
  county: "region",
  city: "city",
  town: "town",
  suburb: "town",
  borough: "town",
  village: "village",
  hamlet: "village",
  isolated_dwelling: "village",
  locality: "village",
  neighbourhood: "village",
};

/** OpenStreetMap's key and value (`waterway=stream`, `place=hamlet`, …) as a `PlaceKind`. */
export function placeKind(key: string | undefined, value: string | undefined): PlaceKind {
  if (value && PLACE_VALUES[value] && (!key || key === "place" || key === "boundary")) return PLACE_VALUES[value];
  switch (key) {
    case "waterway":
      return "water";
    case "natural":
      return value && ["peak", "volcano", "ridge", "hill", "mountain_range", "saddle", "cliff"].includes(value)
        ? "mountain"
        : value && ["water", "bay", "spring", "strait", "wetland", "glacier"].includes(value)
          ? "water"
          : "nature";
    case "water":
      return "water";
    case "highway":
    case "railway":
      return "road";
    case "aeroway":
      return "airport";
    case "leisure":
    case "landuse":
      return "nature";
    case "boundary":
      return "region";
    case "building":
      return "address";
    case "amenity":
    case "tourism":
    case "historic":
    case "shop":
    case "man_made":
      return "landmark";
    case "place":
      return "village";
    default:
      return value ? "landmark" : "place";
  }
}

export interface PlaceResult {
  label: string;
  kind: PlaceKind;
  /** What and where: "Village · North Macedonia". */
  detail: string | null;
  lat: number;
  lon: number;
  /** The place's extent when known, so a town fills the view instead of a fixed zoom. */
  bounds: Bounds | null;
}

interface PhotonFeature {
  geometry: { coordinates: [number, number] };
  properties: {
    name?: string;
    osm_key?: string;
    osm_value?: string;
    city?: string;
    state?: string;
    country?: string;
    street?: string;
    housenumber?: string;
    extent?: [number, number, number, number];
  };
}

const capitalise = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, " ");

export function fromPhoton(f: PhotonFeature): PlaceResult {
  const p = f.properties;
  const [lon, lat] = f.geometry.coordinates;
  const label = p.name ?? ([p.street, p.housenumber].filter(Boolean).join(" ") || "Unnamed place");
  const where = [p.city !== p.name ? p.city : null, p.state, p.country].filter(Boolean).join(", ");
  const what = p.osm_value && p.osm_value !== "yes" ? capitalise(p.osm_value) : null;
  const e = p.extent; // [west, north, east, south]
  return {
    label,
    kind: placeKind(p.osm_key, p.osm_value),
    detail: [what, where].filter(Boolean).join(" · ") || null,
    lat,
    lon,
    bounds: e ? { minLat: e[3], maxLat: e[1], minLon: e[0], maxLon: e[2] } : null,
  };
}

/**
 * One entry per place. OpenStreetMap maps a long stream or road as many pieces, and each
 * comes back as its own result with the same name and the same "where"; they merge, and
 * their extents join so choosing it frames the whole thing. Same-named places elsewhere
 * have a different "where" and stay.
 */
export function dedupePlaces(places: PlaceResult[]): PlaceResult[] {
  const byKey = new Map<string, PlaceResult>();
  for (const p of places) {
    const key = `${p.label.toLowerCase()}|${p.detail ?? ""}`;
    const seen = byKey.get(key);
    if (!seen) {
      byKey.set(key, { ...p });
      continue;
    }
    if (seen.bounds && p.bounds) {
      seen.bounds = {
        minLat: Math.min(seen.bounds.minLat, p.bounds.minLat),
        maxLat: Math.max(seen.bounds.maxLat, p.bounds.maxLat),
        minLon: Math.min(seen.bounds.minLon, p.bounds.minLon),
        maxLon: Math.max(seen.bounds.maxLon, p.bounds.maxLon),
      };
    } else {
      seen.bounds ??= p.bounds;
    }
  }
  return [...byKey.values()];
}

async function photon(query: string, limit: number): Promise<PlaceResult[]> {
  const lang = navigator.language?.slice(0, 2);
  const url = new URL(PHOTON);
  url.searchParams.set("q", query);
  // Ask for more than are shown: merging duplicates would otherwise leave a short list.
  url.searchParams.set("limit", String(limit * 2));
  if (lang && PHOTON_LANGS.has(lang)) url.searchParams.set("lang", lang);
  const r = await fetch(url);
  if (!r.ok) throw new Error(`photon ${r.status}`);
  const data = (await r.json()) as { features: PhotonFeature[] };
  return dedupePlaces(data.features.map(fromPhoton)).slice(0, limit);
}

async function atlas(query: string, limit: number): Promise<PlaceResult[]> {
  const hits = await invoke<PlaceHit[]>("search_places", { query, limit });
  return hits.map((h) => ({ label: h.label, kind: "city" as const, detail: null, lat: h.lat, lon: h.lon, bounds: null }));
}

export async function searchPlaces(query: string, opts: { online: boolean; limit?: number }): Promise<PlaceResult[]> {
  const limit = opts.limit ?? 8;
  if (opts.online) {
    try {
      return await photon(query, limit);
    } catch {
      // No connection or the service is down: the bundled atlas still knows the towns.
    }
  }
  return atlas(query, limit);
}
