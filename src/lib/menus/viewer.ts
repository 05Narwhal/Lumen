import {
  IconAlbum,
  IconAperture,
  IconCopy,
  IconDownload,
  IconFolderOpen,
  IconInfoCircle,
  IconLock,
  IconPhoto,
  IconRotate2,
  IconRotateClockwise2,
  IconStar,
  IconStarOff,
  IconTrash,
  IconUserCircle,
  IconUserScan,
} from "@tabler/icons-react";
import { DIVIDER, icon, openWithItems, tidy, type MenuItem, type OpenWithContext } from "./parts";

export interface ViewerMenuContext {
  favorite: boolean;
  /** 0–5 stars. */
  rating: number;
  mac: boolean;
  inHidden: boolean;
  /** The viewer was opened from an album's grid: its cover can be this photo. */
  inAlbum: boolean;
  /** People with a face in this photo, for *Use as main photo of ▸*. */
  people: { personId: number; name: string; faceId: number }[];
  /** The installed apps that fit this photo, and the default (Round 22 §9). */
  openWith?: OpenWithContext;
}

/** What the viewer's photo menu can do; an action left out leaves its item out. */
export interface ViewerMenuActions {
  favorite: () => void;
  rate: (stars: number) => void;
  addToAlbum: () => void;
  rotate: (dir: 1 | -1) => void;
  /** Photos only: the displayed rendition, edits applied. */
  copyImage?: () => void;
  export: () => void;
  develop?: () => void;
  reveal: () => void;
  setAlbumCover?: () => void;
  useAsMainPhoto?: (personId: number, faceId: number) => void;
  tagPeople?: () => void;
  /** *Edit in* and *Open with ▸*: an app's id, or the *Choose another app…* dialog. */
  openWith?: { editIn: (appId: string) => void; chooseApp: () => void };
  showInfo: () => void;
  moveToHidden: () => void;
  trash: () => void;
}

/**
 * Right-click on the photo in the viewer: the grid tile's actions for this one photo, and
 * what only the viewer knows about (the rendition on screen, the album it was opened from,
 * the people in it). Trash comes last.
 */
export function viewerMenu(ctx: ViewerMenuContext, a: ViewerMenuActions): MenuItem[] {
  return tidy([
    { label: "Rate", rating: ctx.rating, onRate: a.rate },
    {
      label: ctx.favorite ? "Remove from Favorites" : "Add to Favorites",
      icon: icon(ctx.favorite ? IconStarOff : IconStar),
      shortcut: "f",
      onSelect: a.favorite,
    },
    { label: "Add to album…", icon: icon(IconAlbum), shortcut: "a", onSelect: a.addToAlbum },
    { label: "Rotate left", icon: icon(IconRotate2), shortcut: "Mod+[", onSelect: () => a.rotate(-1) },
    { label: "Rotate right", icon: icon(IconRotateClockwise2), shortcut: "Mod+]", onSelect: () => a.rotate(1) },
    DIVIDER,
    a.copyImage && { label: "Copy image", icon: icon(IconCopy), shortcut: "Mod+C", onSelect: a.copyImage },
    { label: "Export…", icon: icon(IconDownload), onSelect: a.export },
    a.develop && { label: "Develop", icon: icon(IconAperture), onSelect: a.develop },
    ...(ctx.openWith && a.openWith ? openWithItems(ctx.openWith, a.openWith) : []),
    { label: ctx.mac ? "Show original in Finder" : "Show original in folder", icon: icon(IconFolderOpen), onSelect: a.reveal },
    ctx.inAlbum && a.setAlbumCover && { label: "Set as album cover", icon: icon(IconPhoto), onSelect: a.setAlbumCover },
    ctx.people.length > 0 &&
      a.useAsMainPhoto && {
        label: "Use as main photo of",
        icon: icon(IconUserCircle),
        children: ctx.people.map((p) => ({ label: p.name, onSelect: () => a.useAsMainPhoto!(p.personId, p.faceId) })),
      },
    a.tagPeople && { label: "Tag people…", icon: icon(IconUserScan), shortcut: "p", onSelect: a.tagPeople },
    { label: "Show info", icon: icon(IconInfoCircle), shortcut: "i", onSelect: a.showInfo },
    !ctx.inHidden && { label: "Move to Hidden", icon: icon(IconLock), onSelect: a.moveToHidden },
    DIVIDER,
    { label: "Move to Trash", icon: icon(IconTrash), shortcut: "Delete", danger: true, onSelect: a.trash },
  ]);
}
