import { useEffect, useRef, useSyncExternalStore } from "react";
import { baseMapVersion, drawBaseMap, subscribeBaseMap } from "@/lib/basemap";
import { project, type Viewport } from "@/lib/mercator";
import "./styles.scss";

/**
 * A still map with one pin, for the viewer's info panel. Same bundled base map as the
 * Map screen, so it costs nothing extra and looks the same.
 */
export function MiniMap({
  lat,
  lon,
  width = 272,
  height = 150,
  zoom = 6,
}: {
  lat: number;
  lon: number;
  width?: number;
  height?: number;
  zoom?: number;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const view: Viewport = { lat, lon, zoom, width, height };
  const baseMap = useSyncExternalStore(subscribeBaseMap, baseMapVersion);

  useEffect(() => {
    if (canvas.current) drawBaseMap(canvas.current, view);
  }, [view, baseMap]);

  const pin = project(lat, lon, view);
  return (
    <div className="mini-map" style={{ width, height }}>
      <canvas ref={canvas} style={{ width, height }} aria-label={`Map showing ${lat.toFixed(3)}, ${lon.toFixed(3)}`} />
      <span className="mini-map__pin" style={{ left: pin.x, top: pin.y }} aria-hidden />
    </div>
  );
}
