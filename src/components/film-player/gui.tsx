import { Button } from "@mantine/core";
import { IconEdit, IconMaximize, IconMinimize, IconMovie, IconX } from "@tabler/icons-react";
import { listen } from "@tauri-apps/api/event";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useState } from "react";
import { useNavigate } from "react-router";
import { ControlButton, MediaControls } from "@/components/media-controls";
import { SlideshowPreview } from "@/components/slideshow-preview";
import { Spinner } from "@/components/spinner";
import { toast } from "@/components/toast";
import { closeFilm, exportMemoryFilm, filmStore, type FilmRequest } from "@/lib/film";
import { useIdle } from "@/hooks/use-idle";
import { setWindowFullscreen } from "@/lib/fullscreen";
import { invoke, isAppError } from "@/lib/ipc";
import { timeline } from "@/lib/slideshow-timing";
import type { FilmProgress } from "@/types/generated/FilmProgress";
import type { MemoryFilm } from "@/types/generated/MemoryFilm";
import { mediaUrl } from "@/utils/media";
import { WindowChrome } from "@/components/window-chrome";
import { useWindowDrag } from "@/hooks/use-window-drag";
import { matches } from "@/lib/keybinds";
import "./styles.scss";

/** What the player says while the film is made, by `film://progress` stage. */
const STAGE_TEXT: Record<string, string> = {
  music: "Choosing music…",
  beat: "Finding the beat…",
  ready: "Starting…",
};

/**
 * A memory played as a film (Round 4F, Task 5): music, transitions and cuts on the beat,
 * made by `memory_film` the first time and kept. The cover shows while it is made. Its bar is
 * the viewer's video controls (Round 6): Space pauses, ←/→ skip 5 s, M mutes, F is full
 * screen, Esc closes. *Edit slideshow* keeps the film as a slideshow to change; *Export
 * video* saves it as an MP4 without one.
 */
export function FilmPlayer({ request, onClose }: { request: FilmRequest; onClose: () => void }) {
  const navigate = useNavigate();
  const drag = useWindowDrag();
  const [film, setFilm] = useState<MemoryFilm | null>(null);
  const [stage, setStage] = useState<string | null>(null);
  const [timeMs, setTimeMs] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [saving, setSaving] = useState(false);
  const [muted, setMuted] = useState(false);
  const [fullscreen, setFullscreen] = useState(false);
  // The bar, the title row and the cursor step aside while the film plays untouched (Round 8B).
  const [idle] = useIdle(!!film);
  const { memoryId } = request;
  const total = film ? timeline(film.doc).totalMs : 0;
  const toggleFullscreen = useCallback(() => {
    setFullscreen((on) => {
      void setWindowFullscreen(!on);
      return !on;
    });
  }, []);
  const close = useCallback(() => {
    if (fullscreen) void setWindowFullscreen(false);
    onClose();
  }, [fullscreen, onClose]);

  useEffect(() => {
    let live = true;
    const unlisten = listen<FilmProgress>("film://progress", (e) => {
      if (live && e.payload.memoryId === memoryId) setStage(e.payload.stage);
    });
    invoke<MemoryFilm>("memory_film", { memoryId })
      .then((f) => {
        if (!live) return;
        if (f.doc.clips.length === 0) {
          toast({ message: "This memory has nothing to play." });
          onClose();
          return;
        }
        setFilm(f);
        setPlaying(true);
      })
      .catch((e: unknown) => {
        if (!live) return;
        toast({ message: isAppError(e) ? e.message : "The film could not be made." });
        onClose();
      });
    return () => {
      live = false;
      void unlisten.then((off) => off());
    };
  }, [memoryId, onClose]);

  const edit = useCallback(async () => {
    setSaving(true);
    try {
      const id = await invoke<number>("save_memory_film", { memoryId });
      onClose();
      navigate(`/slideshows/${id}`);
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "The slideshow could not be saved." });
    } finally {
      setSaving(false);
    }
  }, [memoryId, navigate, onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const handled = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (matches(e, "film.close")) {
        handled();
        // Full screen is left first; a second Escape closes the film (Round 8B).
        if (fullscreen) toggleFullscreen();
        else close();
      } else if (!film) {
        return;
      } else if (matches(e, "film.play-pause")) {
        handled();
        setPlaying((p) => !p);
      } else if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        handled();
        const step = e.key === "ArrowRight" ? 5000 : -5000;
        setTimeMs((t) => Math.max(0, Math.min(total, t + step)));
      } else if (matches(e, "film.mute")) {
        handled();
        setMuted((m) => !m);
      } else if (matches(e, "film.fullscreen")) {
        handled();
        toggleFullscreen();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [film, close, total, toggleFullscreen, fullscreen]);
  return (
    <motion.div
      className="film-player"
      role="dialog"
      aria-modal="true"
      aria-label={request.title}
      data-idle={idle}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.22, ease: [0.2, 0.8, 0.2, 1] }}
    >
      <WindowChrome autoHide />
      <div className="film-player__top" {...drag}>
        <span className="film-player__title">{request.title}</span>
        {film?.bpm != null && <span className="film-player__bpm">Cut to {film.bpm} BPM</span>}
        <span className="film-player__spacer" />
        <Button variant="default" size="xs" leftSection={<IconMovie size={14} />} disabled={!film} onClick={() => void exportMemoryFilm(memoryId)}>
          Export video
        </Button>
        <Button variant="default" size="xs" leftSection={<IconEdit size={14} />} loading={saving} disabled={!film} onClick={() => void edit()}>
          Edit slideshow
        </Button>
        <button type="button" className="film-player__icon" aria-label="Close" onClick={close}>
          <IconX size={18} />
        </button>
      </div>
      <div className="film-player__stage">
        {film ? (
          <div className="film-player__frame">
            <SlideshowPreview doc={film.doc} slideshowId={0} timeMs={timeMs} playing={playing} muted={muted} onTime={setTimeMs} onEnded={() => setPlaying(false)} />
          </div>
        ) : (
          <div className="film-player__making">
            {request.coverAssetId != null && <img className="film-player__cover" src={mediaUrl("preview", request.coverAssetId)} alt="" />}
            <div className="film-player__status" role="status">
              <Spinner label="Making the film" />
              {STAGE_TEXT[stage ?? ""] ?? "Putting the film together…"}
            </div>
          </div>
        )}
      </div>
      <div className="film-player__bottom">
        <MediaControls
          playing={playing}
          onToggle={() => setPlaying((p) => !p)}
          timeMs={timeMs}
          durationMs={total}
          onSeek={setTimeMs}
          muted={muted}
          onToggleMute={() => setMuted((m) => !m)}
          disabled={!film}
          hidden={idle}
        >
          <ControlButton label={fullscreen ? "Exit full screen" : "Full screen"} tip={fullscreen ? "Exit full screen (F)" : "Full screen (F)"} onClick={toggleFullscreen} disabled={!film}>
            {fullscreen ? <IconMinimize size={18} /> : <IconMaximize size={18} />}
          </ControlButton>
        </MediaControls>
      </div>
    </motion.div>
  );
}

/** Mounted once in the shell: plays whatever `playFilm` asked for. */
export function FilmPlayerHost() {
  const request = filmStore.use();
  return <AnimatePresence>{request && <FilmPlayer key={request.memoryId} request={request} onClose={closeFilm} />}</AnimatePresence>;
}
