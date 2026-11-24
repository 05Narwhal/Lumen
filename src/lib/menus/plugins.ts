import { IconDownload, IconEye, IconFileText, IconInfoCircle, IconPalette, IconRefresh, IconSettings, IconTrash } from "@tabler/icons-react";
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

export interface StorePluginMenuContext {
  /** The store button's word: *Get* installs, *Update* updates, *Open* shows the installed plugin. */
  action: "Get" | "Open" | "Update";
  /** Installed plugins can be removed. */
  installed: boolean;
  /** The installed plugin has settings or apps to show. */
  hasSettings: boolean;
}

export interface StorePluginMenuActions {
  primary: () => void;
  details: () => void;
  settings: () => void;
  remove: () => void;
}

/** A plugin in the store (tile, installed row, plugin page ⋯): Get / Update / Open · Details · Settings · Remove. */
export function storePluginMenu(ctx: StorePluginMenuContext, a: StorePluginMenuActions): MenuItem[] {
  return tidy([
    ctx.action !== "Open" && { label: ctx.action === "Get" ? "Get" : "Update", icon: icon(IconDownload), onSelect: a.primary },
    { label: "Details", icon: icon(IconInfoCircle), onSelect: a.details },
    ctx.installed && ctx.hasSettings && { label: "Settings", icon: icon(IconSettings), onSelect: a.settings },
    ctx.installed && { divider: true },
    ctx.installed && { label: "Remove", icon: icon(IconTrash), danger: true, onSelect: a.remove },
  ]);
}

export interface StoreThemeMenuContext {
  action: "Get" | "Keep" | "Use" | "In use";
  installed: boolean;
}

export interface StoreThemeMenuActions {
  primary: () => void;
  tryIt: () => void;
  details: () => void;
  remove: () => void;
}

/** A theme in the store (tile, theme page ⋯): Get / Keep / Use · Try · Details · Remove. */
export function storeThemeMenu(ctx: StoreThemeMenuContext, a: StoreThemeMenuActions): MenuItem[] {
  return tidy([
    ctx.action !== "In use" && { label: ctx.action, icon: icon(ctx.action === "Use" ? IconPalette : IconDownload), onSelect: a.primary },
    ctx.action === "Get" && { label: "Try", icon: icon(IconEye), onSelect: a.tryIt },
    { label: "Details", icon: icon(IconInfoCircle), onSelect: a.details },
    ctx.installed && { divider: true },
    ctx.installed && { label: "Remove", icon: icon(IconTrash), danger: true, onSelect: a.remove },
  ]);
}
