import type { PresenceScreen } from "@/types/generated/PresenceScreen";
import { createStore } from "./store";

/** True while a slideshow plays in its editor (the film player has its own store). */
export const slideshowPlayingStore = createStore(false);

/**
 * What Lumen is doing, as coarse as it gets (Round 17). This is the only thing the Discord
 * status ever learns from the page: one of four fixed words, never a path, id, name or query.
 */
export function screenFor(input: { pathname: string; search: string; film: boolean; slideshowPlaying: boolean }): PresenceScreen {
  if (input.film || input.slideshowPlaying) return "slideshow";
  if (new URLSearchParams(input.search).has("asset")) return "viewing";
  if (input.pathname.startsWith("/develop")) return "developing";
  return "browsing";
}
