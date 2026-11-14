import { IconRepeat, IconRepeatOff } from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Link } from "react-router";
import { ControlButton, MediaControls } from "@/components/media-controls";
import { Spinner } from "@/components/spinner";
import { usePlayback, useProxyProgress } from "@/hooks/use-media-status";
import { useSettings } from "@/hooks/use-settings";
import { matches } from "@/lib/keybinds";
import { playableAt, stepAt } from "@/lib/trim";
import type { Trim } from "@/types/generated/Trim";
import { mediaUrl } from "@/utils/media";

/**
 * Video stage of the viewer: native `<video>` when the WebView can play the file, the
 * H.264 proxy once it is transcoded (with a "Preparing playback…" poster meanwhile), or
 * the format label with a pointer to the codec card. Own compact controls; Space toggles.
 */
export function VideoPlayer({
  id,
  extension,
  trim,
  videoRef,
  onTime,
  idle = false,
}: {
  id: number;
  /** The viewer's controls are idle (Round 8B): the playback bar hides with them. */
  idle?: boolean;
  extension: string;
  /** Plays only the kept segments: at a cut it seeks on to the next one. */
  trim?: Trim | null;
  /** For the trimmer, which seeks the same element. */
  videoRef?: RefObject<HTMLVideoElement | null>;
  onTime?: (ms: number) => void;
}) {
  const { settings, update } = useSettings();
  const loop = settings?.playback.loopVideos ?? false;
  const muteDefault = settings?.playback.muteByDefault ?? false;
  const playback = usePlayback(id);
  const progress = useProxyProgress(id);
  const ownVideo = useRef<HTMLVideoElement>(null);
  const video = videoRef ?? ownVideo;
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(muteDefault);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);

  useEffect(() => setMuted(muteDefault), [muteDefault, id]);

  const segKey = trim?.segments.map((s) => `${s.start_ms}-${s.end_ms}`).join(",") ?? "";
  // biome-ignore lint/correctness/useExhaustiveDependencies: keyed by value, not identity
  const segs = useMemo(() => (trim && trim.segments.length > 0 ? trim.segments : null), [segKey]);

  // Skipping cuts: checked every frame while playing, since `timeupdate` comes only about
  // four times a second and would let a quarter-second of a cut show.
  useEffect(() => {
    const v = video.current;
    if (!v || !segs || !playing) return;
    let raf = 0;
    const tick = () => {
      const step = stepAt(segs, v.currentTime * 1000, loop);
      if (step.seek !== null) v.currentTime = step.seek / 1000;
      if (step.pause) v.pause();
      else raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [segs, playing, loop, video]);

  const info = playback.data;
  const src =
    info?.mode === "native"
      ? mediaUrl("video", id)
      : info?.mode === "proxy" && info.proxyReady
        ? mediaUrl("video", id, { proxy: 1 })
        : null;

  const toggle = () => {
    const v = video.current;
    if (!v) return;
    if (v.paused) void v.play().catch(() => {});
    else v.pause();
  };

  useEffect(() => {
    if (!src) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      if (t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA")) return;
      if (matches(e, "viewer.play-pause")) {
        e.preventDefault();
        toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [src]);

  if (!info) {
    return (
      <div className="video-player video-player--waiting" role="status">
        <Spinner label="Loading" />
      </div>
    );
  }

  if (info.mode === "unavailable") {
    return (
      <div className="viewer__unsupported" role="status">
        <span>{extension.toUpperCase()}</span>
        <p>
          This video needs FFmpeg to play. <Link to="/settings?section=playback">Set it up in Settings</Link>.
        </p>
      </div>
    );
  }

  if (!src) {
    const pct = Math.round((progress ?? 0) * 100);
    return (
      <div className="video-player video-player--preparing" role="status" aria-live="polite">
        <img className="video-player__poster" src={mediaUrl("thumb", id, { s: 1024 })} alt="" />
        <div className="video-player__preparing">
          <Spinner label="Preparing playback" />
          <span>Preparing playback… {pct}%</span>
          <span className="video-player__preparing-sub">
            {info.videoCodec?.toUpperCase() ?? extension.toUpperCase()} is converted once for smooth playback.
          </span>
        </div>
      </div>
    );
  }

  const total = duration || (info.durationMs ?? 0) / 1000;
  return (
    <div className="video-player" data-playing={playing}>
      {/* biome-ignore lint/a11y/useMediaCaption: user media has no captions */}
      <video
        ref={video}
        key={src}
        className="video-player__video"
        src={src}
        poster={mediaUrl("thumb", id, { s: 1024 })}
        autoPlay
        // A trimmed clip loops itself, from its first kept moment rather than from 0.
        loop={loop && !segs}
        muted={muted}
        playsInline
        onClick={toggle}
        onPlay={(e) => {
          // Play pressed at the end or inside a cut starts from the next kept moment.
          if (segs) {
            const v = e.currentTarget;
            const at = playableAt(segs, v.currentTime * 1000);
            v.currentTime = (at ?? segs[0].start_ms) / 1000;
          }
          setPlaying(true);
        }}
        onPause={() => setPlaying(false)}
        onTimeUpdate={(e) => {
          setTime(e.currentTarget.currentTime);
          onTime?.(e.currentTarget.currentTime * 1000);
        }}
        onDurationChange={(e) => setDuration(e.currentTarget.duration || 0)}
      />
      <MediaControls
        playing={playing}
        onToggle={toggle}
        timeMs={time * 1000}
        durationMs={total * 1000}
        onSeek={(ms) => {
          if (video.current) video.current.currentTime = ms / 1000;
          setTime(ms / 1000);
        }}
        muted={muted}
        onToggleMute={() => setMuted((m) => !m)}
        hidden={idle}
      >
        <ControlButton
          label="Loop"
          tip={loop ? "Loop on" : "Loop off"}
          pressed={loop}
          onClick={() => update((s) => ({ ...s, playback: { ...s.playback, loopVideos: !s.playback.loopVideos } }))}
        >
          {loop ? <IconRepeat size={18} /> : <IconRepeatOff size={18} />}
        </ControlButton>
      </MediaControls>
    </div>
  );
}
