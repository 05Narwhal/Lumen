import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "@/components/toast";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { Recipe } from "@/types/generated/Recipe";
import { mediaUrl } from "@/utils/media";

/** Mirrors `Recipe::identity()`: no crop, no turn, no flip, no develop, no trim. */
export const IDENTITY: Recipe = {
  version: 2,
  crop: null,
  rotate: 0,
  straighten: 0,
  flip_h: false,
  flip_v: false,
  develop: null,
  trim: null,
};

/** The stored recipe (null for none). Shared with the viewer, which plays a video's trim. */
export function useSavedRecipe(assetId: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.edit(assetId),
    queryFn: () => invoke<Recipe | null>("get_edit", { assetId }),
    staleTime: Infinity,
    enabled,
  });
}

/** Slider changes arrive every frame; render once per pause. */
const PREVIEW_DEBOUNCE_MS = 120;

export interface EditSession {
  /** What the controls show. */
  draft: Recipe;
  /** What is stored; null when the photo has no edit (or before `loaded`). */
  saved: Recipe | null;
  loaded: boolean;
  /** The latest rendered preview; null until the first render. */
  previewUrl: string | null;
  rendering: boolean;
  dirty: boolean;
  /**
   * Change the draft. Changes with the same `merge` key less than a second apart are one
   * undo step, so a slider drag undoes as a whole.
   */
  set: (patch: Partial<Recipe> | ((d: Recipe) => Recipe), merge?: string) => void;
  /** One control back to neutral (double-click a slider). */
  reset: (key: keyof Recipe) => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  /** Save, with an "Edit saved" toast that can undo it. */
  done: () => Promise<void>;
  /** Throw the draft away. */
  cancel: () => void;
  /** Delete the stored edit, with a "Reverted to original" toast that can undo it. */
  revert: () => Promise<void>;
}

const same = (a: Recipe, b: Recipe) => JSON.stringify(a) === JSON.stringify(b);

/**
 * One photo being edited. Previews are rendered by Rust from the unedited photo on every
 * pause in slider movement; nothing is stored until `done`. While the crop tool is open the
 * preview shows the whole frame, so the crop rectangle has something to frame.
 */
/** The recipe with its geometry taken out: what Rust renders when the editor draws geometry itself. */
export const withoutGeometry = (r: Recipe): Recipe => ({
  ...r,
  crop: null,
  rotate: 0,
  straighten: 0,
  flip_h: false,
  flip_v: false,
  trim: null,
});

/** Merged changes further apart than this are separate undo steps. */
const MERGE_MS = 1000;

export function useEditSession(
  assetId: number,
  opts: {
    cropping?: boolean;
    /** False for a video, which has no still preview. */
    preview?: boolean;
    /** The editor draws geometry itself; Rust renders only the rest, so a turn needs no render. */
    clientGeometry?: boolean;
  } = {},
): EditSession {
  const qc = useQueryClient();
  const cropping = !!opts.cropping;
  const withPreview = opts.preview ?? true;
  const clientGeometry = !!opts.clientGeometry;
  const q = useSavedRecipe(assetId);
  const saved = q.data ?? null;
  const loaded = q.isSuccess;
  const [draft, setDraft] = useState<Recipe>(IDENTITY);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);
  const [rendering, setRendering] = useState(false);
  const request = useRef(0);
  const [history, setHistory] = useState<{ past: Recipe[]; future: Recipe[] }>({
    past: [],
    future: [],
  });
  const lastMerge = useRef<{ key: string; at: number } | null>(null);

  // The draft starts from what is stored, once that is known.
  useEffect(() => {
    if (loaded) {
      setDraft(saved ?? IDENTITY);
      setHistory({ past: [], future: [] });
    }
  }, [loaded, saved]);

  const previewRecipe = clientGeometry ? withoutGeometry(draft) : cropping ? { ...draft, crop: null } : draft;
  // Keyed by value: with client geometry, a turn leaves the rendered recipe unchanged.
  const previewKey = JSON.stringify(previewRecipe);

  // biome-ignore lint/correctness/useExhaustiveDependencies: previewKey carries the recipe
  useEffect(() => {
    if (!loaded || !withPreview) return;
    const recipe = JSON.parse(previewKey) as Recipe;
    const n = ++request.current;
    const t = setTimeout(() => {
      setRendering(true);
      invoke<string>("render_edit_preview", { assetId, recipe })
        .then((token) => {
          // A slow render must not replace a newer one.
          if (n === request.current) setPreviewUrl(mediaUrl("edit-preview", token));
        })
        .catch(() => {})
        .finally(() => n === request.current && setRendering(false));
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(t);
  }, [assetId, previewKey, loaded, withPreview]);

  // The draft and its history move together; `draftRef` lets the callbacks stay stable.
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const commit = useCallback((next: Recipe, merge?: string) => {
    const prev = draftRef.current;
    if (same(prev, next)) return;
    const now = Date.now();
    const merging = !!merge && lastMerge.current?.key === merge && now - lastMerge.current.at < MERGE_MS;
    lastMerge.current = merge ? { key: merge, at: now } : null;
    if (!merging) setHistory((h) => ({ past: [...h.past, prev].slice(-100), future: [] }));
    draftRef.current = next;
    setDraft(next);
  }, []);
  const set = useCallback(
    (patch: Partial<Recipe> | ((d: Recipe) => Recipe), merge?: string) =>
      commit(typeof patch === "function" ? patch(draftRef.current) : { ...draftRef.current, ...patch }, merge),
    [commit],
  );
  const reset = useCallback((key: keyof Recipe) => commit({ ...draftRef.current, [key]: IDENTITY[key] }), [commit]);
  const undo = useCallback(() => {
    setHistory((h) => {
      const prev = h.past.at(-1);
      if (!prev) return h;
      const current = draftRef.current;
      draftRef.current = prev;
      setDraft(prev);
      lastMerge.current = null;
      return { past: h.past.slice(0, -1), future: [current, ...h.future] };
    });
  }, []);
  const redo = useCallback(() => {
    setHistory((h) => {
      const next = h.future[0];
      if (!next) return h;
      const current = draftRef.current;
      draftRef.current = next;
      setDraft(next);
      lastMerge.current = null;
      return { past: [...h.past, current], future: h.future.slice(1) };
    });
  }, []);
  const cancel = useCallback(() => {
    setDraft(saved ?? IDENTITY);
    setHistory({ past: [], future: [] });
  }, [saved]);

  const refresh = useCallback(() => qc.invalidateQueries({ queryKey: queryKeys.edit(assetId) }), [qc, assetId]);
  const store = useCallback(
    async (recipe: Recipe | null) => {
      if (recipe) await invoke<number>("save_edit", { assetId, recipe });
      else await invoke("revert_edit", { assetId });
      await refresh();
    },
    [assetId, refresh],
  );

  const done = useCallback(async () => {
    const before = saved;
    try {
      await store(draft);
      toast({ message: "Edit saved", undo: () => void store(before) });
    } catch (e) {
      toast({
        message: isAppError(e) ? e.message : "The edit could not be saved.",
      });
    }
  }, [draft, saved, store]);

  const revert = useCallback(async () => {
    try {
      const removed = await invoke<Recipe | null>("revert_edit", { assetId });
      await refresh();
      if (removed)
        toast({
          message: "Reverted to original",
          undo: () => void store(removed),
        });
    } catch (e) {
      toast({
        message: isAppError(e) ? e.message : "The edit could not be reverted.",
      });
    }
  }, [assetId, refresh, store]);

  return {
    draft,
    saved,
    loaded,
    previewUrl,
    rendering,
    dirty: loaded && !same(draft, saved ?? IDENTITY),
    set,
    reset,
    undo,
    redo,
    canUndo: history.past.length > 0,
    canRedo: history.future.length > 0,
    done,
    cancel,
    revert,
  };
}
