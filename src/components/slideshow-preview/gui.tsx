import { useQueries } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useElementSize } from "@/hooks/use-element-size";
import { playbackQuery } from "@/hooks/use-media-status";
import { frameAt, timeline, videoTimeAt } from "@/lib/slideshow-timing";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { mediaUrl } from "@/utils/media";
import "./styles.scss";

/** How wide the backdrop blur is, as a fraction of the frame's width. */
const BLUR_FRACTION = 1 / 40;
/** The backdrop is darkened so the sharp copy is what the eye lands on (Rust dims by 0.82). */
const BACKDROP_DIM = 0.18;
/** How far a playing video may drift from the show before it is put back, in seconds. */
const VIDEO_DRIFT_S = 0.3;

/** A picture's own size: a video's frame, or an image's. */
function sizeOf(src: HTMLImageElement | HTMLVideoElement | undefined): [number, number] {
  if (!src) return [0, 0];
  if (typeof HTMLVideoElement !== "undefined" && src instanceof HTMLVideoElement) return [src.videoWidth, src.videoHeight];
  return [src.width, src.height];
}

/**
 * How a frame is drawn, matching `photos_media::frame::compose` and the geometry both it
 * and this read from `frameLayout`: each placement is a rectangle in frame coordinates,
 * painted in order, with the clip's motion and the transition applied to the whole frame.
 */
export function drawFrame(
  ctx: CanvasRenderingContext2D,
  doc: SlideshowDoc,
  timeMs: number,
  size: { width: number; height: number },
  images: Map<number, HTMLImageElement>,
  videos: Map<number, HTMLVideoElement> = new Map(),
) {
  const { width, height } = size;
  const frameAspect = doc.resolution[0] / Math.max(1, doc.resolution[1]);
  const aspectOf = (assetId: number) => {
    const [w, h] = sizeOf(images.get(assetId) ?? videos.get(assetId));
    return w && h ? w / h : frameAspect;
  };
  ctx.clearRect(0, 0, width, height);
  const { layers, backdrop } = frameAt(doc, timeMs, aspectOf);
  if (backdrop) {
    ctx.fillStyle = backdrop === "black" ? "#000" : "#fff";
    ctx.fillRect(0, 0, width, height);
  }
  for (const layer of layers) {
    ctx.save();
    if (layer.clip?.kind === "rect") {
      ctx.beginPath();
      ctx.rect(layer.clip.x * width, layer.clip.y * height, layer.clip.w * width, layer.clip.h * height);
      ctx.clip();
    } else if (layer.clip?.kind === "circle") {
      ctx.beginPath();
      ctx.arc(width / 2, height / 2, layer.clip.r * Math.hypot(width, height), 0, Math.PI * 2);
      ctx.clip();
    } else if (layer.clip?.kind === "stripes") {
      const { axis, count, p } = layer.clip;
      const band = 1 / count;
      const filled = band * p;
      ctx.beginPath();
      for (let i = 0; i < count; i += 1) {
        // Alternate bands fill from the far edge, so the picture knits together rather
        // than marching across in one direction.
        const start = i * band + (i % 2 === 0 ? 0 : band - filled);
        if (axis === "x") ctx.rect(start * width, 0, filled * width, height);
        else ctx.rect(0, start * height, width, filled * height);
      }
      ctx.clip();
    } else if (layer.clip?.kind === "scale") {
      // Squashed about the frame's centre, before the motion transform below.
      ctx.translate(width / 2, height / 2);
      ctx.scale(Math.max(layer.clip.sx, 0.0001), Math.max(layer.clip.sy, 0.0001));
      ctx.translate(-width / 2, -height / 2);
    }
    ctx.globalAlpha = layer.alpha;
    // The frame is zoomed about a centre moved by (x, y) frame widths and heights, then
    // carried sideways or upward by the transition.
    ctx.translate(width / 2 + layer.offsetX * width, height / 2 + layer.offsetY * height);
    ctx.scale(layer.scale, layer.scale);
    ctx.translate(-width * (0.5 + layer.x), -height * (0.5 + layer.y));
    // A video clip paints its video, as it plays (Round 6); a photograph its picture.
    const isVideo = Boolean(doc.clips[layer.index]?.video);
    for (const placement of layer.placements) {
      const asset = layer.assets[placement.photo];
      // Until the video has a frame to show, its poster stands in.
      const video = isVideo ? videos.get(asset) : undefined;
      const img = video && video.readyState >= 2 && video.videoWidth > 0 ? video : images.get(asset);
      const [iw, ih] = sizeOf(img);
      if (!img || !iw || !ih) continue;
      const [x, y, w, h] = [placement.rect.x * width, placement.rect.y * height, placement.rect.w * width, placement.rect.h * height];
      if (placement.blurred) {
        ctx.filter = `blur(${Math.round(width * BLUR_FRACTION)}px)`;
        ctx.drawImage(img, x, y, w, h);
        ctx.filter = "none";
        ctx.fillStyle = `rgba(0, 0, 0, ${BACKDROP_DIM})`;
        ctx.fillRect(x, y, w, h);
      } else {
        ctx.drawImage(img, x, y, w, h);
      }
    }
    ctx.restore();
  }
}

/** Decibels as a multiplier, with the fades at either end of the show. */
export function audioVolume(doc: SlideshowDoc, timeMs: number, totalMs: number): number {
  const track = doc.audio;
  if (!track) return 0;
  let v = 10 ** (track.gain_db / 20);
  if (track.fade_in_ms > 0) v *= Math.min(1, Math.max(0, timeMs / track.fade_in_ms));
  if (track.fade_out_ms > 0) v *= Math.min(1, Math.max(0, (totalMs - timeMs) / track.fade_out_ms));
  return Math.min(1, Math.max(0, v));
}

/**
 * Where the track plays from. A library track (`music/…`) by its own name, because the show's
 * own audio route reads the *saved* show and the editor saves 600 ms after a change: a track
 * just picked was asked for too early, failed, and never retried. Anything else goes through
 * the show, with the path in the URL so a new track is a new URL.
 */
export function audioSrc(doc: SlideshowDoc, slideshowId: number, retry = 0): string | null {
  const path = doc.audio?.path;
  if (!path) return null;
  const inLibrary = /^music[\\/]([^\\/]+)$/.exec(path);
  return inLibrary ? mediaUrl("library-music", inLibrary[1]) : mediaUrl("slideshow-audio", slideshowId, retry ? { t: path, r: retry } : { t: path });
}

/** A file from elsewhere can still be asked for before the save lands: ask again, briefly. */
const AUDIO_RETRIES = 3;
const AUDIO_RETRY_MS = 800;

type Props = {
  doc: SlideshowDoc;
  slideshowId: number;
  /** Where the show is, in milliseconds. The editor owns it; playing reports back. */
  timeMs: number;
  playing: boolean;
  onTime: (ms: number) => void;
  /** Called once when playback reaches the end. */
  onEnded?: () => void;
  /** The music silenced (a memory's film, Round 6). */
  muted?: boolean;
};

/**
 * The editor's canvas: the document drawn at `timeMs`, and while `playing` a single
 * `requestAnimationFrame` clock that reports the time back so the transport and the
 * timeline follow the same value.
 */
export function SlideshowPreview({ doc, slideshowId, timeMs, playing, onTime, onEnded, muted = false }: Props) {
  const [boxRef, size] = useElementSize<HTMLDivElement>(50);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const audioRef = useRef<HTMLAudioElement>(null);
  const images = useRef(new Map<number, HTMLImageElement>());
  const videos = useRef(new Map<number, HTMLVideoElement>());
  const total = timeline(doc).totalMs;
  const [retry, setRetry] = useState({ path: "", n: 0 });
  const path = doc.audio?.path ?? "";
  const tries = retry.path === path ? retry.n : 0;
  const src = audioSrc(doc, slideshowId, tries);
  const retryTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(retryTimer.current), []);
  const onAudioError = () => {
    if (tries >= AUDIO_RETRIES || /^music[\\/]/.test(path)) return;
    clearTimeout(retryTimer.current);
    retryTimer.current = setTimeout(() => setRetry({ path, n: tries + 1 }), AUDIO_RETRY_MS);
  };
  /** How much of the track's head the beat sync dropped: the show's 0 is this far in. */
  const startMs = doc.audio?.start_ms ?? 0;
  /** Where the show is while it runs, without a re-render per frame to carry it. */
  const clock = useRef(timeMs);
  /**
   * The times our own clock has reported and React may not have rendered yet. A `timeMs`
   * that is one of these is ours arriving late; anything else came from outside. One
   * remembered value was not enough: at a transition the render falls frames behind, and
   * a late time read as a seek sent the track back to it — the music repeated a moment.
   */
  const emitted = useRef<number[]>([]);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    drawFrame(ctx, doc, timeMs, { width: canvas.width, height: canvas.height }, images.current, videos.current);
  }, [doc, timeMs]);
  const drawLatest = useRef(draw);
  drawLatest.current = draw;

  // Every video clip's video (Round 6): muted, played natively or through its H.264 proxy as
  // the viewer would, and painted into the canvas like a photograph.
  const videoIds = [...new Set(doc.clips.filter((c) => c.video).map((c) => c.asset_id))];
  const playbacks = useQueries({ queries: videoIds.map((id) => playbackQuery(id)) });
  const videoSrcs = videoIds.map((id, i) => {
    const info = playbacks[i]?.data;
    if (info?.mode === "native") return mediaUrl("video", id);
    if (info?.mode === "proxy" && info.proxyReady) return mediaUrl("video", id, { proxy: 1 });
    return null;
  });
  const videoKey = videoIds.map((id, i) => `${id}=${videoSrcs[i] ?? ""}`).join(",");
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed by value, not identity
  useEffect(() => {
    const wanted = new Map(videoIds.map((id, i) => [id, videoSrcs[i]] as const).filter(([, s]) => s !== null));
    for (const [id, v] of videos.current) {
      if (wanted.get(id) === v.getAttribute("src")) continue;
      v.pause();
      v.removeAttribute("src");
      videos.current.delete(id);
    }
    for (const [id, src] of wanted) {
      if (!src || videos.current.has(id)) continue;
      const v = document.createElement("video");
      v.muted = true;
      v.playsInline = true;
      v.preload = "auto";
      v.addEventListener("loadeddata", () => drawLatest.current());
      v.addEventListener("seeked", () => drawLatest.current());
      v.setAttribute("src", src);
      videos.current.set(id, v);
    }
  }, [videoKey]);
  useEffect(
    () => () => {
      for (const v of videos.current.values()) {
        v.pause();
        v.removeAttribute("src");
      }
      videos.current.clear();
    },
    [],
  );

  // Each visible video is where the show says it is: playing along while the show plays,
  // seeked and still while it is paused, and paused when off screen. Small drift while
  // playing is left alone — seeking a playing video stutters it.
  useEffect(() => {
    const onScreen = new Map<number, number>();
    for (const layer of frameAt(doc, timeMs).layers) {
      const at = videoTimeAt(doc, layer.index, timeMs);
      if (at !== null) onScreen.set(doc.clips[layer.index].asset_id, at / 1000);
    }
    for (const [id, v] of videos.current) {
      const at = onScreen.get(id);
      if (at === undefined) {
        if (!v.paused) v.pause();
        continue;
      }
      if (playing) {
        if (v.paused) {
          v.currentTime = at;
          void v.play().catch(() => {});
        } else if (Math.abs(v.currentTime - at) > VIDEO_DRIFT_S) {
          v.currentTime = at;
        }
      } else {
        if (!v.paused) v.pause();
        if (Math.abs(v.currentTime - at) > 0.04) v.currentTime = at;
      }
    }
  }, [doc, timeMs, playing]);

  // Every clip's photo, kept for as long as the show holds it: the same picture is often
  // on screen for several seconds and again in the next transition.
  useEffect(() => {
    const wanted = new Set(doc.clips.flatMap((c) => [c.asset_id, ...c.extra]));
    for (const id of images.current.keys()) if (!wanted.has(id)) images.current.delete(id);
    for (const id of wanted) {
      if (images.current.has(id)) continue;
      const img = new Image();
      img.onload = () => draw();
      img.src = mediaUrl("preview", id);
      images.current.set(id, img);
    }
  }, [doc, draw]);

  // The canvas is sized in device pixels, so the picture is sharp on a scaled display.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || size.width === 0) return;
    const dpr = typeof window === "undefined" ? 1 : Math.min(2, window.devicePixelRatio || 1);
    const [w, h] = doc.resolution;
    canvas.width = Math.round(size.width * dpr);
    canvas.height = Math.round((size.width * h) / w) * dpr;
    draw();
  }, [size.width, doc.resolution, draw]);

  useEffect(draw, [draw]);

  // A time that did not come from our own clock — the transport, the clip strip, a drag
  // along the timeline, the first render — moves both the clock and the track. A time that
  // did is left alone. Chasing the element's own `currentTime` every frame is what made
  // the track stutter: a beat-synced show trims the track's head, so the show's time and
  // the element's are permanently `start_ms` apart, every frame read that as drift, and
  // every frame seeked. Each seek restarts the decoder, so nothing ever played.
  const lastStart = useRef(startMs);
  useEffect(() => {
    const own = emitted.current.findIndex((t) => Math.abs(t - timeMs) < 0.5);
    const sameTrim = lastStart.current === startMs;
    lastStart.current = startMs;
    if (own !== -1 && sameTrim) {
      // Everything reported before it has been rendered past; forget those.
      emitted.current.splice(0, own + 1);
      return;
    }
    clock.current = timeMs;
    emitted.current = [];
    const audio = audioRef.current;
    if (audio) audio.currentTime = (startMs + timeMs) / 1000;
  }, [timeMs, startMs]);

  // A new element (a new track, or a retry) starts at 0:00; put it where the show is. The
  // first element is placed by the effect above, which runs on mount.
  const firstSrc = useRef(src);
  useEffect(() => {
    if (src === firstSrc.current) return;
    firstSrc.current = src;
    const audio = audioRef.current;
    if (audio) audio.currentTime = (startMs + clock.current) / 1000;
    // Only a change of element moves it; the clock and trim are read, not watched.
  }, [src]);

  // Starting and stopping, once per change. This used to sit in the per-frame effect
  // below, which left a queue of interrupted `play()` requests behind every seek.
  useEffect(() => {
    const audio = audioRef.current;
    if (!playing) {
      audio?.pause();
      return;
    }
    // Pressing play at the end replays from the top, and the track goes with it. Any other
    // position the effect above has already put the track at, so it is left where it is —
    // resuming after a pause must not seek, which is audible.
    if (clock.current >= total) {
      clock.current = 0;
      if (audio) audio.currentTime = startMs / 1000;
    }
    emitted.current = [];
    void audio?.play().catch(() => {});
    // `src`: a new track is a new element, which starts paused.
  }, [playing, total, startMs, src]);

  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    let last = performance.now();
    const tick = (now: number) => {
      // While the track is playing it is the clock: the picture follows the music rather
      // than the music being pulled to the picture, so the track plays straight through.
      // Before it starts, after it ends, or with no track, frames advance by wall time.
      const audio = audioRef.current;
      const audible = audio && !audio.paused && !audio.ended && audio.readyState >= 2;
      const next = audible ? Math.max(clock.current, audio.currentTime * 1000 - startMs) : clock.current + (now - last);
      clock.current = Math.min(total, next);
      last = now;
      emitted.current.push(clock.current);
      if (emitted.current.length > 240) emitted.current.shift();
      onTime(clock.current);
      if (clock.current >= total) {
        onEnded?.();
        return;
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing, total, startMs, onTime, onEnded]);

  // The volume does follow every frame, because the fades are part of the document and a
  // float assignment costs nothing.
  useEffect(() => {
    const audio = audioRef.current;
    if (audio) audio.volume = audioVolume(doc, timeMs, total);
  }, [doc, timeMs, total]);

  return (
    <div className="slideshow-preview" ref={boxRef} style={{ aspectRatio: `${doc.resolution[0]} / ${doc.resolution[1]}` }}>
      <canvas ref={canvasRef} className="slideshow-preview__canvas" aria-label="Slideshow preview" />
      {src && <audio key={src} ref={audioRef} src={src} preload="auto" muted={muted} onError={onAudioError} />}
    </div>
  );
}
