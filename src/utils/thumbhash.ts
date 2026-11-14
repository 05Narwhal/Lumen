import { thumbHashToDataURL } from "thumbhash";

const cache = new Map<string, string>();
const MAX = 2000;

function fromBase64(b64: string): Uint8Array {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Decodes a base64 ThumbHash into a tiny PNG data URL. Memoised (LRU-ish). */
export function thumbhashToDataUrl(b64: string): string {
  const hit = cache.get(b64);
  if (hit) return hit;
  let url: string;
  try {
    url = thumbHashToDataURL(fromBase64(b64));
  } catch {
    url = "";
  }
  if (cache.size >= MAX) {
    const first = cache.keys().next().value;
    if (first !== undefined) cache.delete(first);
  }
  cache.set(b64, url);
  return url;
}
