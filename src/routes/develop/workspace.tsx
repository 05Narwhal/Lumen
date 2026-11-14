import { useQueryClient } from "@tanstack/react-query";
import { IconChevronDown, IconChevronUp } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, type MouseEvent } from "react";
import { Link, useNavigate } from "react-router";
import { useContextMenu, type MenuItem } from "@/components/context-menu";
import { openExportSheet } from "@/components/export-sheet";
import { cropCancel, cropDone, DevelopPanels, openCrop, requestAuto, requestRetouch } from "@/components/develop-panels";
import { DevelopSidebar } from "@/components/develop-sidebar";
import { Filmstrip, nextSelection } from "@/components/filmstrip";
import { toast } from "@/components/toast";
import { isMac } from "@/components/window-controls";
import { useDevelopSession, type DevelopSession } from "@/hooks/use-develop-session";
import { useAsset } from "@/hooks/use-asset";
import { useDevelopStrip } from "@/hooks/use-develop-strip";
import { isOversized } from "@/lib/oversized";
import { useOpenWith } from "@/lib/external-apps";
import { DEFAULT_DEVELOP, developSelection, PANELS } from "@/lib/develop";
import { maskUi } from "@/lib/develop-masks";
import { matches } from "@/lib/keybinds";
import { developSourceStore, stripHiddenStore, toggleDevelopStrip } from "@/lib/develop-source";
import { invoke } from "@/lib/ipc";
import { applyMark, FLAG_NONE, FLAG_PICK, FLAG_REJECT, LABELS, markForKey } from "@/lib/marks";
import { queryKeys } from "@/lib/query-keys";
import type { PreviousRecipe } from "@/types/generated/PreviousRecipe";
import type { Recipe } from "@/types/generated/Recipe";
import type { StripItem } from "@/types/generated/StripItem";
import { DevelopCanvas } from "./canvas";
import { CropStage } from "./crop-stage";
import { useCopyPaste } from "./copy-paste";
import { useMcpBridge } from "./mcp-bridge";
import { TopBar } from "./top-bar";
import { useStripEdits } from "./strip-edits";

const isTyping = (t: EventTarget | null) => t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);

/**
 * Develop's own keys, only while it is open: undo and redo, culling (`P` / `X` / `U`,
 * `Alt+1`–`Alt+5`, `Alt+0`, on the selection or else the open photo), ←/→ through the
 * filmstrip, and select all / none.
 */
function useDevelopKeys(session: DevelopSession, assetId: number, items: StripItem[], open: (id: number) => void, toggleStrip: () => void) {
  const qc = useQueryClient();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isTyping(e.target) || e.defaultPrevented) return;
      const mod = isMac() ? e.metaKey : e.ctrlKey;
      // The keys come from the registry (`lib/keybinds.ts`), so the person's own choices apply.
      const act = (run: () => void) => {
        e.preventDefault();
        run();
      };
      if (matches(e, "develop.undo")) return act(() => session.undo());
      if (matches(e, "develop.redo")) return act(() => session.redo());
      if (matches(e, "develop.select-all")) return act(() => developSelection.set({ ids: new Set(items.map((i) => i.id)), anchor: assetId }));
      if (matches(e, "develop.select-none")) return act(() => developSelection.set({ ids: new Set(), anchor: null }));
      // The panels own Auto's and Retouch's tasks, so the keys ask them like the palette does.
      if (matches(e, "develop-auto")) return act(requestAuto);
      if (matches(e, "develop-retouch")) return act(requestRetouch);
      // The crop tool (Phase 8e): `R` opens it, and again or Enter applies; Esc cancels.
      const crop = session.tool === "crop";
      if (matches(e, "develop.crop") || (crop && matches(e, "develop.done-crop"))) return act(() => (crop ? cropDone(session) : openCrop(session)));
      if (crop && matches(e, "develop.cancel-crop")) return act(() => cropCancel(session));
      // A mask's column (Round 19): Esc stops painting, then leaves the mask like *Done*.
      if (!mod && !e.altKey && !e.shiftKey && e.key === "Escape" && maskUi.get().selected !== null) {
        return act(() => {
          if (maskUi.get().painting) maskUi.set((u) => ({ ...u, painting: null }));
          else {
            maskUi.set((u) => ({ ...u, selected: null }));
            session.setTool(null);
          }
        });
      }
      // F6, as in Lightroom: fold the filmstrip away for more room.
      if (matches(e, "develop-filmstrip")) return act(toggleStrip);
      const mark = markForKey(e);
      if (mark) {
        e.preventDefault();
        const sel = [...developSelection.get().ids];
        void applyMark(qc, mark.kind, sel.length > 0 ? sel : [assetId], mark.value);
        return;
      }
      const step = matches(e, "develop.next") ? 1 : matches(e, "develop.previous") ? -1 : 0;
      if (step !== 0) {
        // Not from a slider or the curve: they use the arrows themselves.
        if (e.target instanceof HTMLElement && e.target.closest('[role="slider"]')) return;
        const i = items.findIndex((it) => it.id === assetId);
        const next = items[i + step];
        if (i >= 0 && next) {
          e.preventDefault();
          open(next.id);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [session, assetId, items, open, qc, toggleStrip]);
}

/**
 * The Develop screen (spec §1): top bar, presets and history on the left, the canvas, the
 * histogram and panels on the right, the filmstrip along the bottom.
 */
export function DevelopWorkspace({ assetId }: { assetId: number }) {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const session = useDevelopSession(assetId);
  const source = developSourceStore.use();
  const strip = useDevelopStrip(source);
  const sel = developSelection.use();
  const clip = useCopyPaste(session, assetId);
  // Round 22 lane C: *Edit in* and *Open with ▸* in the canvas's menu.
  const openAsset = useAsset(assetId).data;
  const openWith = useOpenWith(openAsset ? { mediaType: openAsset.mediaType, raw: openAsset.raw } : null);
  const { open: openMenu } = useContextMenu();
  const open = useCallback(
    (id: number) => {
      if (id === assetId) return;
      void session.flush();
      navigate(`/develop/${id}`);
    },
    [assetId, navigate, session],
  );
  const stripIds = useMemo(() => strip.items.map((i) => i.id), [strip.items]);
  const edits = useStripEdits(session, assetId, stripIds);
  // The neighbours' settings are fetched ahead, so stepping to one fills the panels at once.
  useEffect(() => {
    const i = stripIds.indexOf(assetId);
    for (const id of [stripIds[i - 1], stripIds[i + 1]]) {
      if (id !== undefined) void qc.prefetchQuery({ queryKey: queryKeys.developDraft(id), queryFn: () => invoke<Recipe | null>("get_develop_draft", { assetId: id }), staleTime: Infinity });
    }
  }, [stripIds, assetId, qc]);
  // Round 22 §2: Develop works on the reduced copy of an image too large to decode whole.
  const asset = useAsset(assetId);
  const oversized = isOversized(asset.data?.width, asset.data?.height);
  const stripHidden = stripHiddenStore.use();
  const toggleStrip = toggleDevelopStrip;
  useDevelopKeys(session, assetId, strip.items, open, toggleStrip);
  useMcpBridge(session, open);
  // The selection belongs to this visit to Develop.
  useEffect(() => () => developSelection.set({ ids: new Set(), anchor: null }), []);

  const onSelect = (id: number, mods: { ctrl: boolean; shift: boolean }) => {
    const ids = strip.items.map((i) => i.id);
    developSelection.set(nextSelection(ids, sel.ids, sel.anchor ?? assetId, id, mods));
  };

  const resetDevelop = async (ids: number[]) => {
    const here = ids.includes(assetId);
    if (here) session.apply(() => DEFAULT_DEVELOP, "Reset develop");
    const others = ids.filter((i) => i !== assetId);
    const previous = others.length ? await invoke<PreviousRecipe[]>("paste_develop", { ids: others, develop: DEFAULT_DEVELOP, panels: [...PANELS.map((p) => p.value), "masks"], label: "Reset develop" }) : [];
    toast({
      message: ids.length === 1 ? "Reset develop settings" : `Reset develop settings on ${ids.length.toLocaleString()} photos`,
      undo: () => {
        if (here) session.undo();
        if (previous.length) void invoke("restore_develop_drafts", { previous });
      },
    });
  };

  const stripMenu = (e: MouseEvent, it: StripItem) => {
    const targets = sel.ids.has(it.id) ? [...sel.ids] : [it.id];
    const items: MenuItem[] = [
      {
        label: "Flag",
        children: [
          { label: "Pick", shortcut: "P", onSelect: () => void applyMark(qc, "flag", targets, FLAG_PICK) },
          { label: "Reject", shortcut: "X", onSelect: () => void applyMark(qc, "flag", targets, FLAG_REJECT) },
          { label: "Unflag", shortcut: "U", onSelect: () => void applyMark(qc, "flag", targets, FLAG_NONE) },
        ],
      },
      {
        label: "Label",
        children: [
          ...LABELS.map((l) => ({ label: l.name, shortcut: `Alt+${l.value}`, onSelect: () => void applyMark(qc, "label", targets, l.value) })),
          { label: "None", shortcut: "Alt+0", onSelect: () => void applyMark(qc, "label", targets, 0) },
        ],
      },
      { divider: true },
      { label: "Copy settings…", onSelect: clip.openCopy },
      { label: "Paste settings", onSelect: () => void clip.paste() },
      { label: "Reset develop", onSelect: () => void resetDevelop(targets) },
      { divider: true },
      { label: targets.length === 1 ? "Remove from filmstrip" : `Remove ${targets.length.toLocaleString()} from filmstrip`, onSelect: () => void edits.remove(targets) },
    ];
    openMenu(e, items);
  };

  const current = strip.items.find((i) => i.id === assetId) ?? null;
  return (
    <div className="develop" data-strip={stripHidden ? "hidden" : "shown"}>
      <TopBar session={session} assetId={assetId} clip={clip} />
      <aside className="develop__left" aria-label="Presets and history">
        <DevelopSidebar session={session} assetId={assetId} />
      </aside>
      <main className="develop__stage">
        {session.tool === "crop" ? (
          <CropStage session={session} assetId={assetId} />
        ) : (
        <DevelopCanvas
          session={session}
          assetId={assetId}
          marks={current}
          order={stripIds}
          actions={{
            copySettings: clip.openCopy,
            pasteSettings: () => void clip.paste(),
            reset: () => void resetDevelop([assetId]),
            flag: (v) => void applyMark(qc, "flag", [assetId], v === 1 ? FLAG_PICK : v === -1 ? FLAG_REJECT : FLAG_NONE),
            label: (v) => void applyMark(qc, "label", [assetId], v),
            export: () => openExportSheet([assetId]),
            openWith: { editIn: (appId) => openWith.editIn([assetId], appId), chooseApp: () => openWith.chooseApp([assetId]) },
          }}
          openWith={openWith.context}
        />
        )}
        {session.preview?.assetId === assetId && session.preview.cameraRendering && (
          <p className="develop__note" role="note" title="Lumen can't read this RAW file's sensor data, so it shows the camera's JPEG. Adobe DNG Converter lets it develop the RAW.">
            Camera rendering, not RAW ·{" "}
            <Link className="develop__note-link" to="/settings?section=raw-formats">
              Why?
            </Link>
          </p>
        )}
        {oversized && (
          <p className="develop__note" role="note">
            Editing a reduced copy of a very large image
          </p>
        )}
        <button
          type="button"
          className="develop__strip-toggle"
          aria-label={stripHidden ? "Show filmstrip" : "Hide filmstrip"}
          aria-expanded={!stripHidden}
          title={`${stripHidden ? "Show" : "Hide"} filmstrip (F6)`}
          onClick={toggleStrip}
        >
          {stripHidden ? <IconChevronUp size={14} /> : <IconChevronDown size={14} />}
        </button>
      </main>
      <aside className="develop__right" aria-label="Adjustments">
        <DevelopPanels session={session} />
      </aside>
      <footer className="develop__strip" aria-hidden={stripHidden} inert={stripHidden}>
        <Filmstrip items={strip.items} current={assetId} selected={sel.ids} hasMore={strip.hasMore} onLoadMore={strip.loadMore} onOpen={open} onSelect={onSelect} onContextMenu={stripMenu} />
      </footer>
      {clip.dialog}
    </div>
  );
}
