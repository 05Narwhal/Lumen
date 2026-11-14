import {
  IconAlbum,
  IconEye,
  IconLock,
  IconRotate2,
  IconRotateClockwise2,
  IconStar,
  IconStarOff,
  IconTrash,
  IconUserScan,
} from "@tabler/icons-react";
import { DIVIDER, icon, moreIcon, openWithItems, tidy, type MenuItem, type OpenWithContext } from "./parts";

export interface PhotoMenuContext {
  favorite: boolean;
  /** The stars shown: the one photo's, or 0 for a selection. */
  rating?: number;
  /** Inside the Hidden folder there is nowhere further to hide to. */
  inHidden: boolean;
  /** macOS calls the file manager Finder. */
  mac: boolean;
  /** The installed apps that fit, and the default (Round 22 §9); left out, no app entries. */
  openWith?: OpenWithContext;
}

/** What a photo menu can do; an action left out leaves its item out. */
export interface PhotoMenuActions {
  open?: (id: number) => void;
  favorite: (ids: number[], on: boolean) => void;
  rate?: (ids: number[], stars: number) => void;
  addToAlbum: (ids: number[]) => void;
  tagPeople?: (id: number) => void;
  rotate: (ids: number[], dir: 1 | -1) => void;
  reveal: (id: number) => void;
  copyTo: (ids: number[]) => void;
  /** *Edit in* and *Open with ▸*: the app's id, or the *Choose another app…* dialog. */
  openWith?: { editIn: (ids: number[], appId: string) => void; chooseApp: (ids: number[]) => void };
  moveToHidden: (ids: number[]) => void;
  trash: (ids: number[]) => void;
}

/**
 * One photo or a selection of them (right-click on a selected tile acts on the whole
 * selection). Trash, the one destructive item, comes last after a divider.
 */
export function photoMenu(ids: number[], ctx: PhotoMenuContext, a: PhotoMenuActions): MenuItem[] {
  const one = ids.length === 1 ? ids[0] : null;
  const openWith = ctx.openWith
    ? openWithItems(ctx.openWith, {
        editIn: (appId) => a.openWith?.editIn(ids, appId),
        reveal: one !== null ? () => a.reveal(one) : undefined,
        copyTo: () => a.copyTo(ids),
        chooseApp: () => a.openWith?.chooseApp(ids),
      })
    : [];
  return tidy([
    a.rate && { label: "Rate", rating: ctx.rating ?? 0, onRate: (stars: number) => a.rate!(ids, stars) },
    one !== null && a.open && { label: "Open", icon: icon(IconEye), shortcut: "Enter", onSelect: () => a.open!(one) },
    {
      label: ctx.favorite ? "Remove from Favorites" : one !== null ? "Add to Favorites" : `Add ${ids.length} to Favorites`,
      icon: icon(ctx.favorite ? IconStarOff : IconStar),
      shortcut: "f",
      onSelect: () => a.favorite(ids, !ctx.favorite),
    },
    { label: "Add to album…", icon: icon(IconAlbum), shortcut: "a", onSelect: () => a.addToAlbum(ids) },
    // Round 22 §9: the app's own editors, then the rarer actions folded under More.
    ...(openWith.length ? [DIVIDER, ...openWith] : []),
    DIVIDER,
    {
      label: "More",
      icon: moreIcon(),
      children: tidy([
        { label: "Rotate left", icon: icon(IconRotate2), shortcut: "Mod+[", onSelect: () => a.rotate(ids, -1) },
        { label: "Rotate right", icon: icon(IconRotateClockwise2), shortcut: "Mod+]", onSelect: () => a.rotate(ids, 1) },
        // Round 4I: the viewer, with every face's box shown and the info panel's People out.
        one !== null && a.tagPeople && { label: "Tag people…", icon: icon(IconUserScan), onSelect: () => a.tagPeople!(one) },
        // Out of every view and behind the Hidden folder's password.
        !ctx.inHidden && { label: "Move to Hidden", icon: icon(IconLock), onSelect: () => a.moveToHidden(ids) },
      ]),
    },
    DIVIDER,
    {
      label: one !== null ? "Move to Trash" : `Move ${ids.length} to Trash`,
      icon: icon(IconTrash),
      shortcut: "Delete",
      danger: true,
      onSelect: () => a.trash(ids),
    },
  ]);
}
