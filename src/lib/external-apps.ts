import { useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { openPaletteSub } from "@/components/command-palette/store";
import { toast } from "@/components/toast";
import { pickFile, pickFolder } from "@/lib/dialogs";
import { isMac } from "@/components/window-controls";
import { selectionStore } from "@/hooks/use-selection";
import { invoke, isAppError } from "@/lib/ipc";
import type { OpenWithContext } from "@/lib/menus/parts";
import { queryKeys } from "@/lib/query-keys";
import type { AssetDetail } from "@/types/generated/AssetDetail";
import type { DefaultApps } from "@/types/generated/DefaultApps";
import type { ExternalApp } from "@/types/generated/ExternalApp";
import type { Media } from "@/types/generated/Media";
import type { OpenedInApp } from "@/types/generated/OpenedInApp";

/** What decides the media of a photo: a tile has both, so does a viewer's asset. */
export interface MediaFacts {
  mediaType: number;
  raw: boolean;
}

/** Stale after a minute: a newly installed editor shows up without a restart. */
const STALE_MS = 60_000;

/** The installed apps (every kind of media) and the person's defaults. */
export function useExternalApps() {
  const apps = useQuery({ queryKey: queryKeys.externalApps, queryFn: () => invoke<ExternalApp[]>("list_external_apps", { media: null }), staleTime: STALE_MS });
  const defaults = useQuery({ queryKey: queryKeys.defaultApps, queryFn: () => invoke<DefaultApps>("get_default_apps"), staleTime: STALE_MS });
  return { apps: apps.data ?? [], defaults: defaults.data ?? null };
}

/** Fetches (or reuses) both answers outside a component: the palette's commands. */
export async function loadExternalApps(qc: QueryClient): Promise<{ apps: ExternalApp[]; defaults: DefaultApps | null }> {
  const [apps, defaults] = await Promise.all([
    qc.fetchQuery({ queryKey: queryKeys.externalApps, queryFn: () => invoke<ExternalApp[]>("list_external_apps", { media: null }), staleTime: STALE_MS }),
    qc.fetchQuery({ queryKey: queryKeys.defaultApps, queryFn: () => invoke<DefaultApps>("get_default_apps"), staleTime: STALE_MS }),
  ]);
  return { apps, defaults };
}

const kindOf = (m: MediaFacts): Media => (m.mediaType === 1 ? "video" : m.raw ? "raw" : "photo");

/** What the selection is: one kind, or `mixed`. A Live Photo is a photo. */
export function mediaOf(selection: readonly MediaFacts[]): Media | "mixed" {
  const kinds = new Set(selection.map(kindOf));
  return kinds.size === 1 ? [...kinds][0] : "mixed";
}

/** The apps that take every kind of media in the selection, in the list's order. */
export function appsFor(apps: readonly ExternalApp[], selection: readonly MediaFacts[]): ExternalApp[] {
  const kinds = new Set(selection.map(kindOf));
  return apps.filter((a) => [...kinds].every((k) => a.accepts.includes(k)));
}

/**
 * The app *Edit in* opens: the default for the selection's kind, or, for a mix, the first
 * default (photo, RAW, video) that takes everything. Absent when none is set or it is gone.
 */
export function defaultFor(selection: readonly MediaFacts[], apps: readonly ExternalApp[], defaults: DefaultApps | null): ExternalApp | null {
  if (!defaults || selection.length === 0) return null;
  const fit = appsFor(apps, selection);
  const media = mediaOf(selection);
  const ids = media === "mixed" ? [defaults.photo, defaults.raw, defaults.video] : [defaults[media]];
  for (const id of ids) {
    const found = id ? fit.find((a) => a.id === id) : undefined;
    if (found) return found;
  }
  return null;
}

/** *Edit in* / *Open with*: copies are made and the app is started (spec §8). */
export async function editIn(qc: QueryClient, ids: number[], app: ExternalApp): Promise<void> {
  try {
    const name = ids.length === 1 ? await invoke<AssetDetail>("get_asset", { id: ids[0] }).then((a) => a.filename).catch(() => null) : null;
    await invoke<OpenedInApp>("open_in_app", { assetIds: ids, appId: app.id });
    toast({ message: name ? `Opening ${name} in ${app.name}` : ids.length === 1 ? `Opening photo in ${app.name}` : `Opening ${ids.length} photos in ${app.name}` });
  } catch (e) {
    if (isAppError(e) && e.code === "app_missing") void qc.invalidateQueries({ queryKey: queryKeys.externalApps });
    toast({ message: isAppError(e) ? e.message : `Could not open in ${app.name}.` });
  }
}

/** *Choose another app…*: the OS file dialog, the app remembered, then *Edit in* with it. */
export async function chooseAnotherApp(qc: QueryClient, ids: number[]): Promise<void> {
  const mac = isMac();
  const opts = {
    title: "Choose an app",
    filters: mac ? undefined : [{ name: "Apps", extensions: ["exe", "bat", "cmd", "sh", "AppImage"] }],
  };
  // macOS apps are folders (`.app`), which its dialog treats as a folder pick.
  const path = mac ? await pickFolder(opts) : await pickFile(opts);
  if (typeof path !== "string") return;
  try {
    const app = await invoke<ExternalApp>("add_custom_app", { path });
    await qc.invalidateQueries({ queryKey: queryKeys.externalApps });
    await editIn(qc, ids, app);
  } catch (e) {
    toast({ message: isAppError(e) ? e.message : "That isn't an app Lumen can open photos with." });
  }
}

/**
 * What the viewer's and Develop's menus take (Round 22 §9) for the photo open now: the apps
 * that fit, the default, and the two actions. `context` is absent until the photo is known.
 */
export function useOpenWith(photo: MediaFacts | null) {
  const qc = useQueryClient();
  const { apps, defaults } = useExternalApps();
  const mac = isMac();
  const mediaType = photo?.mediaType;
  const raw = photo?.raw;
  const context = useMemo<OpenWithContext | undefined>(() => {
    if (mediaType === undefined || raw === undefined) return undefined;
    const facts = [{ mediaType, raw }];
    return { apps: appsFor(apps, facts), defaultApp: defaultFor(facts, apps, defaults), mac };
  }, [mediaType, raw, apps, defaults, mac]);
  const editInApp = useCallback(
    (ids: number[], appId: string) => {
      const app = apps.find((a) => a.id === appId);
      if (app) void editIn(qc, ids, app);
    },
    [qc, apps],
  );
  const chooseApp = useCallback((ids: number[]) => void chooseAnotherApp(qc, ids), [qc]);
  return { context, editIn: editInApp, chooseApp };
}

/** Most photos the palette and the shortcut will look up one by one for their kind. */
const MOST_BY_KEY = 200;

/**
 * What the shortcut and the palette act on: the photo open in Develop or the viewer, else the
 * selection, each with what it is (the app list depends on it).
 */
async function target(qc: QueryClient): Promise<{ ids: number[]; facts: MediaFacts[] } | null> {
  const develop = /^\/develop\/(\d+)/.exec(window.location.pathname);
  const viewer = Number(new URLSearchParams(window.location.search).get("asset"));
  const ids = develop ? [Number(develop[1])] : viewer ? [viewer] : [...selectionStore.get().ids];
  if (ids.length === 0) return null;
  if (ids.length > MOST_BY_KEY) {
    toast({ message: `Select up to ${MOST_BY_KEY} photos, or right-click them, to open them in an app.` });
    return null;
  }
  try {
    const details = await Promise.all(ids.map((id) => qc.fetchQuery({ queryKey: queryKeys.asset(id), queryFn: () => invoke<AssetDetail>("get_asset", { id }), staleTime: STALE_MS })));
    return { ids, facts: details.map((d) => ({ mediaType: d.mediaType, raw: d.raw })) };
  } catch {
    return null;
  }
}

/** The name the palette shows on *Edit in …*: the photo default, else the first default set. */
export function defaultLabel(qc: QueryClient): string | null {
  const apps = qc.getQueryData<ExternalApp[]>(queryKeys.externalApps);
  const d = qc.getQueryData<DefaultApps>(queryKeys.defaultApps);
  const id = d?.photo ?? d?.raw ?? d?.video;
  const app = id ? apps?.find((a) => a.id === id) : undefined;
  return app ? `Edit in ${app.name}` : null;
}

/** *Edit in <default>* (`Mod+Shift+E`): the photo open now, else the selection. */
export async function editInDefault(qc: QueryClient): Promise<void> {
  const t = await target(qc);
  if (!t) return;
  const { apps, defaults } = await loadExternalApps(qc);
  const app = defaultFor(t.facts, apps, defaults);
  if (!app) {
    toast({ message: "No app is set for these photos. Choose one in Settings → External apps." });
    return;
  }
  await editIn(qc, t.ids, app);
}

/** *Open with…* in the palette: a list of the apps that fit, then the same as the menu's. */
export async function openWithPicker(qc: QueryClient): Promise<void> {
  const t = await target(qc);
  if (!t) return;
  const { apps } = await loadExternalApps(qc);
  const fit = appsFor(apps, t.facts);
  openPaletteSub({
    title: "Open with",
    items: [
      ...fit.map((app) => ({ id: `open-with:${app.id}`, label: app.name, iconUrl: app.iconUrl, run: () => void editIn(qc, t.ids, app) })),
      { id: "open-with:choose", label: "Choose another app…", run: () => void chooseAnotherApp(qc, t.ids) },
    ],
  });
}
