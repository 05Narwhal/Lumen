import { useCallback, useEffect, useState } from "react";
import { PanelChoice } from "@/components/panel-choice";
import { toast } from "@/components/toast";
import type { DevelopSession } from "@/hooks/use-develop-session";
import { developClipboard, developSelection, mergePanels } from "@/lib/develop";
import { invoke, isAppError } from "@/lib/ipc";
import { matches } from "@/lib/keybinds";
import type { Panel } from "@/types/generated/Panel";
import type { PreviousRecipe } from "@/types/generated/PreviousRecipe";

const photos = (n: number) => (n === 1 ? "1 photo" : `${n.toLocaleString()} photos`);

/**
 * Copy settings (choosing panels) and paste them — onto the filmstrip's selection when there
 * is one, else onto the open photo — with one Undo for all of it. `Mod+Shift+C` / `Mod+Shift+V`.
 * *Sync settings* is a copy of everything pasted onto the selection.
 */
export function useCopyPaste(session: DevelopSession, assetId: number) {
  const [choosing, setChoosing] = useState(false);

  const paste = useCallback(
    async (clip = developClipboard.get()) => {
      if (!clip) {
        toast({ message: "Copy settings from a photo first." });
        return;
      }
      const selected = [...developSelection.get().ids];
      const targets = selected.length > 0 ? selected : [assetId];
      const others = targets.filter((id) => id !== assetId);
      const here = targets.includes(assetId);
      if (here) session.apply((d) => mergePanels(d, clip.develop, clip.panels), "Pasted settings");
      let previous: PreviousRecipe[] = [];
      try {
        if (others.length > 0) previous = await invoke<PreviousRecipe[]>("paste_develop", { ids: others, develop: clip.develop, panels: clip.panels, label: "Pasted settings" });
      } catch (e) {
        toast({ message: isAppError(e) ? e.message : "The settings could not be pasted." });
        return;
      }
      toast({
        message: `Pasted settings onto ${photos(targets.length)}`,
        undo: () => {
          if (here) session.undo();
          if (previous.length > 0) void invoke("restore_develop_drafts", { previous });
        },
      });
    },
    [assetId, session],
  );

  const copy = useCallback(
    (panels: Panel[]) => {
      developClipboard.set({ develop: session.develop, panels });
      setChoosing(false);
      toast({ message: "Copied settings" });
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

  const dialog = <PanelChoice opened={choosing} title="Copy settings" confirmLabel="Copy" masks onConfirm={(c) => copy(c.panels)} onClose={() => setChoosing(false)} />;
  return { openCopy: () => setChoosing(true), paste, sync, dialog };
}
