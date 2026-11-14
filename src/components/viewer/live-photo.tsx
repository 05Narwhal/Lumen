import { IconLivePhoto } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import { usePlayback } from "@/hooks/use-media-status";
import { mediaUrl } from "@/utils/media";

/**
 * A Live Photo's motion: the still's paired movie (photos-core pairs them at import), played
 * muted over the still on demand — hovering the LIVE button, holding the photo, or Space —
 * the way Photos does it. The movie goes through the same playback path as any video, so a
 * codec the webview lacks gets the H.264 proxy.
 */
export function useLiveMotion(movieId: number | null) {
  const playback = usePlayback(movieId);
  const info = playback.data;
  const [playing, setPlaying] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    setPlaying(false);
    setFailed(false);
  }, [movieId]);
  const src =
    movieId == null || failed
      ? null
      : info?.mode === "native"
        ? mediaUrl("video", movieId)
        : info?.mode === "proxy" && info.proxyReady
          ? mediaUrl("video", movieId, { proxy: 1 })
          : null;
  return {
    isLive: movieId != null,
    src,
    playing: playing && src != null,
    play: () => setPlaying(true),
    stop: () => setPlaying(false),
    toggle: () => setPlaying((p) => !p),
    fail: () => {
      setFailed(true);
      setPlaying(false);
    },
  };
}

export type LiveMotion = ReturnType<typeof useLiveMotion>;

/**
 * The movie over the still. Mounted from the first play on and kept, so stopping fades it
 * out instead of cutting; every play starts from the beginning.
 */
export function LiveMotionLayer({ live }: { live: LiveMotion }) {
  const video = useRef<HTMLVideoElement>(null);
  const [mounted, setMounted] = useState(false);
  useEffect(() => {
    if (live.playing) setMounted(true);
  }, [live.playing]);
  useEffect(() => {
    const v = video.current;
    if (!v) return;
    if (live.playing) {
      v.currentTime = 0;
      // jsdom has no media playback; a real webview returns a promise.
      v.play?.()?.catch(() => live.fail());
    } else {
      v.pause?.();
    }
  }, [live.playing, mounted, live]);
  if (!mounted || !live.src) return null;
  return (
    <video
      ref={video}
      className="viewer__live"
      src={live.src}
      muted
      playsInline
      data-playing={live.playing}
      onEnded={live.stop}
      onError={live.fail}
      aria-hidden
    />
  );
}

export function LiveButton({ live }: { live: LiveMotion }) {
  if (!live.isLive) return null;
  const ready = live.src != null;
  return (
    <button
      type="button"
      className="viewer__live-button"
      data-playing={live.playing}
      disabled={!ready}
      aria-label={ready ? "Play Live Photo (Space)" : "Live Photo motion is not available"}
      title={ready ? "Hover or hold the photo to play · Space" : "The motion could not be played"}
      onPointerEnter={live.play}
      onPointerLeave={live.stop}
      onClick={live.toggle}
    >
      <IconLivePhoto size={14} />
      LIVE
    </button>
  );
}
