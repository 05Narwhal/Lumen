import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { PanelChoice } from "@/components/panel-choice";
import { toast } from "@/components/toast";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { aiMasksReady, retarget } from "@/lib/ai-masks";
import { type CopiedSettings, developClipboard, developSelection, mergePanels } from "@/lib/develop";
import { parseSettings, serializeSettings } from "@/lib/develop-clipboard";
import { invoke, isAppError } from "@/lib/ipc";
import { matches } from "@/lib/keybinds";
import { queryKeys } from "@/lib/query-keys";
import type { AiMaskList } from "@/types/generated/AiMaskList";
import type { AppInfo } from "@/types/generated/AppInfo";
import type { ExperimentalTask } from "@/types/generated/ExperimentalTask";
import type { Panel } from "@/types/generated/Panel";
import type { PreviousRecipe } from "@/types/generated/PreviousRecipe";
import type { Settings } from "@/types/generated/Settings";

const photos = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);

/**
 * Copy settings (choosing panels) and paste them — onto the filmstrip's selection when there
 * is one, else onto the open photo — with one Undo for all of it. `Mod+Shift+C` / `Mod+Shift+V`.
 * *Sync settings* is a copy of everything pasted onto the selection.
 */
export function useCopyPaste(session: DevelopSession, assetId: number) {
  const [choosing, setChoosing] = useState(false);
  /** Settings read from the system clipboard, waiting for the person to choose what to paste. */
  const [incoming, setIncoming] = useState<{ settings: CopiedSettings; notes: string[] } | null>(null);
  /** The text *Copy settings* last wrote to the system clipboard: pasting it back needs no question. */
  const written = useRef<string | null>(null);
  const client = useQueryClient();

  /** Pastes `clip` onto the selection, else onto the open photo, with one Undo for all of it. */
  const apply = useCallback(
    async (clip: CopiedSettings, notes: string[] = []) => {
      const selected = [...developSelection.get().ids];
      const targets = selected.length > 0 ? selected : [assetId];
      const others = targets.filter((id) => id !== assetId);
      const here = targets.includes(assetId);
      let develop = clip.develop;
      // AI masks travel as their kind: point them at this photo's detections (Round 24 §5).
      const hasAi = clip.panels.includes("masks") && !!develop.masks?.some((m) => m.shapes.some((s) => s.kind === "ai"));
      if (here && hasAi) {
        const ready = aiMasksReady(client.getQueryData<Settings>(queryKeys.settings), client.getQueryData<ExperimentalTask[]>(queryKeys.experimentalCatalog));
        if (!ready) notes = [...notes, "AI masks need the AI masks switch in Settings, so they were not detected on this photo."];
        else {
          try {
            const list = await invoke<AiMaskList>("ai_masks_detect", { assetId, again: false });
            client.setQueryData(queryKeys.aiMasks(assetId), list);
            develop = { ...develop, masks: develop.masks?.map((m) => retarget(m, list)) };
            const lost = develop.masks?.some((m) => m.shapes.some((s) => s.kind === "ai" && !list.masks.some((x) => x.kind === s.target && x.instance === s.instance)));
            if (lost) notes = [...notes, "Some AI masks found nothing like them on this photo."];
          } catch (e) {
            notes = [...notes, isAppError(e) ? e.message : "AI masks could not be detected on this photo."];
          }
        }
      }
      const pasted = { ...clip, develop };
      if (here) session.apply((d) => mergePanels(d, pasted.develop, pasted.panels), "Pasted settings");
      let previous: PreviousRecipe[] = [];
      try {
        if (others.length > 0) previous = await invoke<PreviousRecipe[]>("paste_develop", { ids: others, develop: pasted.develop, panels: pasted.panels, label: "Pasted settings" });
      } catch (e) {
        toast({ message: isAppError(e) ? e.message : "The settings could not be pasted." });
        return;
      }
      for (const message of notes) toast({ message });
      toast({
        message: `Pasted settings onto ${photos(targets.length)}`,
        undo: () => {
          if (here) session.undo();
          if (previous.length > 0) void invoke("restore_develop_drafts", { previous });
        },
      });
    },
    [assetId, session, client],
  );

  /** Lumen's settings on the system clipboard from somewhere else (another photo, a message), if any. */
  const fromSystem = useCallback(async () => {
    try {
      const text = await navigator.clipboard?.readText();
      if (!text || text === written.current) return null;
      return parseSettings(text);
    } catch {
      return null;
    }
  }, []);

  const paste = useCallback(
    async (clip?: CopiedSettings) => {
      if (clip) return apply(clip);
      // Valid Lumen settings on the system clipboard win over Lumen's own clipboard.
      const system = await fromSystem();
      if (system) {
        const notes: string[] = [];
        if (system.droppedBrush > 0) notes.push("Brush masks stay with their photo.");
        if (system.newer) notes.push("These settings come from a newer Lumen.");
        setIncoming({ settings: system.settings, notes });
        return;
      }
      const own = developClipboard.get();
      if (!own) {
        toast({ message: "Copy settings from a photo first." });
        return;
      }
      return apply(own);
    },
    [apply, fromSystem],
  );

  const copy = useCallback(
    (panels: Panel[]) => {
      developClipboard.set({ develop: session.develop, panels });
      setChoosing(false);
      toast({ message: "Copied settings" });
      // Also as readable JSON on the system clipboard, without masks the person did not choose.
      const develop = panels.includes("masks") ? session.develop : { ...session.develop, masks: undefined };
      void (async () => {
        try {
          const info = await invoke<AppInfo>("get_app_info").catch(() => null);
          const text = serializeSettings({ develop, panels }, info?.version ?? "unknown");
          await navigator.clipboard?.writeText(text);
          written.current = text;
        } catch {
          // The system clipboard refused: Lumen's own clipboard still holds the copy.
        }
      })();
    },
    [session.develop],
  );

  /** Everything this photo has, onto the selection. */
  const sync = useCallback(() => {
    const all: Panel[] = ["basic", "curve", "hsl", "grading", "detail", "effects"];
    void paste({ develop: session.develop, panels: all });
  }, [paste, session.develop]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (matches(e, "develop.copy-settings")) setChoosing(true);
      else if (matches(e, "develop.paste-settings")) void paste();
      else return;
      e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [paste]);

  const dialog = (
    <>
      <PanelChoice opened={choosing} title="Copy settings" confirmLabel="Copy" masks onConfirm={(c) => copy(c.panels)} onClose={() => setChoosing(false)} />
      <PanelChoice
        opened={incoming !== null}
        title="Paste settings"
        confirmLabel="Paste"
        masks
        onConfirm={(c) => {
          const next = incoming;
          setIncoming(null);
          if (next) void apply({ develop: next.settings.develop, panels: c.panels }, next.notes);
        }}
        onClose={() => setIncoming(null)}
      />
    </>
  );
  return { openCopy: () => setChoosing(true), paste, sync, dialog };
}
