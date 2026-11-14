import { ActionIcon, Button, TextInput } from "@mantine/core";
import { IconArrowLeft, IconDownload, IconPlayerPauseFilled, IconPlayerPlayFilled, IconPlus, IconSlideshow } from "@tabler/icons-react";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router";
import { EmptyState } from "@/components/empty-state";
import { PhotoPicker } from "@/components/photo-picker";
import { SlideshowPreview } from "@/components/slideshow-preview";
import { Spinner } from "@/components/spinner";
import { toast } from "@/components/toast";
import { useFfmpegStatus } from "@/hooks/use-media-status";
import { useSlideshowEditor } from "@/hooks/use-slideshow-editor";
import { exportSlideshow } from "@/lib/library-actions";
import { slideshowPlayingStore } from "@/lib/presence";
import { invoke, isAppError } from "@/lib/ipc";
import type { Clip } from "@/types/generated/Clip";
import { timeline } from "@/lib/slideshow-timing";
import { matches } from "@/lib/keybinds";
import { formatDuration } from "@/utils/date";
import { ClipStrip } from "./clip-strip";
import { Inspector } from "./inspector";

/** True when a keystroke belongs to whatever the person is typing in. */
const typing = (target: EventTarget | null) => {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
};

/** `/slideshows/:id` (UX spec Â§6.6): preview and transport, the clips, and the inspector. */
export function SlideshowEditor({ id }: { id: number }) {
  const { doc, name, selected, isLoading, error, select, rename, update, updateClip, trimClip, moveClip, removeClip, addClips, rebuild, syncToMusic, syncing } =
    useSlideshowEditor(id);
  const [adding, setAdding] = useState(false);
  /** Photos and videos picked are framed by the backend â a video at its best part â and
   *  join the end of the show (Round 6). */
  const addPicked = async (assetIds: number[]) => {
    setAdding(false);
    if (assetIds.length === 0) return;
    try {
      addClips(await invoke<Clip[]>("slideshow_clips", { assetIds }));
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not add those to the slideshow." });
    }
  };
  const { status } = useFfmpegStatus();
  const [timeMs, setTimeMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  // The Discord status says "Watching a slideshow" while one plays (Round 17).
  useEffect(() => {
    slideshowPlayingStore.set(playing);
    return () => slideshowPlayingStore.set(false);
  }, [playing]);
  const total = doc ? timeline(doc).totalMs : 0;
  const starts = doc ? timeline(doc).starts : [];
  const canExport = status?.available !== false && (doc?.clips.length ?? 0) > 0;

  const seek = useCallback((ms: number) => {
    setPlaying(false);
    setTimeMs(ms);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (typing(e.target)) return;
      if (matches(e, "slideshow.play-pause")) {
        e.preventDefault();
        setPlaying((p) => !p);
      }
      if (matches(e, "slideshow.remove-clip") && selected !== null) {
        e.preventDefault();
        removeClip(selected);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selected, removeClip]);

  const exportNow = () => exportSlideshow(id);

  if (isLoading) {
    return (
      <div className="slideshows-route__loading">
        <Spinner size="md" label="Loading slideshow" />
      </div>
    );
  }
  if (error || !doc) {
    return (
      <div className="slideshows-route">
        <EmptyState icon={<IconSlideshow />} title="This slideshow is gone" description="It may have been deleted." action={<Button component={Link} to="/memories?tab=slideshows">Back to slideshows</Button>} />
      </div>
    );
  }

  return (
    <div className="slideshow-editor">
      <header className="slideshow-editor__bar">
        <ActionIcon component={Link} to="/memories?tab=slideshows" variant="subtle" color="gray" aria-label="Back to slideshows">
          <IconArrowLeft size={18} />
        </ActionIcon>
        <TextInput aria-label="Slideshow name" value={name} onChange={(e) => rename(e.currentTarget.value)} className="slideshow-editor__name" />
        <div className="slideshow-editor__bar-end">
          {status?.available === false && <span className="slideshow-editor__hint">Exporting needs FFmpeg â set it up in Settings.</span>}
          <Button variant="default" leftSection={<IconPlus size={16} />} onClick={() => setAdding(true)}>
            Add photos or videos…
          </Button>
          <Button leftSection={<IconDownload size={16} />} disabled={!canExport} onClick={() => void exportNow()}>
            Export
          </Button>
        </div>
      </header>

      <div className="slideshow-editor__body">
        <div className="slideshow-editor__stage">
          <div className="slideshow-editor__viewport">
            <SlideshowPreview doc={doc} slideshowId={id} timeMs={timeMs} playing={playing} onTime={setTimeMs} onEnded={() => setPlaying(false)} />
          </div>
          <div className="slideshow-editor__transport">
            <ActionIcon variant="filled" aria-label={playing ? "Pause" : "Play"} onClick={() => setPlaying((p) => !p)}>
              {playing ? <IconPlayerPauseFilled size={16} /> : <IconPlayerPlayFilled size={16} />}
            </ActionIcon>
            <span className="slideshow-editor__clock">
              {formatDuration(timeMs)} / {formatDuration(total)}
            </span>
            <input
              type="range"
              aria-label="Playback position"
              className="slideshow-editor__scrub"
              min={0}
              max={total}
              step={50}
              value={Math.round(timeMs)}
              onChange={(e) => seek(Number(e.currentTarget.value))}
            />
          </div>
          <div className="slideshow-editor__strip" onClick={(e) => e.target === e.currentTarget && select(null)}>
            <ClipStrip doc={doc} selected={selected} onSelect={select} onMove={moveClip} onTrim={trimClip} starts={starts} onSeek={seek} />
          </div>
        </div>
        <Inspector
          doc={doc}
          selected={selected}
          onClipChange={updateClip}
          onDocChange={update}
          onShuffle={() => void rebuild("random")}
          onSyncToMusic={() => void syncToMusic()}
          syncing={syncing}
        />
      </div>
      {adding && (
        <PhotoPicker title="Add photos or videos" confirmLabel="Add" onClose={() => setAdding(false)} onPick={(ids) => void addPicked(ids)} />
      )}
    </div>
  );
}
