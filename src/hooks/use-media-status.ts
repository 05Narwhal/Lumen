import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { invalidateLayouts } from "@/lib/events";
import { invoke } from "@/lib/ipc";
import { proxyStore } from "@/lib/proxy-store";
import { queryKeys } from "@/lib/query-keys";
import { reportVideoSupport } from "@/lib/video-support";
import type { FfmpegStatus } from "@/types/generated/FfmpegStatus";
import type { PlaybackInfo } from "@/types/generated/PlaybackInfo";

/** FFmpeg presence. `check()` asks the backend to look again (after an install). */
export function useFfmpegStatus() {
  const qc = useQueryClient();
  const q = useQuery({
    queryKey: queryKeys.ffmpeg,
    queryFn: () => invoke<FfmpegStatus>("get_ffmpeg_status"),
    staleTime: 60_000,
  });
  const check = useCallback(async () => {
    const s = await qc.fetchQuery({ queryKey: queryKeys.ffmpeg, queryFn: () => invoke<FfmpegStatus>("get_ffmpeg_status"), staleTime: 0 });
    if (s.available) {
      void invalidateLayouts(qc);
      qc.invalidateQueries({ queryKey: ["asset"] });
      qc.invalidateQueries({ queryKey: ["playback"] });
    }
    return s;
  }, [qc]);
  return { status: q.data, isFetching: q.isFetching, check };
}

/** How the viewer should play a video; asks the backend to start the proxy when needed. */
/** The query behind `usePlayback`, shared with the slideshow preview (Round 6). */
export const playbackQuery = (id: number) => ({
  queryKey: queryKeys.playback(id),
  queryFn: async () => {
    // Rust decides native or proxy from what this WebView plays (Round 8A).
    await reportVideoSupport();
    const info = await invoke<PlaybackInfo>("get_playback", { id });
    if (info.mode === "proxy" && !info.proxyReady) await invoke("prepare_proxy", { id }).catch(() => {});
    return info;
  },
  staleTime: 10_000,
});

export function usePlayback(id: number | null) {
  return useQuery({ ...playbackQuery(id ?? -1), enabled: id !== null });
}

/** 0..1 while a proxy transcodes, undefined otherwise (fed by the event bridge). */
export function useProxyProgress(id: number | null): number | undefined {
  const s = proxyStore.use();
  return id === null ? undefined : s.progress[id];
}

export function useProxyCache() {
  const qc = useQueryClient();
  const size = useQuery({ queryKey: queryKeys.proxyCache, queryFn: () => invoke<number>("get_proxy_cache_size") });
  const clear = useCallback(async () => {
    const freed = await invoke<number>("clear_proxy_cache");
    qc.invalidateQueries({ queryKey: queryKeys.proxyCache });
    qc.invalidateQueries({ queryKey: ["playback"] });
    return freed;
  }, [qc]);
  return { bytes: size.data ?? 0, clear };
}
