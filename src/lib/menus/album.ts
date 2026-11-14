import { IconArrowUp, IconArrowsShuffle, IconArrowsSort, IconCopy, IconDownload, IconEye, IconEyeOff, IconFilter, IconFolderOpen, IconFolderPlus, IconFolderSymlink, IconPalette, IconPencil, IconPhoto, IconTrash } from "@tabler/icons-react";
import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

/** What an album menu can do; an action left out leaves its item out. */
export interface AlbumMenuActions {
  open: () => void;
  rename: () => void;
  /** A smart album's rules, in place of Rename. */
  editRules?: () => void;
  /** Pick the cover from the album's own photos. */
  setCover?: () => void;
  /** Its photos or an icon cover: shape, icon and colour (Julian's 2026-11-08 list). */
  editCover?: () => void;
  /** New automatic prints (distinct album covers, 2026-11-09); forgets a hand-picked cover. */
  shuffleCover?: () => void;
  setHides: (hides: boolean) => void;
  duplicate?: () => void;
  export?: () => void;
  /** Collections (Round 5A): pick one to put the album in, or take it out of its own. */
  moveToCollection?: () => void;
  removeFromCollection?: () => void;
  delete: () => void;
}

/**
 * An album, on `/albums` and in the sidebar alike: Open · Rename… (Edit rules… for a smart
 * album) · Set cover from… · Cover… · Shuffle cover · hide or show its photos in the Library · Duplicate album ·
 * Export album… · Move to collection… · Remove from collection · — · Delete album. Deleting
 * asks first only when the album holds photos
 * (the caller's `delete` decides).
 */
export function albumMenu(album: AlbumSummary, a: AlbumMenuActions): MenuItem[] {
  const smart = album.kind === 1;
  return tidy([
    { label: "Open", icon: icon(IconFolderOpen), onSelect: a.open },
    smart && a.editRules
      ? { label: "Edit rules…", icon: icon(IconFilter), onSelect: a.editRules }
      : !smart && { label: "Rename…", icon: icon(IconPencil), onSelect: a.rename },
    a.setCover && album.itemCount > 0 && { label: "Set cover from…", icon: icon(IconPhoto), onSelect: a.setCover },
    a.editCover && { label: "Cover…", icon: icon(IconPalette), onSelect: a.editCover },
    // Only photos can be shuffled, and one photo has nothing to shuffle with.
    a.shuffleCover && !album.coverStyle && album.itemCount > 1 && { label: "Shuffle cover", icon: icon(IconArrowsShuffle), onSelect: a.shuffleCover },
    // A smart album's photos are found by its rules; hiding them would hide them from those rules.
    !smart &&
      (album.hidesItems
        ? { label: "Show its photos in the Library", icon: icon(IconEye), onSelect: () => a.setHides(false) }
        : { label: "Hide its photos from the Library", icon: icon(IconEyeOff), onSelect: () => a.setHides(true) }),
    a.duplicate && { label: "Duplicate album", icon: icon(IconCopy), onSelect: a.duplicate },
    a.export && album.itemCount > 0 && { label: "Export album…", icon: icon(IconDownload), onSelect: a.export },
    a.moveToCollection && { label: "Move to collection…", icon: icon(IconFolderSymlink), onSelect: a.moveToCollection },
    a.removeFromCollection && album.collectionId != null && { label: "Remove from collection", icon: icon(IconArrowUp), onSelect: a.removeFromCollection },
    DIVIDER,
    { label: "Delete album", icon: icon(IconTrash), danger: true, onSelect: a.delete },
  ]);
}

/** What an album's own page folds into its ⋯ menu. */
export interface AlbumPageMenuActions {
  byDate: boolean;
  setByDate: (byDate: boolean) => void;
  setHides: (hides: boolean) => void;
}

/**
 * The album page's header actions that fold away when the window is narrow: the order
 * (manual or by date) and whether its photos stay out of the Library. A smart album has
 * neither, so the caller shows no menu for it.
 */
export function albumPageMenu(album: AlbumSummary, a: AlbumPageMenuActions): MenuItem[] {
  return [
    a.byDate
      ? { label: "Sort manually", icon: icon(IconArrowsSort), onSelect: () => a.setByDate(false) }
      : { label: "Sort by date", icon: icon(IconArrowsSort), onSelect: () => a.setByDate(true) },
    album.hidesItems
      ? { label: "Show its photos in the Library", icon: icon(IconEye), onSelect: () => a.setHides(false) }
      : { label: "Hide its photos from the Library", icon: icon(IconEyeOff), onSelect: () => a.setHides(true) },
  ];
}

/** What a collection menu can do. */
export interface CollectionMenuActions {
  open: () => void;
  newInside: () => void;
  rename: () => void;
  /** Its albums' covers or an icon cover. */
  editCover?: () => void;
  move: () => void;
  /** Out of the collection it is in, to the top. */
  moveOut: () => void;
  delete: () => void;
}

/**
 * A collection (Round 5A), on `/albums` and in the sidebar: Open · New collection inside… ·
 * Rename… · Cover… · Move to collection… · Move to the top (when inside one) · — · Delete collection.
 * Deleting keeps its albums (they move up) and offers Undo, so it never asks.
 */
export function collectionMenu(c: { inside: boolean }, a: CollectionMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconFolderOpen), onSelect: a.open },
    { label: "New collection inside…", icon: icon(IconFolderPlus), onSelect: a.newInside },
    { label: "Rename…", icon: icon(IconPencil), onSelect: a.rename },
    a.editCover && { label: "Cover…", icon: icon(IconPalette), onSelect: a.editCover },
    { label: "Move to collection…", icon: icon(IconFolderSymlink), onSelect: a.move },
    c.inside && { label: "Move to the top", icon: icon(IconArrowUp), onSelect: a.moveOut },
    DIVIDER,
    { label: "Delete collection", icon: icon(IconTrash), danger: true, onSelect: a.delete },
  ]);
}
