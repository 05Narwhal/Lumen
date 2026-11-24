import { useComputedColorScheme } from "@mantine/core";
import { Map as MapLibre, setWorkerUrl } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { buildStyle, type MapColours } from "@/lib/map-style";
import type { MapStyle } from "@/types/generated/MapStyle";
import type { Pin } from "./model";

setWorkerUrl(workerUrl);

const STATE_NAME = { anchor: "known place", inferred: "worked out", guess: "guess" } as const;

function colours(el: HTMLElement): MapColours {
  const cs = getComputedStyle(el);
  const t = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  return { water: t("--map-water", "#0e0e0e"), land: t("--map-land", "#1a1a1a"), coast: t("--map-coast", "#666"), border: t("--map-border", "#555") };
}

export interface PlaceMapProps {
  pins: Pin[];
  /** The moment on the card: its pins are drawn large, the rest small. */
  selectedGroup: string | null;
  online: boolean;
  style: MapStyle;
  esriKey?: string | null;
  /** The next click on the map is a place, not a pan (*Move pin*). */
  placing: boolean;
  onSelectPin: (pin: Pin) => void;
  /** A pin dropped somewhere else: its photos go there. */
  onMovePin: (pin: Pin, lat: number, lon: number) => void;
  onPlace: (lat: number, lon: number) => void;
}

/**
 * The review screen's map: MapLibre's base map (the Map page's style and settings) with a DOM
 * pin per spot. ● a known place, ◐ worked out from neighbours, ○ a guess. Pins are moved on
 * every camera frame, as on the Map page, and a pin drags with the pointer, then reports
 * where it was dropped.
 */
export function PlaceMap({ pins, selectedGroup, online, style, esriKey, placing, onSelectPin, onMovePin, onPlace }: PlaceMapProps) {
  const host = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibre | null>(null);
  const els = useRef(new Map<string, { el: HTMLElement; lat: number; lon: number }>());
  const scheme = useComputedColorScheme("light");
  const live = useRef({ placing, onPlace });
  live.current = { placing, onPlace };

  const place = useCallback(() => {
    const m = map.current;
    if (!m) return;
    for (const { el, lat, lon } of els.current.values()) {
      const p = m.project([lon, lat]);
      el.style.transform = `translate3d(${p.x}px, ${p.y}px, 0)`;
    }
  }, []);

  useEffect(() => {
    if (!host.current) return;
    const m = new MapLibre({
      container: host.current,
      style: { version: 8, sources: {}, layers: [] },
      center: [0, 20],
      zoom: 1,
      attributionControl: { compact: true },
      dragRotate: false,
      pitchWithRotate: false,
      maxZoom: 19,
    });
    m.touchZoomRotate.disableRotation();
    map.current = m;
    m.on("move", place);
    m.on("resize", place);
    m.on("click", (e) => {
      if (live.current.placing) live.current.onPlace(e.lngLat.lat, e.lngLat.lng);
    });
    return () => {
      m.remove();
      map.current = null;
    };
  }, [place]);

  useEffect(() => {
    const m = map.current;
    if (!m || !host.current) return;
    let alive = true;
    void buildStyle({ online, style, dark: scheme === "dark", colours: colours(host.current), esriKey }).then(({ style: s, maxZoom }) => {
      if (!alive) return;
      m.setStyle(s, { diff: false });
      m.setMaxZoom(maxZoom);
    });
    return () => {
      alive = false;
    };
  }, [online, style, scheme, esriKey]);

  // Show the moment that is open: when it changes, fit to its pins.
  const fitted = useRef<string | null>(null);
  useEffect(() => {
    const m = map.current;
    if (!m || fitted.current === selectedGroup) return;
    const mine = pins.filter((p) => p.group === selectedGroup);
    if (mine.length === 0) return;
    fitted.current = selectedGroup;
    const lats = mine.map((p) => p.lat);
    const lons = mine.map((p) => p.lon);
    const pad = 0.005;
    m.fitBounds(
      [
        [Math.min(...lons), Math.min(...lats)],
        [Math.max(...lons, Math.min(...lons) + pad), Math.max(...lats, Math.min(...lats) + pad)],
      ],
      { padding: 64, maxZoom: 15, duration: 600 },
    );
  }, [pins, selectedGroup]);

  useLayoutEffect(place, [pins, place]);

  const register = (pin: Pin) => (el: HTMLElement | null) => {
    if (el) {
      els.current.set(pin.key, { el, lat: pin.lat, lon: pin.lon });
      const p = map.current?.project([pin.lon, pin.lat]);
      if (p) el.style.transform = `translate3d(${p.x}px, ${p.y}px, 0)`;
    } else {
      els.current.delete(pin.key);
    }
  };

  const startDrag = (pin: Pin) => (e: React.PointerEvent<HTMLButtonElement>) => {
    const m = map.current;
    const box = host.current?.getBoundingClientRect();
    const entry = els.current.get(pin.key);
    if (e.button !== 0 || !m || !box || !entry) return;
    e.stopPropagation();
    const target = e.currentTarget;
    target.setPointerCapture(e.pointerId);
    const from = { x: e.clientX, y: e.clientY };
    let moved = false;
    const at = (ev: PointerEvent) => ({ x: ev.clientX - box.left, y: ev.clientY - box.top });
    const onMove = (ev: PointerEvent) => {
      if (!moved && Math.hypot(ev.clientX - from.x, ev.clientY - from.y) < 4) return;
      moved = true;
      const p = at(ev);
      entry.el.style.transform = `translate3d(${p.x}px, ${p.y}px, 0)`;
    };
    const onUp = (ev: PointerEvent) => {
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
      target.removeEventListener("pointercancel", onUp);
      if (!moved) return onSelectPin(pin);
      const ll = m.unproject([at(ev).x, at(ev).y]);
      onMovePin(pin, ll.lat, ll.lng);
    };
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
    target.addEventListener("pointercancel", onUp);
  };

  return (
    <div className="place-map" role="application" aria-label="Map of the places to review" data-placing={placing}>
      <div ref={host} className="place-map__map" />
      <div className="place-map__pins">
        {pins.map((p) => (
          <div key={p.key} ref={register(p)} className="place-map__anchor">
            <button
              type="button"
              className="place-map__pin"
              data-state={p.state}
              data-selected={p.group === selectedGroup}
              data-confirmed={p.confirmed}
              aria-label={`${p.assets.length} ${p.assets.length === 1 ? "photo" : "photos"}, ${STATE_NAME[p.state]}`}
              title={`${p.assets.length.toLocaleString()} · ${STATE_NAME[p.state]} · drag to move`}
              onPointerDown={startDrag(p)}
            >
              {p.assets.length > 1 && <span className="place-map__count">{p.assets.length > 99 ? "99+" : p.assets.length}</span>}
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
