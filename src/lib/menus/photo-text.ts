import { IconCopy, IconFileText, IconSearch } from "@tabler/icons-react";
import { searchLabel } from "@/lib/photo-text";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export interface PhotoTextMenuActions {
  copy: () => void;
  copyAll: () => void;
  search: (text: string) => void;
}

/**
 * Text read in a photo (Round 13-3), on the overlay or in the info's *Text* section: Copy ·
 * Copy all · — · Search for “…”. `text` is the selection, else the line under the pointer;
 * away from any line only *Copy all* is left.
 */
export function photoTextMenu(ctx: { text: string | null }, a: PhotoTextMenuActions): MenuItem[] {
  const text = ctx.text?.trim() || null;
  return tidy([
    !!text && { label: "Copy", icon: icon(IconCopy), onSelect: a.copy },
    { label: "Copy all", icon: icon(IconFileText), onSelect: a.copyAll },
    DIVIDER,
    !!text && { label: `Search for “${searchLabel(text)}”`, icon: icon(IconSearch), onSelect: () => a.search(text) },
  ]);
}
