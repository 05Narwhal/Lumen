import type { Icon } from "@tabler/icons-react";
import { createElement } from "react";
import { IconAppWindow, IconDots } from "@tabler/icons-react";
import type { MenuItem } from "@/components/context-menu";
import type { ExternalApp } from "@/types/generated/ExternalApp";

export type { MenuItem };

export const icon = (I: Icon) => createElement(I, { size: 16 });

export const DIVIDER: MenuItem = { divider: true };

/** Items with the gaps closed: `false`/`null` entries dropped, no divider first, last or twice. */
export function tidy(items: (MenuItem | false | null | undefined)[]): MenuItem[] {
  const out: MenuItem[] = [];
  for (const it of items) {
    if (!it) continue;
    if ("divider" in it && (out.length === 0 || "divider" in out[out.length - 1])) continue;
    out.push(it);
  }
  while (out.length && "divider" in out[out.length - 1]) out.pop();
  return out;
}

/** The labels of a menu, dividers as `—`: what tests compare. */
export const labels = (items: MenuItem[]): string[] => items.map((i) => ("divider" in i ? "—" : i.label));

/** The key *Edit in <default app>* has (Mod+E is Export). */
export const EDIT_IN_KEY = "Mod+Shift+E";

export interface OpenWithContext {
  /** The installed apps that take every selected photo, in the list's order. */
  apps: ExternalApp[];
  /** The app *Edit in* opens for this selection, or none. */
  defaultApp: ExternalApp | null;
  mac: boolean;
}

export interface OpenWithActions {
  editIn(appId: string): void;
  /** One photo only: left out, the entry is left out. */
  reveal?(): void;
  copyTo?(): void;
  chooseApp(): void;
}

/**
 * *Edit in <default>* (absent without a default) and *Open with ▸* (Round 22 §9): the apps that
 * fit, a divider, the file-manager and copy entries, a divider, *Choose another app…*. The photo,
 * viewer and Develop menus all take their two entries from here.
 */
export function openWithItems(ctx: OpenWithContext, a: OpenWithActions): MenuItem[] {
  const d = ctx.defaultApp;
  return tidy([
    d && { label: `Edit in ${d.name}`, iconUrl: d.iconUrl, shortcut: EDIT_IN_KEY, onSelect: () => a.editIn(d.id) },
    {
      label: "Open with",
      icon: icon(IconAppWindow),
      children: tidy([
        ...ctx.apps.map((app): MenuItem => ({ label: app.name, iconUrl: app.iconUrl, onSelect: () => a.editIn(app.id) })),
        DIVIDER,
        a.reveal && { label: ctx.mac ? "Reveal in Finder" : "Show in Explorer", onSelect: a.reveal },
        a.copyTo && { label: "Copy files to…", onSelect: a.copyTo },
        DIVIDER,
        { label: "Choose another app…", onSelect: a.chooseApp },
      ]),
    },
  ]);
}

/** One entry a plugin adds to a menu (Round 25 P4a): already resolved, so a factory knows no plugin. */
export interface MenuContribution {
  id: string;
  label: string;
  icon: Icon | null;
  /** `ids`: the photos the menu was opened on, for a plugin tool that works on a selection. */
  run: (ids?: number[]) => void;
}

/** A menu's plugin entries as items; nothing when there are none. */
export function contributionItems(list: MenuContribution[] | undefined, ids?: number[]): MenuItem[] {
  return (list ?? []).map((c) => ({ label: c.label, icon: c.icon ? icon(c.icon) : undefined, onSelect: () => c.run(ids) }));
}

/** A submenu's icon for *More*. */
export const moreIcon = () => icon(IconDots);
