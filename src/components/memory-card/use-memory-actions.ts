import type { MouseEvent } from "react";
import { useNavigate } from "react-router";
import { useContextMenu } from "@/components/context-menu";
import { openExportSheet } from "@/components/export-sheet";
import { toast } from "@/components/toast";
import { useHideMemory } from "@/hooks/use-memories";
import { exportMemoryFilm, playFilm } from "@/lib/film";
import { invoke, isAppError } from "@/lib/ipc";
import { memoryMenu } from "@/lib/menus";
import type { MemorySummary } from "@/types/generated/MemorySummary";

/**
 * What a memory does, in one place for the card and Home's hero: play its film, open it, and the
 * right-click menu (Play · Open · Make slideshow · Export photos… · Hide memory).
 */
export function useMemoryActions(memory: MemorySummary) {
  const { open: openMenu } = useContextMenu();
  const navigate = useNavigate();
  const hide = useHideMemory();
  const gone = (e: unknown) => toast({ message: isAppError(e) ? e.message : "This memory is gone." });
  const play = () => playFilm({ memoryId: memory.id, title: memory.title, coverAssetId: memory.coverAssetId });
  const open = () => navigate(`/memories/${memory.id}`);
  const menu = () =>
    memoryMenu(memory, {
      play,
      open,
      makeSlideshow: () =>
        void invoke<number>("save_memory_film", { memoryId: memory.id }).then((id) => navigate(`/slideshows/${id}`), gone),
      exportVideo: () => void exportMemoryFilm(memory.id),
      exportPhotos: () =>
        void invoke<number[]>("memory_curated_ids", { id: memory.id }).then(
          (ids) => (ids.length ? openExportSheet(ids) : toast({ message: "This memory has no photos to export." })),
          // A memory regenerated away since the list was drawn.
          gone,
        ),
      setHidden: (hidden) => void hide(memory, hidden).catch(gone),
    });
  return { play, open, menu, onContextMenu: (e: MouseEvent) => openMenu(e, menu()) };
}
