// What the face radar checks after a correction (Round 10): the photos the user is looking at.
// With the viewer open, the grid it pages through (filter and all), nearest to the open photo
// first; with the viewer closed, the grid on screen. Without either, Rust checks the photos
// taken around the correction.
import type { ViewerScope } from "./viewer-scope";
import { invoke } from "./ipc";

let grid: ViewerScope | null = null;
let viewer: { scope: ViewerScope | null; at: number } | null = null;
let sent = "";

function send() {
  const scope = viewer ? viewer.scope : grid;
  const args = scope && scope.width > 0 ? { ...scope, at: viewer?.at ?? null, open: true } : { open: false };
  const key = JSON.stringify(args);
  if (key === sent) return;
  sent = key;
  invoke("set_face_radar_view", args).catch(() => {});
}

/** The grid on screen, or `null` when it goes. */
export function noteRadarGrid(scope: ViewerScope | null) {
  grid = scope;
  send();
}

/** The grid the open viewer pages through (`null` for none) and its photo; `null` when it closes. */
export function noteRadarViewer(v: { scope: ViewerScope | null; at: number } | null) {
  viewer = v;
  send();
}

/** The grid on screen, for *Select all* too. */
export function gridOnScreen() {
  return grid;
}
