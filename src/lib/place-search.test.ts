import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { invokeMock, mockCommand, resetInvokeMock } from "@/test/tauri-mock";
import { dedupePlaces, fromPhoton, placeKind, searchPlaces } from "./place-search";

const vevchani = {
  geometry: { coordinates: [20.592266, 41.2402702] },
  properties: { name: "Vevchani", osm_value: "village", country: "North Macedonia", extent: [20.5155626, 41.264269, 20.6484598, 41.2247997] },
};

describe("fromPhoton", () => {
  it("names the place, says what and where it is, and keeps its extent", () => {
    expect(fromPhoton(vevchani as never)).toEqual({
      label: "Vevchani",
      kind: "village",
      detail: "Village · North Macedonia",
      lat: 41.2402702,
      lon: 20.592266,
      bounds: { minLat: 41.2247997, maxLat: 41.264269, minLon: 20.5155626, maxLon: 20.6484598 },
    });
  });
});

describe("searchPlaces", () => {
  beforeEach(() => resetInvokeMock());
  afterEach(() => vi.unstubAllGlobals());

  it("asks OpenStreetMap when online detail is on, so villages are found", async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ features: [vevchani] })));
    vi.stubGlobal("fetch", fetchMock);
    const got = await searchPlaces("vevch", { online: true });
    expect(got[0].label).toBe("Vevchani");
    expect(String((fetchMock.mock.calls[0] as unknown[])[0])).toContain("q=vevch");
    expect(invokeMock).not.toHaveBeenCalledWith("search_places", expect.anything());
  });

  it("falls back to the bundled atlas offline, or when the service fails", async () => {
    mockCommand("search_places", () => [{ label: "Ohrid, North Macedonia", lat: 41.11, lon: 20.8 }]);
    vi.stubGlobal("fetch", vi.fn(async () => Promise.reject(new TypeError("offline"))));
    expect((await searchPlaces("ohr", { online: true }))[0]).toMatchObject({ label: "Ohrid, North Macedonia", bounds: null });
    expect((await searchPlaces("ohr", { online: false }))[0].label).toBe("Ohrid, North Macedonia");
  });
});

describe("placeKind", () => {
  it("files OpenStreetMap tags under the few kinds the list shows icons for", () => {
    expect(placeKind("place", "city")).toBe("city");
    expect(placeKind("place", "hamlet")).toBe("village");
    expect(placeKind("waterway", "stream")).toBe("water");
    expect(placeKind("natural", "peak")).toBe("mountain");
    expect(placeKind("highway", "residential")).toBe("road");
    expect(placeKind("amenity", "place_of_worship")).toBe("landmark");
    expect(placeKind("aeroway", "aerodrome")).toBe("airport");
    expect(placeKind("boundary", "administrative")).toBe("region");
    expect(placeKind("place", "country")).toBe("country");
    expect(placeKind(undefined, undefined)).toBe("place");
  });
});

describe("dedupePlaces", () => {
  it("merges the pieces OpenStreetMap maps one stream in, and keeps same-named places elsewhere", () => {
    const seg = (w: number, e: number) => ({
      label: "Kropa",
      kind: "water" as const,
      detail: "Stream · Gornji Grad, Slovenia",
      lat: 46.3,
      lon: w,
      bounds: { minLat: 46.2, maxLat: 46.4, minLon: w, maxLon: e },
    });
    const other = { ...seg(14.0, 14.1), detail: "Village · Kropa, Slovenia", kind: "village" as const };
    const got = dedupePlaces([seg(14.8, 14.9), seg(14.9, 15.0), other, seg(15.0, 15.1)]);
    expect(got).toHaveLength(2);
    expect(got[0].bounds).toEqual({ minLat: 46.2, maxLat: 46.4, minLon: 14.8, maxLon: 15.1 });
    expect(got[1].detail).toBe("Village · Kropa, Slovenia");
  });
});
