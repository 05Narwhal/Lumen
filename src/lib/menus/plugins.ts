import { IconFileText, IconInfoCircle, IconRefresh, IconSettings, IconTrash } from "@tabler/icons-react";
import { icon, tidy, type MenuItem } from "./parts";

export interface PluginMenuContext {
  developerMode: boolean;
  /** Loaded from a folder rather than installed. */
  dev: boolean;
  /** The plugin has settings to show. */
  hasSettings: boolean;
}

export interface PluginMenuActions {
  details: () => void;
  settings: () => void;
  log: () => void;
  reload: () => void;
  remove: () => void;
}

/** A plugin row's ⋯ menu: Details · Settings · Log (Developer mode) · Reload (dev plugins) · Remove. */
export function pluginMenu(ctx: PluginMenuContext, a: PluginMenuActions): MenuItem[] {
  return tidy([
    { label: "Details", icon: icon(IconInfoCircle), onSelect: a.details },
    ctx.hasSettings && { label: "Settings", icon: icon(IconSettings), onSelect: a.settings },
    ctx.developerMode && { label: "Log", icon: icon(IconFileText), onSelect: a.log },
    ctx.dev && { label: "Reload", icon: icon(IconRefresh), onSelect: a.reload },
    { divider: true },
    { label: "Remove", icon: icon(IconTrash), danger: true, onSelect: a.remove },
  ]);
}
