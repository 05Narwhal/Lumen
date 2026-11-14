import { IconEyeOff, IconPencil, IconPhoto } from "@tabler/icons-react";
import { DIVIDER, icon, tidy, type MenuItem } from "./parts";

export interface DeviceMenuActions {
  open: () => void;
  rename: () => void;
  hide: () => void;
}

/**
 * A camera or phone the library's photos came from (the sidebar's *Devices*): Open · Rename…
 * · — · Hide device. Hiding is undone from the toast and from Settings; nothing is deleted.
 */
export function deviceMenu(a: DeviceMenuActions): MenuItem[] {
  return tidy([
    { label: "Open", icon: icon(IconPhoto), onSelect: a.open },
    { label: "Rename…", icon: icon(IconPencil), onSelect: a.rename },
    DIVIDER,
    { label: "Hide device", icon: icon(IconEyeOff), onSelect: a.hide },
  ]);
}
