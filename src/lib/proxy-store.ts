import { createStore } from "@/lib/store";
import type { ProxyFinished } from "@/types/generated/ProxyFinished";

/** Live proxy-transcode progress per asset id, written by the event bridge. */
export const proxyStore = createStore<{ progress: Record<number, number> }>({ progress: {} });

export function setProxyProgress(assetId: number, progress: number) {
  proxyStore.set((s) => ({ progress: { ...s.progress, [assetId]: progress } }));
}

export function clearProxyProgress(assetId: number) {
  proxyStore.set((s) => {
    const { [assetId]: _, ...rest } = s.progress;
    return { progress: rest };
  });
}

/** What to say when a proxy fails: nothing for one the background prep made unasked (Round 9A). */
export function proxyFailureMessage(p: ProxyFinished): string | null {
  return p.ok || p.background ? null : "This video could not be converted for playback.";
}
