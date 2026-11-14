// jsdom has no WebGL, so tests that mount the map get this stand-in for maplibre-gl's Map:
// it fires `load` and `moveend` like the real one, reports a fixed viewport, projects
// points linearly and records camera calls so a test can assert on them. Use it with
//   vi.mock("maplibre-gl", async () => ({ Map: (await import("@/test/maplibre-mock")).FakeMap, setWorkerUrl: () => {} }));
type Handler = (...args: unknown[]) => void;

export const mapCalls = { fitBounds: [] as unknown[][], flyTo: [] as unknown[], setStyle: [] as unknown[] };

export class FakeMap {
  private handlers = new Map<string, Handler[]>();
  private z: number;
  touchZoomRotate = { disableRotation: () => {} };

  constructor(opts: { zoom?: number }) {
    this.z = opts.zoom ?? 1;
    setTimeout(() => {
      this.emit("load");
      this.emit("moveend");
    }, 0);
  }

  on(event: string, h: Handler) {
    this.handlers.set(event, [...(this.handlers.get(event) ?? []), h]);
    return this;
  }
  once(event: string, h: Handler) {
    return this.on(event, h);
  }
  emit(event: string) {
    for (const h of this.handlers.get(event) ?? []) h();
  }
  getZoom() {
    return this.z;
  }
  getBounds() {
    return { getWest: () => -20, getSouth: () => 30, getEast: () => 30, getNorth: () => 55 };
  }
  project([lon, lat]: [number, number]) {
    return { x: (lon + 180) * 2, y: (90 - lat) * 2 };
  }
  fitBounds(...args: unknown[]) {
    mapCalls.fitBounds.push(args);
  }
  flyTo(opts: unknown) {
    mapCalls.flyTo.push(opts);
  }
  setStyle(style: unknown) {
    mapCalls.setStyle.push(style);
  }
  setMaxZoom() {}
  remove() {}
}
