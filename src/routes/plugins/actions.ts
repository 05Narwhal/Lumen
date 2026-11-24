import { useState } from "react";
import { useNavigate } from "react-router";
import { openPluginInstall } from "@/components/plugin-install";
import { toast } from "@/components/toast";
import { usePlugins } from "@/hooks/use-plugins";
import { type MenuItem } from "@/lib/menus/parts";
import { storePluginMenu } from "@/lib/menus/plugins";
import { removePluginWithUndo } from "@/lib/plugin-remove";
import { useStoreStatus } from "@/hooks/use-store-status";
import { isAppError } from "@/lib/ipc";
import { storeInstall, storeUpdate } from "@/lib/plugin-store";
import type { StoreEntry } from "@/types/generated/StoreEntry";
import type { CardAction } from "./detail";

/** What Get / Open / Update mean for each plugin, and running them (shared by the list and the page). */
export function usePluginActions() {
  const { plugins, reload } = usePlugins();
  const { status, reload: reloadStatus } = useStoreStatus();
  const [busyId, setBusyId] = useState<string | null>(null);
  const navigate = useNavigate();
  const updates = status?.updates ?? [];

  const actionOf = (e: Pick<StoreEntry, "id">): CardAction => {
    if (updates.some((u) => u.id === e.id)) return { label: "Update", disabled: false };
    if (plugins.some((p) => p.id === e.id)) return { label: "Open", disabled: false };
    return { label: "Get", disabled: false };
  };

  /** *Get* / *Update*: the verified package opens in the install sheet, or is installed at once. */
  const run = async (id: string, update: boolean) => {
    if (busyId) return;
    setBusyId(id);
    try {
      const r = await (update ? storeUpdate(id) : storeInstall(id));
      if (r.path) openPluginInstall(r.path);
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : update ? "Could not update that plugin." : "Could not download that plugin." });
    } finally {
      setBusyId(null);
      reloadStatus();
    }
  };

  /** The button's own action: *Open* shows the installed plugin in Settings → Plugins. */
  const act = (e: Pick<StoreEntry, "id">) => {
    const a = actionOf(e);
    if (a.label === "Open") navigate(`/settings?section=plugins&plugin=${encodeURIComponent(e.id)}`);
    else void run(e.id, a.label === "Update");
  };
  /** The right-click / ⋯ menu of a plugin, in the store or installed. */
  const menuOf = (e: Pick<StoreEntry, "id" | "name">): MenuItem[] => {
    const info = plugins.find((p) => p.id === e.id);
    const id = encodeURIComponent(e.id);
    return storePluginMenu(
      { action: actionOf(e).label, installed: !!info, hasSettings: !!info && (info.settings.length > 0 || info.detectIds.length > 0) },
      {
        primary: () => act(e),
        details: () => navigate(`/plugins/${id}`),
        settings: () => navigate(`/settings?section=plugins&plugin=${id}`),
        remove: () => void removePluginWithUndo({ id: e.id, name: info?.name ?? e.name }, reload),
      },
    );
  };
  return { plugins, status, updates, busyId, actionOf, run, act, menuOf, reload };
}
