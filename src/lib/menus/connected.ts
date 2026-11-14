import { IconAlbum, IconDownload, IconPlayerEject, IconExternalLink, IconFocusCentered, IconHistory, IconLayoutGrid, IconPhoto, IconPlayerPlay, IconRefresh } from "@tabler/icons-react";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export type ConnectedKind = "volume" | "iphone" | "mac";

export interface ConnectedMenuActions {
  open: () => void;
  browse: () => void;
  /** Cards and phones: what is new on them. */
  importNew?: () => void;
  /** Cards: forget what was imported from it, then import it all. */
  importAgain?: () => void;
  /** Cards and drives the OS can let go of. */
  eject?: () => void;
  /** Cards: forget what was imported from it, so it all counts as new. */
  forget?: () => void;
}

/**
 * A Connected entry (Phase E's sources): Open · Browse… · Import new · Import everything again
 * · — · Eject · Forget this device. Devices are read-only sources: nothing here writes to or
 * deletes from one; *Eject* only asks the OS to let it go, and only cards and drives have it.
 */
export function connectedMenu(kind: ConnectedKind, a: ConnectedMenuActions): MenuItem[] {
  const card = kind === "volume";
  return tidy([
    { label: "Open", icon: icon(IconExternalLink), onSelect: a.open },
    { label: "Browse…", icon: icon(IconLayoutGrid), onSelect: a.browse },
    kind !== "mac" && a.importNew && { label: "Import new", icon: icon(IconDownload), onSelect: a.importNew },
    card && a.importAgain && { label: "Import everything again", icon: icon(IconRefresh), onSelect: a.importAgain },
    DIVIDER,
    card && a.eject && { label: "Eject", icon: icon(IconPlayerEject), onSelect: a.eject },
    card && a.forget && { label: "Forget this device", icon: icon(IconHistory), onSelect: a.forget },
  ]);
}

export interface MapMenuActions {
  open: () => void;
  slideshow: () => void;
  addToAlbum: () => void;
  zoomToFit: () => void;
}

/** A pin on the map, one photo or a group: Open photos here · Start slideshow of these · Add to album… · Zoom to fit. */
export function mapMenu(single: boolean, a: MapMenuActions): MenuItem[] {
  return tidy([
    { label: single ? "Open photo" : "Open photos here", icon: icon(single ? IconPhoto : IconExternalLink), onSelect: a.open },
    !single && { label: "Start slideshow of these", icon: icon(IconPlayerPlay), onSelect: a.slideshow },
    { label: "Add to album…", icon: icon(IconAlbum), onSelect: a.addToAlbum },
    { label: "Zoom to fit", icon: icon(IconFocusCentered), onSelect: a.zoomToFit },
  ]);
}
