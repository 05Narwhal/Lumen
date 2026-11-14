import { editVersion } from "@/lib/edit-versions";

export type MediaKind = "thumb" | "preview" | "original" | "full" | "video" | "probe" | "device-thumb" | "edit-preview" | "slideshow-audio" | "music-preview" | "library-music" | "face" | "pet" | "ai-mask" | "recovered";

export const isWindows = () =>
  typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);

export const isMac = () => typeof navigator !== "undefined" && /Mac OS X|Macintosh/i.test(navigator.userAgent);

/**
 * URL for the `photos://` protocol. Tauri exposes custom schemes as
 * `photos://localhost/...` on macOS/Linux and `http://photos.localhost/...` on Windows.
 */
export function buildMediaUrl(
  windows: boolean,
  kind: MediaKind,
  id: number | string,
  params?: Record<string, string | number>,
) {
  const base = windows ? "http://photos.localhost" : "photos://localhost";
  const qs = params
    ? "?" + new URLSearchParams(Object.entries(params).map(([k, v]) => [k, String(v)])).toString()
    : "";
  return `${base}/${kind}/${id}${qs}`;
}

type MediaUrlOverride = (kind: MediaKind, id: number | string, params?: Record<string, string | number>) => string;

/** Dev-only: the browser shim swaps in placeholder images because there is no protocol handler. */
function override(): MediaUrlOverride | undefined {
  if (!import.meta.env.DEV) return undefined;
  return (window as unknown as { __LUMEN_MEDIA_URL__?: MediaUrlOverride }).__LUMEN_MEDIA_URL__;
}

/** Kinds whose image changes when the photo is edited; their URLs carry the edit version. */
const EDITABLE: MediaKind[] = ["thumb", "preview", "full"];

export const mediaUrl = (kind: MediaKind, id: number | string, params?: Record<string, string | number>) => {
  const v = EDITABLE.includes(kind) ? editVersion(Number(id)) : undefined;
  const p = v ? { ...params, v } : params;
  return override()?.(kind, id, p) ?? buildMediaUrl(isWindows(), kind, id, p);
};
