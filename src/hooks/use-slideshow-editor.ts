import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { Clip } from "@/types/generated/Clip";
import type { SlideshowDetail } from "@/types/generated/SlideshowDetail";
import type { LayoutStyle } from "@/types/generated/LayoutStyle";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import { saveSlideshow, useSlideshow } from "./use-slideshows";

/** Long enough that typing a name is one save, short enough that nothing is ever lost. */
const AUTOSAVE_MS = 600;
/** Trimming lands on tenths of a second, so durations stay readable. */
export const TRIM_STEP_MS = 100;
export const MIN_CLIP_MS = 500;
export const MAX_CLIP_MS = 30_000;

/**
 * The editor's document: loaded once, edited in memory, autosaved after a pause. Every
 * change goes through `update`, so there is one place that marks the document dirty.
 */
export function useSlideshowEditor(id: number) {
  const query = useSlideshow(id);
  const qc = useQueryClient();
  const [doc, setDoc] = useState<SlideshowDoc | null>(null);
  const [name, setName] = useState("");
  const [selected, setSelected] = useState<number | null>(null);
  const [syncing, setSyncing] = useState(false);
  const dirty = useRef(false);
  const latest = useRef<{ name: string; doc: SlideshowDoc } | null>(null);

  const loaded = query.data;
  useEffect(() => {
    if (!loaded) return;
    setDoc(loaded.doc);
    setName(loaded.name);
    dirty.current = false;
  }, [loaded]);

  latest.current = doc ? { name, doc } : null;

  // One save per pause, and the list's rows follow the new length.
  useEffect(() => {
    if (!dirty.current || !doc) return;
    const timer = setTimeout(() => {
      const state = latest.current;
      if (!state) return;
      dirty.current = false;
      saveSlideshow(id, state.name, state.doc)
        .then(() => qc.invalidateQueries({ queryKey: queryKeys.slideshows }))
        .catch((e: unknown) => toast({ message: isAppError(e) ? e.message : "Could not save the slideshow." }));
    }, AUTOSAVE_MS);
    return () => clearTimeout(timer);
  }, [doc, name, id, qc]);

  const update = useCallback((fn: (doc: SlideshowDoc) => SlideshowDoc) => {
    dirty.current = true;
    setDoc((d) => (d ? fn(d) : d));
  }, []);

  const rename = useCallback((value: string) => {
    dirty.current = true;
    setName(value);
  }, []);

  const updateClip = useCallback(
    (index: number, patch: Partial<Clip>) =>
      update((d) => ({ ...d, clips: d.clips.map((c, i) => (i === index ? { ...c, ...patch } : c)) })),
    [update],
  );

  /** `deltaMs` is the drag, snapped to `TRIM_STEP_MS`. */
  const trimClip = useCallback(
    (index: number, deltaMs: number) =>
      update((d) => ({
        ...d,
        clips: d.clips.map((c, i) => {
          if (i !== index) return c;
          const wanted = c.duration_ms + Math.round(deltaMs / TRIM_STEP_MS) * TRIM_STEP_MS;
          return { ...c, duration_ms: Math.min(MAX_CLIP_MS, Math.max(MIN_CLIP_MS, wanted)) };
        }),
      })),
    [update],
  );

  const moveClip = useCallback(
    (from: number, to: number) => {
      update((d) => {
        const clips = [...d.clips];
        const [moved] = clips.splice(from, 1);
        clips.splice(to, 0, moved);
        return { ...d, clips };
      });
      setSelected((s) => (s === from ? to : s));
    },
    [update],
  );

  const removeClip = useCallback(
    (index: number) => {
      update((d) => ({ ...d, clips: d.clips.filter((_, i) => i !== index) }));
      setSelected((s) => {
        if (s === null) return s;
        if (s === index) return index === 0 ? null : index - 1;
        return s > index ? s - 1 : s;
      });
    },
    [update],
  );

  /** Photos and videos added from the picker (Round 6), framed by the backend, at the end. */
  const addClips = useCallback((clips: Clip[]) => update((d) => ({ ...d, clips: [...d.clips, ...clips] })), [update]);

  /** Lay the same photos out again: the editor's *Shuffle*. */
  const rebuild = useCallback(
    async (style: LayoutStyle) => {
      try {
        const rebuilt = await invoke<SlideshowDetail>("rebuild_slideshow", { id, style });
        dirty.current = false;
        setDoc(rebuilt.doc);
        setName(rebuilt.name);
        setSelected(null);
        qc.invalidateQueries({ queryKey: queryKeys.slideshow(id) });
      } catch (e) {
        toast({ message: isAppError(e) ? e.message : "Could not lay the slideshow out again." });
      }
    },
    [id, qc],
  );

  /** Re-cut the show so its cuts land on the beat of its own music. */
  const syncToMusic = useCallback(async () => {
    setSyncing(true);
    try {
      const synced = await invoke<SlideshowDetail>("sync_slideshow_to_music", { id });
      setSelected(null);
      qc.invalidateQueries({ queryKey: queryKeys.slideshow(id) });
      toast({ message: synced.bpm ? `Cut to ${synced.bpm} BPM` : "Cut to the beat" });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not find the beat of that track." });
    } finally {
      setSyncing(false);
    }
  }, [id, qc]);

  return {
    doc,
    name,
    selected,
    rebuild,
    syncToMusic,
    syncing,
    isLoading: query.isLoading,
    error: query.error,
    select: setSelected,
    rename,
    update,
    updateClip,
    trimClip,
    moveClip,
    removeClip,
    addClips,
  };
}
