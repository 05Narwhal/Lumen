import { useQueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState } from "react";
import { toast } from "@/components/toast";
import { useQuery } from "@tanstack/react-query";
import { IDENTITY } from "@/hooks/use-edit-session";
import { developOf, hasGeometry, sameRecipe, withDevelop, withoutGeometry } from "@/lib/develop";
import { FLUSH_REQUESTED } from "@/lib/events";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { AutoResult } from "@/types/generated/AutoResult";
import type { RetouchResult } from "@/types/generated/RetouchResult";
import type { Develop } from "@/types/generated/Develop";
import type { DevelopHistogram } from "@/types/generated/DevelopHistogram";
import type { DevelopPreview } from "@/types/generated/DevelopPreview";
import type { Recipe } from "@/types/generated/Recipe";
import { mediaUrl } from "@/utils/media";

/**
 * What Develop opens a photo with (Round 6): its draft, or else the library's edit. Develop
 * saves only drafts, so nothing set here changes the photo in the library until it is
 * exported.
 */
export function useDevelopDraft(assetId: number, enabled = true) {
  return useQuery({
    queryKey: queryKeys.developDraft(assetId),
    queryFn: () => invoke<Recipe | null>("get_develop_draft", { assetId }),
    staleTime: Infinity,
    enabled,
  });
}

/**
 * Saves wait for a pause, so a burst of changes is one write (and one version). Every change
 * is saved — a slider still held included — not only what a release records (Round 5).
 */
export const SAVE_DEBOUNCE_MS = 600;

/** Tells Rust a save is waiting, so closing the window writes it first (`hold_close`). */
function holdClose(hold: boolean) {
  invoke("hold_close", { hold }).catch(() => {});
}

export const historyKey = (assetId: number) => ["develop-history", assetId] as const;

/**
 * Requests are numbered across the whole app, so a reply for a photo left behind is old. The
 * count starts at the page's load time in ms: Rust keeps each photo's newest number for the
 * life of the process, and a reloaded page (Round 15's recovery) counting from 1 again would
 * have every preview of a photo it had shown called stale.
 */
let requestCounter = Date.now();

interface PreviewRequest {
  assetId: number;
  recipe: Recipe;
  before: boolean;
  clipping: boolean;
  /** Rendered without its geometry, for the crop tool to lay out itself. */
  uncropped: boolean;
}

export interface Rendered {
  url: string;
  width: number;
  height: number;
}

/** A preview and the photo it shows: the last photo stays on screen while the next renders. */
export interface Preview extends Rendered {
  assetId: number;
  /** Rendered without crop, turns, flips or straighten (the crop tool is open). */
  uncropped?: boolean;
  /** Rendered from the camera's JPEG: a RAW rawler refuses, without Adobe DNG Converter. */
  cameraRendering?: boolean;
}

/** A tool that takes over the canvas (Phase 8e). */
export type DevelopTool = "crop" | "masks";

/**
 * Before and after (Round 4J): off, the whole photo switched (`\`), a divider, or two panes.
 * In split and side by side the main render stays the after and `beforeView` is the before.
 */
export type CompareMode = "off" | "toggle" | "split" | "side";

/** The photo with its develop settings off and its geometry kept (Lightroom's before). */
export const beforeRecipe = (r: Recipe): Recipe => ({ ...r, develop: null });

/** Decode before showing, so a new photo slides in whole instead of painting in. */
export async function decoded(url: string): Promise<void> {
  if (typeof Image === "undefined") return;
  const img = new Image();
  img.src = url;
  if (typeof img.decode !== "function") return;
  await img.decode().catch(() => {});
}

export interface DevelopSession {
  /** The recipe the controls show. */
  draft: Recipe;
  develop: Develop;
  loaded: boolean;
  /** The newest render, of this photo or — until this one's first arrives — the last one. */
  preview: Preview | null;
  histogram: DevelopHistogram | null;
  rendering: boolean;
  compare: CompareMode;
  setCompare: (mode: CompareMode) => void;
  /** The whole photo shows its before (`compare === "toggle"`). */
  before: boolean;
  /** `setCompare("toggle")` or `setCompare("off")`. */
  setBefore: (on: boolean) => void;
  /** The before for split and side by side: rendered once per photo and geometry. */
  beforeView: Rendered | null;
  clipping: boolean;
  setClipping: (on: boolean) => void;
  /** Change the develop settings without recording it (a slider mid-drag). */
  update: (change: Partial<Develop> | ((d: Develop) => Develop)) => void;
  /** Record what `update` changed: an undo step, a history step named `label`, a save. */
  commit: (label: string) => void;
  /** `update` and `commit` at once (a button, a preset, a reset). */
  apply: (change: Partial<Develop> | ((d: Develop) => Develop), label: string) => void;
  undo: () => void;
  redo: () => void;
  canUndo: boolean;
  canRedo: boolean;
  /**
   * Show `recipe` as the current state (a History or snapshot click). Nothing is recorded;
   * the next change continues from here and, when `stepId` is given, drops the history steps
   * after it.
   */
  goTo: (recipe: Recipe, stepId: number | null) => void;
  /** Render this instead of the draft until cleared (a preset hovered). */
  setHover: (recipe: Recipe | null) => void;
  /** Write any pending save now. */
  flush: () => Promise<void>;
  /**
   * *Auto* (Phase 8d) on the draft on screen: its Basic values are applied, flagged `auto`,
   * as one history step — with the trained model (`source = "model"`) its curve, HSL and
   * grading too. `null` when the photo changed (or was not loaded) before it finished, or
   * `stillWanted` says it was cancelled. `task` is the `useAiTask` id for its stages.
   */
  auto: (task?: string, stillWanted?: () => boolean) => Promise<AutoResult | null>;
  /**
   * *Retouch* (Round 12, Experimental): the result's develop, crop and straighten as one
   * history step, so one Undo brings the old ones back. `null` as for `auto`.
   */
  retouch: (task?: string, stillWanted?: () => boolean) => Promise<RetouchResult | null>;
  /** The photo this session develops. */
  assetId: number;
  /** Render again with nothing changed (a denoised base was just rebuilt). */
  refresh: () => void;
  /** The tool over the canvas, if any. While Crop is open the preview has no geometry. */
  tool: DevelopTool | null;
  setTool: (tool: DevelopTool | null) => void;
  /** Change the whole recipe without recording it (a crop handle mid-drag). */
  editRecipe: (change: (r: Recipe) => Recipe) => void;
  /** `editRecipe` and `commit` at once (a turn, a flip). */
  applyRecipe: (change: (r: Recipe) => Recipe, label: string) => void;
  /** Drop what changed since the last recorded step (the crop tool's Esc). */
  discard: () => void;
  /**
   * An AI assistant's change (Round 16, `mcp-bridge`): show `recipe` and record it as one
   * step named `label`. `false` when this photo's settings are not in yet.
   */
  applyExternal: (recipe: Recipe, label: string) => boolean;
  /** Undo the newest step if an AI assistant made it: the recipe now shown, else `null`. */
  undoAi: () => Recipe | null;
}

/**
 * One photo in Develop. The draft renders on every change — one request in flight, the
 * newest waiting — and Rust drops a render that a newer request overtook. Changes are
 * recorded on commit (a slider's release), saved after a pause, and each is a history step.
 */
export function useDevelopSession(assetId: number): DevelopSession {
  const qc = useQueryClient();
  const saved = useDevelopDraft(assetId);
  const [loadedFor, setLoadedFor] = useState<number | null>(null);
  const loaded = loadedFor === assetId;
  // Read by callbacks: a change made before this photo's settings are in would be the last
  // photo's values saved onto this one.
  const loadedRef = useRef(loaded);
  loadedRef.current = loaded;
  const assetRef = useRef(assetId);
  assetRef.current = assetId;
  const [draft, setDraft] = useState<Recipe>(IDENTITY);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const [preview, setPreview] = useState<Preview | null>(null);
  const previewRef = useRef(preview);
  previewRef.current = preview;
  const [histogram, setHistogram] = useState<DevelopHistogram | null>(null);
  const [rendering, setRendering] = useState(false);
  const [compare, setCompare] = useState<CompareMode>("off");
  const before = compare === "toggle";
  const setBefore = useCallback((on: boolean) => setCompare(on ? "toggle" : "off"), []);
  const [beforeView, setBeforeView] = useState<(Rendered & { key: string }) | null>(null);
  const [clipping, setClipping] = useState(false);
  const [hover, setHover] = useState<Recipe | null>(null);
  const [tool, setTool] = useState<DevelopTool | null>(null);
  const [stacks, setStacks] = useState<{ past: Recipe[]; future: Recipe[] }>({ past: [], future: [] });
  /** The last recorded state: what undo returns to and what a commit compares against. */
  const committed = useRef<Recipe>(IDENTITY);
  /** Set after a History click: the next commit first drops the steps after it. */
  const backAt = useRef<number | null>(null);


  // Saving: debounced, flushed when the photo changes or the screen closes.
  const pendingSave = useRef<{ assetId: number; recipe: Recipe } | null>(null);
  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const holding = useRef(false);
  // Every write, in order. A flush waits for the writes before it too: one asked for while
  // an earlier save is still being written (a busy database) must not let the window go.
  const writes = useRef<Promise<void>>(Promise.resolve());
  const flush = useCallback(async () => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = null;
    const p = pendingSave.current;
    pendingSave.current = null;
    if (p) {
      writes.current = writes.current.then(async () => {
        try {
          await invoke<number>("save_develop_draft", { assetId: p.assetId, recipe: p.recipe });
        } catch (e) {
          toast({ message: isAppError(e) ? e.message : "The develop settings could not be saved." });
        }
      });
    }
    await writes.current;
    // Nothing new arrived while writing: the window may close freely again.
    if (!pendingSave.current && holding.current) {
      holding.current = false;
      holdClose(false);
    }
  }, []);
  const scheduleSave = useCallback(
    (recipe: Recipe) => {
      pendingSave.current = { assetId, recipe };
      // The cached draft follows at once: coming back to this photo (staleTime is forever)
      // must start from its edits, not from what it was when first opened.
      qc.setQueryData(queryKeys.developDraft(assetId), recipe);
      if (!holding.current) {
        holding.current = true;
        holdClose(true);
      }
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => void flush(), SAVE_DEBOUNCE_MS);
    },
    [assetId, flush, qc],
  );
  useEffect(() => () => void flush(), [assetId, flush]);
  // Closing the window: Rust holds the close while a save waits and asks for it here; once it
  // is written the window is closed again (`true`), through the import guard as usual.
  // *Restart Lumen* asks the same with `false`: write, and leave the window to Rust.
  useEffect(() => {
    const un = listen<boolean>(FLUSH_REQUESTED, (e) => {
      void flush().then(() => {
        holding.current = false;
        holdClose(false);
        if (e.payload !== false) return getCurrentWindow().close();
      });
    });
    return () => void un.then((f) => f());
  }, [flush]);

  // The draft starts from what is stored — once per photo, so a save's refetch never
  // snaps a slider back.
  // Only from fresh data: a neighbour fetched ahead and changed since (a paste, a sync) is
  // stale until its refetch lands, and its old recipe must not become the draft.
  useEffect(() => {
    if (saved.isSuccess && !saved.isStale && loadedFor !== assetId) {
      // A save still waiting belongs to the photo left behind; write it now, not later
      // under whatever this photo does.
      if (pendingSave.current && pendingSave.current.assetId !== assetId) void flush();
      const start = saved.data ?? IDENTITY;
      committed.current = start;
      draftRef.current = start;
      setDraft(start);
      setStacks({ past: [], future: [] });
      backAt.current = null;
      setTool(null);
      setLoadedFor(assetId);
    }
  }, [saved.isSuccess, saved.isStale, saved.data, assetId, loadedFor, flush]);

  // The preview loop.
  const inFlight = useRef(false);
  const waiting = useRef<PreviewRequest | null>(null);
  const applied = useRef(0);
  // Each request carries its photo: one sent after the photo changed must not borrow the
  // new id.
  const send = useCallback((req: PreviewRequest) => {
    inFlight.current = true;
    setRendering(true);
    const request = ++requestCounter;
    invoke<DevelopPreview>("develop_preview", { assetId: req.assetId, recipe: req.recipe, request, before: req.before, clipping: req.clipping })
        .then(async (p) => {
          // Overtaken in Rust, older than what is shown, or of a photo already left.
          if (p.superseded || request < applied.current || req.assetId !== assetRef.current) return;
          const url = mediaUrl("edit-preview", p.token);
          // A new photo's first render is decoded before it replaces the last photo.
          if (previewRef.current?.assetId !== req.assetId) await decoded(url);
          if (request < applied.current || req.assetId !== assetRef.current) return;
          applied.current = request;
          setPreview({ url, width: p.width, height: p.height, assetId: req.assetId, uncropped: req.uncropped || !hasGeometry(req.recipe), cameraRendering: p.cameraRendering });
          if (p.histogram) setHistogram(p.histogram);
        })
        .catch((e) => {
          if (isAppError(e)) toast({ message: e.message });
        })
        .finally(() => {
          inFlight.current = false;
          const next = waiting.current;
          waiting.current = null;
          if (next) send(next);
          else setRendering(false);
        });
  }, []);
  // The crop tool draws turns, flips, straighten and the crop itself over the whole photo.
  const uncropped = tool === "crop";
  const shown = uncropped ? withoutGeometry(hover ?? draft) : hover ?? draft;
  const [renders, setRenders] = useState(0);
  const refresh = useCallback(() => setRenders((n) => n + 1), []);
  const requestKey = JSON.stringify([assetId, shown, before, clipping, renders]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: requestKey carries what is shown
  useEffect(() => {
    if (!loaded) return;
    const req = { assetId, recipe: shown, before, clipping, uncropped };
    if (inFlight.current) waiting.current = req;
    else send(req);
  }, [requestKey, loaded]);

  // The before for split and side by side: once per photo and geometry, not per slider move.
  const beforeKey = loaded && (compare === "split" || compare === "side") ? JSON.stringify([assetId, beforeRecipe(draft)]) : null;
  const beforeKeyRef = useRef(beforeKey);
  beforeKeyRef.current = beforeKey;
  // biome-ignore lint/correctness/useExhaustiveDependencies: beforeKey carries what is asked
  useEffect(() => {
    if (!beforeKey || beforeView?.key === beforeKey) return;
    const recipe = beforeRecipe(draftRef.current);
    const key = beforeKey;
    invoke<DevelopPreview>("develop_before", { assetId, recipe })
      .then(async (p) => {
        if (beforeKeyRef.current !== key) return;
        const url = mediaUrl("edit-preview", p.token);
        await decoded(url);
        if (beforeKeyRef.current === key) setBeforeView({ url, width: p.width, height: p.height, key });
      })
      .catch((e) => {
        if (isAppError(e)) toast({ message: e.message });
      });
  }, [beforeKey]);

  const editRecipe = useCallback(
    (change: (r: Recipe) => Recipe) => {
      if (!loadedRef.current) return;
      const next = change(draftRef.current);
      draftRef.current = next;
      setDraft(next);
      scheduleSave(next);
    },
    [scheduleSave],
  );
  const update = useCallback(
    (change: Partial<Develop> | ((d: Develop) => Develop)) =>
      editRecipe((cur) => {
        const d = developOf(cur);
        return withDevelop(cur, typeof change === "function" ? change(d) : { ...d, ...change });
      }),
    [editRecipe],
  );

  const record = useCallback(
    (label: string) => {
      if (!loadedRef.current) return;
      const now = draftRef.current;
      const prev = committed.current;
      if (sameRecipe(now, prev)) return;
      committed.current = now;
      setStacks((s) => ({ past: [...s.past, prev].slice(-100), future: [] }));
      scheduleSave(now);
      const back = backAt.current;
      backAt.current = null;
      void (async () => {
        try {
          if (back !== null) await invoke("truncate_history", { assetId, stepId: back });
          await invoke("push_history", { assetId, recipe: now, label });
        } finally {
          void qc.invalidateQueries({ queryKey: historyKey(assetId) });
        }
      })();
    },
    [assetId, qc, scheduleSave],
  );

  const apply = useCallback(
    (change: Partial<Develop> | ((d: Develop) => Develop), label: string) => {
      update(change);
      record(label);
    },
    [update, record],
  );
  const applyRecipe = useCallback(
    (change: (r: Recipe) => Recipe, label: string) => {
      editRecipe(change);
      record(label);
    },
    [editRecipe, record],
  );
  const discard = useCallback(() => {
    if (!loadedRef.current) return;
    const changed = !sameRecipe(draftRef.current, committed.current);
    draftRef.current = committed.current;
    setDraft(committed.current);
    if (changed) scheduleSave(committed.current);
  }, [scheduleSave]);

  const move = useCallback(
    (to: Recipe) => {
      // Undo, redo and History act on this photo's own states, once they are in.
      if (!loadedRef.current) return;
      committed.current = to;
      draftRef.current = to;
      setDraft(to);
      scheduleSave(to);
    },
    [scheduleSave],
  );
  // Read through a ref so undo and redo have no side effects inside a state updater.
  const stacksRef = useRef(stacks);
  stacksRef.current = stacks;
  const undo = useCallback(() => {
    if (!loadedRef.current) return;
    const s = stacksRef.current;
    const prev = s.past.at(-1);
    if (!prev) return;
    setStacks({ past: s.past.slice(0, -1), future: [committed.current, ...s.future] });
    move(prev);
  }, [move]);
  const redo = useCallback(() => {
    if (!loadedRef.current) return;
    const s = stacksRef.current;
    const next = s.future[0];
    if (!next) return;
    setStacks({ past: [...s.past, committed.current], future: s.future.slice(1) });
    move(next);
  }, [move]);

  // The states AI assistants recorded: `undoAi` takes back only those.
  const aiStates = useRef(new WeakSet<Recipe>());
  const applyExternal = useCallback(
    (recipe: Recipe, label: string) => {
      if (!loadedRef.current) return false;
      applyRecipe(() => recipe, label);
      if (committed.current === recipe) aiStates.current.add(recipe);
      return true;
    },
    [applyRecipe],
  );
  const undoAi = useCallback(() => {
    if (!loadedRef.current || !aiStates.current.has(committed.current)) return null;
    undo();
    return committed.current;
  }, [undo]);

  const goTo = useCallback(
    (recipe: Recipe, stepId: number | null) => {
      if (!loadedRef.current) return;
      const cur = committed.current;
      if (!sameRecipe(cur, recipe)) setStacks((s) => ({ past: [...s.past, cur].slice(-100), future: [] }));
      move(recipe);
      backAt.current = stepId;
    },
    [move],
  );

  const auto = useCallback(
    async (task?: string, stillWanted: () => boolean = () => true): Promise<AutoResult | null> => {
      if (!loadedRef.current) return null;
      const asked = assetRef.current;
      const r = await invoke<AutoResult>("develop_auto", { assetId: asked, recipe: draftRef.current, task: task ?? null });
      // Another photo is open now: its settings are not this photo's to change.
      if (assetRef.current !== asked || !loadedRef.current || !stillWanted()) return null;
      const m = r.source === "model" ? r.look : null;
      apply((d) => ({ ...d, ...(m && { curve: m.curve, hsl: m.hsl, grading: m.grading }), basic: r.basic, auto: true }), "Auto");
      return r;
    },
    [apply],
  );
  const retouch = useCallback(
    async (task?: string, stillWanted: () => boolean = () => true): Promise<RetouchResult | null> => {
      if (!loadedRef.current) return null;
      const asked = assetRef.current;
      const r = await invoke<RetouchResult>("develop_retouch", { assetId: asked, recipe: draftRef.current, task: task ?? null });
      if (assetRef.current !== asked || !loadedRef.current || !stillWanted()) return null;
      applyRecipe((rec) => ({ ...rec, develop: r.develop, crop: r.crop, straighten: r.straighten }), "Retouch");
      return r;
    },
    [applyRecipe],
  );

  return {
    assetId,
    refresh,
    tool,
    setTool,
    editRecipe,
    applyRecipe,
    discard,
    applyExternal,
    undoAi,
    auto,
    retouch,
    draft,
    develop: developOf(draft),
    loaded,
    preview,
    histogram,
    rendering,
    compare,
    setCompare,
    before,
    setBefore,
    beforeView: beforeView && beforeView.key === beforeKey ? beforeView : null,
    clipping,
    setClipping,
    update,
    commit: record,
    apply,
    undo,
    redo,
    canUndo: stacks.past.length > 0,
    canRedo: stacks.future.length > 0,
    goTo,
    setHover,
    flush,
  };
}
