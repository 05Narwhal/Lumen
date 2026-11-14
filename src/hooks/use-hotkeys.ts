import { useEffect } from "react";
import { useLocation, useNavigate } from "react-router";
import { openPalette } from "@/components/command-palette/store";
import { toggleQuickLook } from "@/components/quick-look/store";
import { appCommands } from "@/lib/commands";
import { ADDED_DEFAULTS, matches, setKeyOverrides } from "@/lib/keybinds";
import { playStarBurst } from "@/components/star-burst";
import { invoke } from "@/lib/ipc";
import { toast } from "@/components/toast";
import { invalidateLibraryData } from "@/lib/events";
import { useQueryClient } from "@tanstack/react-query";
import { picking } from "@/lib/pick-mode";
import { selectAllHere, undoLast } from "@/lib/select-here";
import { clearSelection, selectionStore } from "./use-selection";
import { useSettings } from "./use-settings";
import { useTileActions } from "./use-tile-actions";

const isEditable = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);

/** Commands handled above, each with its own conditions (Develop owns the tile-size keys, the viewer owns the arrows). */
const GATED = new Set(["tiles-larger", "tiles-smaller", "tiles-reset", "go-back", "go-forward", "develop-filmstrip"]);

const byId = (id: string) => appCommands.find((c) => c.id === id);

/** One window-level keydown listener for the app-wide shortcuts in the UX spec §8.3. */
export function useGlobalHotkeys() {
  const navigate = useNavigate();
  const location = useLocation();
  const { settings, update } = useSettings();
  // Handlers outside React (and `matches`) read the person's keys from here.
  setKeyOverrides(settings?.keybinds);
  const qc = useQueryClient();
  const { rotate } = useTileActions();
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const ctx = { navigate, update, qc };
      const viewerOpen = new URLSearchParams(window.location.search).has("asset");
      const is = (id: string) => matches(e, id);
      // Pick mode (Round 19 §5) is only for choosing photos: nothing may act on them.
      const pickMode = picking();

      if (is("command-palette")) {
        e.preventDefault();
        openPalette();
        return;
      }
      if (isEditable(e.target)) return;

      const run = (id: string) => {
        e.preventDefault();
        byId(id)?.run(ctx);
      };
      // Develop zooms the photo with these (Round 4J); everywhere else they size the tiles.
      const inDevelop = location.pathname.startsWith("/develop");
      if (is("toggle-sidebar")) return run("toggle-sidebar");
      if (is("go-import")) return run("go-import");
      if (is("go-settings")) return run("go-settings");
      if (!inDevelop) {
        if (is("tiles-larger")) return run("tiles-larger");
        if (is("tiles-smaller")) return run("tiles-smaller");
        if (is("tiles-reset")) return run("tiles-reset");
      }
      // Turn the selection a quarter. Develop and the viewer's editor own their geometry.
      if ((is("rotate-left") || is("rotate-right")) && !viewerOpen && !pickMode && !inDevelop) {
        const ids = [...selectionStore.get().ids];
        if (ids.length === 0) return;
        e.preventDefault();
        void rotate(ids, is("rotate-right") ? 1 : -1);
        return;
      }
      // Round 10. Develop and the viewer's editors own these keys there.
      if (!viewerOpen && !inDevelop) {
        if (is("select-all")) {
          e.preventDefault();
          void selectAllHere();
          return;
        }
        if (is("select-none")) return run("select-none");
        if (is("undo-last") && undoLast()) {
          e.preventDefault();
          return;
        }
      }
      if (is("develop-selection") && !inDevelop && !pickMode) return run("develop-selection");
      if (is("export-selection") && !inDevelop && !pickMode) return run("export-selection");
      // Round 22 lane C: the open photo (viewer, Develop) or the selection.
      if (is("edit-in-default") && !pickMode) return run("edit-in-default");
      if (is("focus-search")) return run("focus-search");
      // Back and forward, as in a browser (Round 10); the viewer pages with the arrows.
      if (!viewerOpen && is("go-back")) return run("go-back");
      if (!viewerOpen && is("go-forward")) return run("go-forward");
      // Any other command the person gave a chord with Ctrl, Cmd or Alt; the rest are not ours.
      if (e.ctrlKey || e.metaKey || e.altKey) {
        const chorded = appCommands.find(
          (c) => (c.group === "Navigate" || c.group === "View" || c.id in ADDED_DEFAULTS) && !GATED.has(c.id) && (c.available?.(qc) ?? true) && is(c.id),
        );
        if (chorded) run(chorded.id);
        return;
      }

      if (is("search")) {
        e.preventDefault();
        document.getElementById("global-search")?.focus();
        return;
      }
      if (viewerOpen || pickMode) return; // the viewer owns Escape, Space and the arrows while open
      if (is("grid.clear")) {
        clearSelection();
        return;
      }
      if (is("grid.favorite") && selectionStore.get().ids.size > 0) {
        const ids = [...selectionStore.get().ids];
        // The same star burst the double-click path plays, over each tile that is
        // actually on screen, so the shortcut feels like the gesture. If none of them
        // are visible, one burst in the middle of the window still confirms the action.
        const onScreen = ids.map((id) => document.getElementById(`tile-${id}`)).filter(Boolean);
        if (onScreen.length === 0) playStarBurst();
        else for (const el of onScreen.slice(0, 12)) playStarBurst(el);
        invoke("set_favorite", { ids, value: true })
          .then(() => {
            invalidateLibraryData(qc);
            toast({ message: ids.length === 1 ? "Added to Favorites" : `Added ${ids.length} to Favorites`, undo: () => void invoke("set_favorite", { ids, value: false }).then(() => invalidateLibraryData(qc)) });
          })
          .catch(() => {});
        return;
      }
      if (is("grid.trash") && selectionStore.get().ids.size > 0) {
        const ids = [...selectionStore.get().ids];
        e.preventDefault();
        const inTrash = window.location.pathname.startsWith("/trash");
        if (inTrash) return; // the trash route offers Restore / Delete now explicitly
        invoke("trash_assets", { ids })
          .then(() => {
            clearSelection();
            invalidateLibraryData(qc);
            toast({ message: ids.length === 1 ? "Moved to Trash" : `Moved ${ids.length} to Trash`, undo: () => void invoke("restore_assets", { ids }).then(() => invalidateLibraryData(qc)) });
          })
          .catch(() => {});
        return;
      }
      if (is("grid.quick-look")) {
        const anchor = selectionStore.get().anchor;
        if (anchor !== null) {
          e.preventDefault();
          toggleQuickLook(anchor);
        }
        return;
      }
      const digit = appCommands.find((c) => c.group === "Navigate" && is(c.id));
      if (digit) {
        e.preventDefault();
        digit.run(ctx);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [navigate, update, location.pathname, qc, rotate]);
}
