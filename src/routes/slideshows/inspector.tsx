import { Button, Select, NumberInput, Slider, Switch } from "@mantine/core";
import { IconArrowsShuffle, IconMusic, IconSparkles, IconTrash, IconWaveSine } from "@tabler/icons-react";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { MusicPicker } from "@/components/music-picker";
import { Segmented } from "@/components/segmented";
import { MAX_CLIP_MS, MIN_CLIP_MS } from "@/hooks/use-slideshow-editor";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { timeline } from "@/lib/slideshow-timing";
import type { Clip } from "@/types/generated/Clip";
import type { FrameKind } from "@/types/generated/FrameKind";
import type { MusicWindow } from "@/types/generated/MusicWindow";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import type { TransitionKind } from "@/types/generated/TransitionKind";
import type { VideoSpan } from "@/types/generated/VideoSpan";

/** The pans `SlideshowDoc::new_for` uses, so a motion switched on here matches a new show. */
const PANS: [number, number][] = [
  [0.02, 0.01],
  [-0.02, 0.01],
  [0.015, -0.01],
  [-0.01, -0.015],
];
const ZOOM = 1.12;

const AUDIO_EXTENSIONS = ["mp3", "m4a", "aac", "wav", "flac", "ogg"];

const kenBurns = (index: number, direction: "in" | "out"): Clip["motion"] => {
  const [dx, dy] = PANS[index % PANS.length];
  const still: [number, number, number] = [0, 0, 1];
  const moved: [number, number, number] = [dx, dy, ZOOM];
  return direction === "in" ? { kind: "ken_burns", from: still, to: moved } : { kind: "ken_burns", from: moved, to: still };
};

const seconds = (ms: number) => `${(ms / 1000).toFixed(1)}s`;
/** A place in a track, as a player shows it: `1:05`. */
const clock = (ms: number) => `${Math.floor(ms / 60_000)}:${String(Math.floor((ms % 60_000) / 1000)).padStart(2, "0")}`;

/**
 * Every join between two clips, grouped so twenty of them stay findable. The order matches
 * `TransitionKind::ALL` in Rust, and `TRANSITION_KINDS` in the preview.
 */
const TRANSITIONS: { group: string; items: { value: TransitionKind; label: string }[] }[] = [
  {
    group: "Fade",
    items: [
      { value: "none", label: "None" },
      { value: "crossfade", label: "Crossfade" },
      { value: "fadeblack", label: "Through black" },
      { value: "fadewhite", label: "Through white" },
    ],
  },
  {
    group: "Slide",
    items: [
      { value: "slide", label: "Slide" },
      { value: "slideup", label: "Slide up" },
      { value: "slidedown", label: "Slide down" },
      { value: "cover", label: "Cover" },
      { value: "reveal", label: "Reveal" },
    ],
  },
  {
    group: "Wipe",
    items: [
      { value: "wipe", label: "Wipe" },
      { value: "wipeup", label: "Wipe up" },
      { value: "corner", label: "Corner" },
      { value: "vertopen", label: "Open vertically" },
      { value: "horzopen", label: "Open across" },
    ],
  },
  {
    group: "Shape",
    items: [
      { value: "circle", label: "Circle in" },
      { value: "circleclose", label: "Circle out" },
      { value: "squeeze", label: "Squeeze" },
      { value: "zoom", label: "Zoom" },
    ],
  },
  {
    group: "Grid",
    items: [
      { value: "slices", label: "Slices" },
      { value: "vslices", label: "Bands" },
    ],
  },
];


/** How a clip's photographs meet the frame. The twin of Rust's `FrameKind`. */
const FRAMES: { value: FrameKind; label: string }[] = [
  { value: "fill", label: "Fill the frame" },
  { value: "blur", label: "Whole, on a blurred backdrop" },
  { value: "grid", label: "Side by side" },
  { value: "quad", label: "Four up" },
  { value: "feature", label: "One large, two small" },
  { value: "stack", label: "Stacked" },
];

/** `FrameKind::capacity` in Rust; the note below the picker reads from it. */
const FRAME_CAPACITY: Record<FrameKind, number> = { fill: 1, blur: 1, grid: 3, quad: 4, feature: 3, stack: 2 };

function ClipPanel({ clip, index, onChange }: { clip: Clip; index: number; onChange: (patch: Partial<Clip>) => void }) {
  const moving = clip.motion.kind === "ken_burns";
  const direction = clip.motion.to[2] >= clip.motion.from[2] ? "in" : "out";
  return (
    <>
      <h2 className="slideshow-editor__panel-title">Clip {index + 1}</h2>
      <label className="slideshow-editor__field">
        <span>Duration · {seconds(clip.duration_ms)}</span>
        <Slider
          aria-label="Clip duration"
          min={MIN_CLIP_MS}
          max={MAX_CLIP_MS}
          step={100}
          value={clip.duration_ms}
          label={seconds}
          onChange={(v) => onChange({ duration_ms: v })}
        />
      </label>
      {clip.video && <VideoSection clip={clip} onChange={onChange} />}
      <Select
        label="Frame"
        value={clip.frame}
        onChange={(v) => v !== null && onChange({ frame: v as FrameKind })}
        // A video plays alone: filling the frame, or whole on its blurred self.
        data={clip.video ? FRAMES.filter((f) => FRAME_CAPACITY[f.value] === 1) : FRAMES}
        size="xs"
      />
      {FRAME_CAPACITY[clip.frame] > 1 && clip.extra.length === 0 && (
        <span className="slideshow-editor__note">
          This frame holds {FRAME_CAPACITY[clip.frame]} photos — Shuffle groups them for you.
        </span>
      )}
      <Select
        label="Transition"
        value={clip.transition.kind}
        onChange={(v) => v !== null && onChange({ transition: { ...clip.transition, kind: v as TransitionKind, auto: false } })}
        data={TRANSITIONS}
        size="xs"
      />
      {clip.transition.kind !== "none" && (
        <label className="slideshow-editor__field">
          <span>Transition length · {seconds(clip.transition.duration_ms)}</span>
          <Slider
            aria-label="Transition length"
            min={100}
            max={3000}
            step={100}
            value={clip.transition.duration_ms}
            label={seconds}
            onChange={(v) => onChange({ transition: { ...clip.transition, duration_ms: v, auto: false } })}
          />
        </label>
      )}
      {!clip.video && (
        <Switch
          label="Ken Burns"
          checked={moving}
          onChange={(e) => onChange({ motion: e.currentTarget.checked ? kenBurns(index, "in") : { kind: "none", from: [0, 0, 1], to: [0, 0, 1] } })}
        />
      )}
      {moving && (
        <div className="slideshow-editor__field">
          <span>Motion</span>
          <Segmented
            label="Motion"
            value={direction}
            onChange={(d) => onChange({ motion: kenBurns(index, d as "in" | "out") })}
            data={[
              { label: "Zoom in", value: "in" },
              { label: "Zoom out", value: "out" },
            ]}
            size="xs"
          />
        </div>
      )}
    </>
  );
}

function DocumentPanel({
  doc,
  onChange,
  onShuffle,
  onSyncToMusic,
  syncing,
}: {
  doc: SlideshowDoc;
  onChange: (fn: (d: SlideshowDoc) => SlideshowDoc) => void;
  onShuffle: () => void;
  onSyncToMusic: () => void;
  syncing: boolean;
}) {
  const track = doc.audio;
  const [picking, setPicking] = useState(false);
  const [fitting, setFitting] = useState(false);
  /** The part of the song that suits the show (Round 6), applied only while the show still
   *  plays that track — the answer can arrive after the user has chosen another. */
  const fit = async (path: string) => {
    setFitting(true);
    try {
      const w = await invoke<MusicWindow>("music_window", { path, targetMs: timeline(doc).totalMs });
      if (!w) return;
      onChange((d) =>
        d.audio?.path === path ? { ...d, audio: { ...d.audio, start_ms: w.start_ms, fade_in_ms: w.fade_in_ms, fade_out_ms: w.fade_out_ms } } : d,
      );
    } catch {
      // Without FFmpeg or a readable file the track simply plays from the top.
    } finally {
      setFitting(false);
    }
  };
  /** Whatever the music came from, it joins the show the same way — at the level the user
   *  was listening at, so the show sounds like the preview did — and then moves to its best part. */
  const attach = (path: string, gainDb = 0) => {
    onChange((d) => ({ ...d, audio: { path, fade_in_ms: 1000, fade_out_ms: 1500, gain_db: Math.round(gainDb * 10) / 10, start_ms: 0 } }));
    void fit(path);
  };
  const chooseFile = async () => {
    const picked = await open({ multiple: false, directory: false, filters: [{ name: "Audio", extensions: AUDIO_EXTENSIONS }] });
    if (typeof picked !== "string") return;
    setPicking(false);
    attach(picked);
  };
  return (
    <>
      <h2 className="slideshow-editor__panel-title">Slideshow</h2>
      <Button variant="default" leftSection={<IconArrowsShuffle size={16} />} onClick={onShuffle}>
        Shuffle frames
      </Button>
      <div className="slideshow-editor__field">
        <span>Resolution</span>
        <Segmented
          label="Resolution"
          value={String(doc.resolution[0])}
          onChange={(v) => onChange((d) => ({ ...d, resolution: v === "1280" ? [1280, 720] : v === "3840" ? [3840, 2160] : [1920, 1080] }))}
          data={[
            { label: "720p", value: "1280" },
            { label: "1080p", value: "1920" },
            { label: "4K", value: "3840" },
          ]}
          size="xs"
        />
      </div>
      <div className="slideshow-editor__field">
        <span>Frames per second</span>
        <Segmented
          label="Frame rate"
          value={String(doc.fps)}
          onChange={(v) => onChange((d) => ({ ...d, fps: Number(v) }))}
          data={[
            { label: "24", value: "24" },
            { label: "30", value: "30" },
            { label: "60", value: "60" },
          ]}
          size="xs"
        />
      </div>
      <h3 className="slideshow-editor__panel-sub">Music</h3>
      {track ? (
        <>
          <p className="slideshow-editor__track" title={track.path}>
            {track.path.split(/[\\/]/).pop()}
          </p>
          <label className="slideshow-editor__field">
            <span>Fade in · {seconds(track.fade_in_ms)}</span>
            <Slider
              aria-label="Fade in"
              min={0}
              max={10_000}
              step={100}
              value={track.fade_in_ms}
              label={seconds}
              onChange={(v) => onChange((d) => ({ ...d, audio: { ...track, fade_in_ms: v } }))}
            />
          </label>
          <label className="slideshow-editor__field">
            <span>Fade out · {seconds(track.fade_out_ms)}</span>
            <Slider
              aria-label="Fade out"
              min={0}
              max={10_000}
              step={100}
              value={track.fade_out_ms}
              label={seconds}
              onChange={(v) => onChange((d) => ({ ...d, audio: { ...track, fade_out_ms: v } }))}
            />
          </label>
          <label className="slideshow-editor__field">
            <span>Volume · {track.gain_db > 0 ? `+${track.gain_db}` : track.gain_db} dB</span>
            <Slider
              aria-label="Volume"
              min={-30}
              max={10}
              step={1}
              value={track.gain_db}
              label={(v) => `${v} dB`}
              onChange={(v) => onChange((d) => ({ ...d, audio: { ...track, gain_db: v } }))}
            />
          </label>
          <p className="slideshow-editor__hint">Plays from {clock(track.start_ms)}</p>
          <Button variant="default" leftSection={<IconSparkles size={16} />} loading={fitting} onClick={() => void fit(track.path)}>
            Best part
          </Button>
          <Button variant="default" leftSection={<IconWaveSine size={16} />} loading={syncing} onClick={onSyncToMusic}>
            Cut to the beat
          </Button>
          <Button variant="subtle" color="red" leftSection={<IconTrash size={16} />} onClick={() => onChange((d) => ({ ...d, audio: null }))}>
            Remove music
          </Button>
        </>
      ) : (
        <Button variant="default" leftSection={<IconMusic size={16} />} onClick={() => setPicking(true)}>
          Choose music…
        </Button>
      )}
      {picking && (
        <MusicPicker
          onClose={() => setPicking(false)}
          onUse={(saved, gainDb) => {
            setPicking(false);
            attach(saved.path, gainDb);
          }}
          onChooseFile={() => void chooseFile()}
        />
      )}
    </>
  );
}

/** The right-hand panel: the selected clip, or the document when nothing is selected. */
export function Inspector({
  doc,
  selected,
  onClipChange,
  onDocChange,
  onShuffle,
  onSyncToMusic,
  syncing,
}: {
  doc: SlideshowDoc;
  selected: number | null;
  onClipChange: (index: number, patch: Partial<Clip>) => void;
  onDocChange: (fn: (d: SlideshowDoc) => SlideshowDoc) => void;
  onShuffle: () => void;
  onSyncToMusic: () => void;
  syncing: boolean;
}) {
  const clip = selected !== null ? doc.clips[selected] : undefined;
  return (
    <aside className="slideshow-editor__inspector" aria-label="Inspector">
      {clip && selected !== null ? (
        <ClipPanel clip={clip} index={selected} onChange={(patch) => onClipChange(selected, patch)} />
      ) : (
        <DocumentPanel doc={doc} onChange={onDocChange} onShuffle={onShuffle} onSyncToMusic={onSyncToMusic} syncing={syncing} />
      )}
    </aside>
  );
}

/**
 * A video clip's part of its video (Round 6): where it starts, and *Best part* to let Lumen
 * look again. The clip plays for its duration from there — past the part's end when it has
 * been lengthened, which shows more of the moment.
 */
function VideoSection({ clip, onChange }: { clip: Clip; onChange: (patch: Partial<Clip>) => void }) {
  const [finding, setFinding] = useState(false);
  const video = clip.video;
  if (!video) return null;
  const findBest = async () => {
    setFinding(true);
    try {
      const span = await invoke<VideoSpan>("video_best_part", { assetId: clip.asset_id });
      if (span) {
        onChange({ video: span, duration_ms: Math.min(MAX_CLIP_MS, Math.max(MIN_CLIP_MS, span.out_ms - span.in_ms)) });
      }
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not look at that video." });
    } finally {
      setFinding(false);
    }
  };
  return (
    <>
      <NumberInput
        label="Starts at (seconds into the video)"
        min={0}
        step={0.5}
        decimalScale={1}
        value={video.in_ms / 1000}
        onChange={(v) => {
          const start = Math.max(0, Math.round(Number(v) * 1000) || 0);
          onChange({ video: { in_ms: start, out_ms: start + clip.duration_ms } });
        }}
        size="xs"
      />
      <Button variant="default" leftSection={<IconSparkles size={16} />} loading={finding} onClick={() => void findBest()}>
        Best part
      </Button>
    </>
  );
}
