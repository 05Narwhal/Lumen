import {
  IconBolt,
  IconCalendar,
  IconCheck,
  IconDownload,
  IconFolder,
  IconListCheck,
  IconMapPin,
  IconMapSearch,
  IconPhoto,
  IconRefresh,
  IconScan,
  IconSearch,
  IconSettings,
  IconSparkles,
  IconTag,
  IconUser,
  IconWand,
  IconWorld,
  type Icon,
} from "@tabler/icons-react";
import { useMemo } from "react";
import { usePlugins } from "@/hooks/use-plugins";
import { toast } from "@/components/toast";
import { isAppError } from "@/lib/ipc";
import { pluginUiAction } from "@/lib/plugins";
import { runPluginTool, type ToolScope } from "@/lib/plugin-tool";
import type { MenuContribution } from "@/lib/menus";
import type { ContributionSlot } from "@/types/generated/ContributionSlot";
import type { PluginInfo } from "@/types/generated/PluginInfo";

/** The icons a contribution may name: the same list as `CONTRIBUTION_ICONS` in `photos-plugins`. */
const ICONS: Record<string, Icon> = {
  "map-pin": IconMapPin,
  "map-search": IconMapSearch,
  search: IconSearch,
  sparkles: IconSparkles,
  wand: IconWand,
  photo: IconPhoto,
  calendar: IconCalendar,
  tag: IconTag,
  user: IconUser,
  world: IconWorld,
  download: IconDownload,
  refresh: IconRefresh,
  settings: IconSettings,
  "list-check": IconListCheck,
  check: IconCheck,
  folder: IconFolder,
  scan: IconScan,
  bolt: IconBolt,
};

export const SLOT_ICONS = Object.keys(ICONS);

/** The Tabler component for a contribution's icon name; none for no name or one off the list. */
export const slotIcon = (name: string | null | undefined): Icon | null => (name ? (ICONS[name] ?? null) : null);

/** One entry a ready plugin adds somewhere. Nothing here knows which plugin it came from. */
export interface Contribution {
  /** Stable per plugin, slot and label: the palette's command id. */
  id: string;
  pluginId: string;
  pluginName: string;
  slot: ContributionSlot;
  label: string;
  icon: Icon | null;
  action: string;
}

/** Only a plugin that is on and ready contributes: off, stopped, broken or too new adds nothing. */
const contributes = (p: PluginInfo) => p.enabled && p.state.type === "ready";

/** Every contribution of every live plugin, in plugin order then declaration order. */
export function liveContributions(plugins: PluginInfo[]): Contribution[] {
  return plugins.filter(contributes).flatMap((p) =>
    p.contributions.map(
      (c): Contribution => ({
        id: `plugin:${p.id}:${c.slot}:${c.label}`,
        pluginId: p.id,
        pluginName: p.name,
        slot: c.slot,
        label: c.label,
        icon: slotIcon(c.icon),
        action: c.action,
      }),
    ),
  );
}

export const contributionsFor = (plugins: PluginInfo[], slot: ContributionSlot): Contribution[] =>
  liveContributions(plugins).filter((c) => c.slot === slot);

/**
 * What the palette lists: every contribution, whatever its slot, once per plugin and label (the
 * same action offered in the viewer menu and the toolbar is one palette entry).
 */
export function paletteContributions(plugins: PluginInfo[]): Contribution[] {
  const seen = new Set<string>();
  return liveContributions(plugins).filter((c) => {
    const key = `${c.pluginId}\u0000${c.label}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/** What running a contribution needs from the app (injected so it can be tested). */
export interface RunContext {
  navigate: (to: string) => void;
  /** Calls the plugin's `ui.action` and resolves with its answer. */
  runAction?: (pluginId: string, action: string) => Promise<string | void>;
  /** Runs a `tool:<name>` action (injected so it can be tested). */
  runTool?: typeof runPluginTool;
  /** What a tool works on when the page knows better than its slot (an album page). */
  scope?: ToolScope;
}

const RUN_TOOL = "tool:";

/**
 * What a `tool:<name>` contribution works on by default: a menu over photos works on the
 * selection the app reports to the backend, a toolbar or the palette on the whole library.
 */
export const scopeOfSlot = (slot: ContributionSlot): ToolScope =>
  slot === "library.selection-menu" || slot === "viewer.menu" ? { scope: "selection" } : { scope: "library" };

/** `scopeOfSlot`, with the photos the menu was opened on when the slot is about a selection. */
export const scopeFor = (slot: ContributionSlot, ids?: number[]): ToolScope => {
  const base = scopeOfSlot(slot);
  return base.scope === "selection" && ids?.length ? { scope: "selection", assets: ids } : base;
};

const OPEN_REVIEW = "open-review:";

/** The review page a contribution opens (`src/routes/review/`). */
export const reviewPageFor = (pluginId: string, review: string) => `/review/${review}?plugin=${encodeURIComponent(pluginId)}`;

/**
 * `open-review:<id>` opens that review page; `tool:<name>` runs that `[[tools]]` entry on the
 * slot's scope (`plugin-tool.ts`); any other action is the plugin's own wasm action, and its
 * answer (a line of text) is shown as a toast.
 */
export async function runContribution(c: Contribution, ctx: RunContext): Promise<void> {
  if (c.action.startsWith(OPEN_REVIEW)) {
    ctx.navigate(reviewPageFor(c.pluginId, c.action.slice(OPEN_REVIEW.length)));
    return;
  }
  if (c.action.startsWith(RUN_TOOL)) {
    await (ctx.runTool ?? runPluginTool)({ id: c.pluginId, name: c.pluginName }, c.action.slice(RUN_TOOL.length), ctx.scope ?? scopeOfSlot(c.slot), ctx.navigate);
    return;
  }
  try {
    const answer = await (ctx.runAction ?? pluginUiAction)(c.pluginId, c.action);
    if (typeof answer === "string" && answer.trim()) toast({ message: answer });
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : `${c.pluginName} couldn't do that.` });
  }
}

/** The live contributions of one slot, from the installed plugins. */
export function useContributions(slot: ContributionSlot): Contribution[] {
  const { plugins } = usePlugins();
  return useMemo(() => contributionsFor(plugins, slot), [plugins, slot]);
}

/** Every live contribution once per plugin and label, for the command palette. */
export function usePaletteContributions(): Contribution[] {
  const { plugins } = usePlugins();
  return useMemo(() => paletteContributions(plugins), [plugins]);
}

/**
 * A slot's contributions as menu entries for a menu factory. `navigate` is null outside the
 * router (a hook test): *open-review* then does nothing.
 */
export function useMenuContributions(slot: ContributionSlot, navigate: ((to: string) => void) | null): MenuContribution[] {
  const list = useContributions(slot);
  return useMemo(
    () => list.map((c) => ({ id: c.id, label: c.label, icon: c.icon, run: (ids?: number[]) => void runContribution(c, { navigate: navigate ?? (() => {}), scope: scopeFor(c.slot, ids) }) })),
    [list, navigate],
  );
}
