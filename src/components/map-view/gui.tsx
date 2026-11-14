import { useComputedColorScheme } from "@mantine/core";
import { Map as MapLibre, setWorkerUrl, type PaddingOptions } from "maplibre-gl";
import "maplibre-gl/dist/maplibre-gl.css";
// The worker imports a shared chunk by relative path, so it is bundled as one module here
// rather than resolved next to the library, which breaks under Vite's dependency bundling.
import workerUrl from "maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url";
import { forwardRef, useCallback, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from "react";
import { buildStyle, clusterZoom, SATELLITE_MAX_ZOOM, toBounds, type MapColours } from "@/lib/map-style";
import type { Bounds } from "@/types/generated/Bounds";
import type { Cluster } from "@/types/generated/Cluster";
import type { MapStyle } from "@/types/generated/MapStyle";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

setWorkerUrl(workerUrl);

export interface MapViewHandle {
  fitBounds: (b: Bounds, padding?: number | PaddingOptions) => void;
  flyTo: (lat: number, lon: number, zoom: number) => void;
  zoom: () => number;
}

export interface MapViewProps {
  clusters: Cluster[];
  online: boolean;
  style: MapStyle;
  /** Where the map opens before anything else is known. */
  initial?: { lat: number; lon: number; zoom: number };
  /** Debounced after the camera settles: the viewport and the zoom the clustering uses. */
  onBoundsChange: (b: Bounds, zoom: number) => void;
  onOpenAsset: (id: number) => void;
  onZoomToCluster: (c: Cluster) => void;
  /** Right-click on a pin (Round 4G): its menu. */
  onClusterMenu?: (c: Cluster, e: React.MouseEvent) => void;
  /** True while the online style could not be reached and the offline floor is showing. */
  onOfflineChange?: (offline: boolean) => void;
}

/** The theme colours the offline floor is drawn in, resolved on the element. */
function colours(el: HTMLElement): MapColours {
  const cs = getComputedStyle(el);
  const t = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback;
  return { water: t("--map-water", "#0e0e0e"), land: t("--map-land", "#1a1a1a"), coast: t("--map-coast", "#666"), border: t("--map-border", "#555") };
}

const markerKey = (c: Cluster) => `${c.coverAssetId}:${c.count}`;

/**
 * The Map screen's map: MapLibre GL for the base map (smooth zoom, vector detail, satellite)
 * and DOM markers on top, one per cluster from Rust, so each carries a real thumbnail and
 * behaves like a button. Markers are moved directly on every camera frame — no React render
 * per frame — and hover scales an inner element, so the thing under the pointer never moves.
 */
export const MapView = forwardRef<MapViewHandle, MapViewProps>(function MapView(
  { clusters, online, style, initial, onBoundsChange, onOpenAsset, onZoomToCluster, onClusterMenu, onOfflineChange },
  ref,
) {
  const host = useRef<HTMLDivElement>(null);
  const map = useRef<MapLibre | null>(null);
  const markers = useRef(new Map<string, { el: HTMLElement; lat: number; lon: number }>());
  const [ready, setReady] = useState(false);
  const scheme = useComputedColorScheme("light");
  const callbacks = useRef({ onBoundsChange, onOfflineChange });
  callbacks.current = { onBoundsChange, onOfflineChange };

  // Put every marker where the camera says it is. Called on every move frame.
  const place = useCallback(() => {
    const m = map.current;
    if (!m) return;
    for (const { el, lat, lon } of markers.current.values()) {
      const p = m.project([lon, lat]);
      el.style.transform = `translate3d(${p.x}px, ${p.y}px, 0)`;
    }
  }, []);

  useEffect(() => {
    if (!host.current) return;
    const start = initial ?? { lat: 20, lon: 0, zoom: 1 };
    const m = new MapLibre({
      container: host.current,
      style: { version: 8, sources: {}, layers: [] },
      center: [start.lon, start.lat],
      zoom: start.zoom,
      attributionControl: { compact: true },
      dragRotate: false,
      pitchWithRotate: false,
      maxZoom: 19,
    });
    m.touchZoomRotate.disableRotation();
    map.current = m;
    let settle: ReturnType<typeof setTimeout> | undefined;
    const report = () => {
      clearTimeout(settle);
      settle = setTimeout(() => callbacks.current.onBoundsChange(toBounds(boundsOf(m)), clusterZoom(m.getZoom())), 120);
    };
    m.on("move", place);
    m.on("moveend", report);
    m.on("resize", place);
    m.once("load", () => {
      setReady(true);
      report();
    });
    return () => {
      clearTimeout(settle);
      m.remove();
      map.current = null;
    };
    // The map is created once; `initial` only matters for that first frame.
  }, [place]);

  // The base map follows the setting, the Map/Satellite choice and the colour scheme. A
  // connection that returns is noticed and the detailed map comes back on its own.
  useEffect(() => {
    const m = map.current;
    if (!m || !host.current) return;
    let alive = true;
    const apply = () =>
      void buildStyle({ online, style, dark: scheme === "dark", colours: colours(host.current!) }).then(({ style: s, offline }) => {
        if (!alive) return;
        m.setStyle(s, { diff: false });
        // Satellite stops before the imagery runs out; the vector map goes to street level.
        m.setMaxZoom(online && !offline && style === "satellite" ? SATELLITE_MAX_ZOOM : 19);
        callbacks.current.onOfflineChange?.(online && offline);
      });
    apply();
    window.addEventListener("online", apply);
    return () => {
      alive = false;
      window.removeEventListener("online", apply);
    };
  }, [online, style, scheme]);

  useLayoutEffect(place, [clusters, ready, place]);

  useImperativeHandle(
    ref,
    () => ({
      fitBounds: (b, padding: number | PaddingOptions = 64) => {
        const m = map.current;
        if (!m) return;
        // A single point has no extent; give it one so fitBounds does not zoom to the max.
        const pad = 0.005;
        m.fitBounds(
          [
            [Math.min(b.minLon, b.maxLon - pad), Math.min(b.minLat, b.maxLat - pad)],
            [Math.max(b.maxLon, b.minLon + pad), Math.max(b.maxLat, b.minLat + pad)],
          ],
          { padding, maxZoom: 16, duration: 700 },
        );
      },
      flyTo: (lat, lon, zoom) => map.current?.flyTo({ center: [lon, lat], zoom, duration: 1200 }),
      zoom: () => map.current?.getZoom() ?? 0,
    }),
    [],
  );

  const register = (key: string, c: Cluster) => (el: HTMLElement | null) => {
    if (el) {
      markers.current.set(key, { el, lat: c.lat, lon: c.lon });
      const p = map.current?.project([c.lon, c.lat]);
      if (p) el.style.transform = `translate3d(${p.x}px, ${p.y}px, 0)`;
    } else {
      markers.current.delete(key);
    }
  };

  return (
    <div className="map-view" role="application" aria-label="Map of your photos">
      <div ref={host} className="map-view__map" />
      <div className="map-view__markers">
        {clusters.map((c) => {
          const single = c.assetId != null;
          const key = markerKey(c);
          return (
            <div key={key} ref={register(key, c)} className="map-view__marker">
              <button
                type="button"
                className="map-view__pin"
                data-single={single}
                title={single ? "Open this photo" : `${c.count.toLocaleString()} photos here · open them`}
                aria-label={single ? "Open this photo" : `Open ${c.count} photos`}
                onClick={() => (single && c.assetId != null ? onOpenAsset(c.assetId) : onZoomToCluster(c))}
                onContextMenu={onClusterMenu ? (e) => onClusterMenu(c, e) : undefined}
              >
                <img src={mediaUrl("thumb", c.coverAssetId, { s: 256 })} alt="" loading="lazy" decoding="async" draggable={false} />
                {!single && <span className="map-view__count">{c.count > 999 ? "999+" : c.count}</span>}
              </button>
            </div>
          );
        })}
      </div>
    </div>
  );
});

function boundsOf(m: MapLibre) {
  const b = m.getBounds();
  return { west: b.getWest(), south: b.getSouth(), east: b.getEast(), north: b.getNorth() };
}
