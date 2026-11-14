import { AnimatePresence, motion } from "motion/react";
import { pickAlbumId, sidebarRail } from "@/lib/pick-mode";
import { rememberPage } from "@/lib/return-to";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Outlet, useLocation, useNavigate } from "react-router";
import { AppShell } from "@/components/app-shell";
import { CommandPalette } from "@/components/command-palette";
import { AlbumPickerHost } from "@/components/album-picker";
import { CollectionPickerHost } from "@/components/collection-picker";
import { CoverEditorHost } from "@/components/cover-editor";
import { CloseGuard } from "@/components/close-guard";
import { PluginInstallHost } from "@/components/plugin-install";
import { FilmPlayerHost } from "@/components/film-player";
import { ConfirmDialogHost } from "@/components/confirm-dialog";
import { ContextMenuHost } from "@/components/context-menu";
import { ExportLossesHost, ExportSheetHost } from "@/components/export-sheet";
import { StarBurstHost } from "@/components/star-burst";
import { PresetNotesHost } from "@/components/preset-import";
import { ImportDropzone } from "@/components/import-dropzone";
import { QuickLook } from "@/components/quick-look";
import { SelectionBar } from "@/components/selection-bar";
import { Toaster } from "@/components/toast";
import { Viewer } from "@/components/viewer";
import { useDevices } from "@/hooks/use-devices";
import { useGlobalHotkeys } from "@/hooks/use-hotkeys";
import { useLibraries } from "@/hooks/use-libraries";
import { clearSelection } from "@/hooks/use-selection";
import { useSettings } from "@/hooks/use-settings";
import { useMcpScope } from "@/hooks/use-mcp-scope";
import { useUpdateNotices } from "@/hooks/use-updates";
import { usePresence } from "@/hooks/use-presence";
import { setEditVersions } from "@/lib/edit-versions";
import { invoke } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { EditVersion } from "@/types/generated/EditVersion";

/**
 * What the page transition is keyed by: the screen, not the photo inside it. Develop's
 * `/develop/:id` changes on every step through the filmstrip, and keying by the whole path
 * faded the screen out and mounted a new one each time (Round 4A: "the view reloads").
 * Other screens with an id (an album, a person) are different pages and still transition.
 */
export function pageKey(pathname: string): string {
  return pathname.startsWith("/develop") ? "/develop" : pathname;
}

/** Screens that fill the window and scroll inside their own panes. */
export const fixedPage = (pathname: string) => pathname.startsWith("/develop") || pathname.startsWith("/settings");

export function RootLayout() {
  const { settings, update } = useSettings();
  const { current, isLoaded } = useLibraries();
  const location = useLocation();
  const navigate = useNavigate();
  useGlobalHotkeys();
  useMcpScope(!!settings?.aiAssistants?.enabled);
  useUpdateNotices();
  usePresence(!!settings?.discord?.enabled);
  const { visible: devices } = useDevices();
  // Every image URL of an edited photo carries its version (`mediaUrl`); load them once and
  // again whenever the library changes.
  const { data: edited } = useQuery({
    queryKey: queryKeys.editVersions,
    queryFn: () => invoke<EditVersion[]>("list_edit_versions"),
    enabled: !!current,
  });
  useEffect(() => setEditVersions(edited ?? []), [edited]);

  useEffect(() => {
    if (isLoaded && current === null) navigate("/setup", { replace: true });
  }, [isLoaded, current, navigate]);

  // Selection lives in the frontend only and does not survive navigation (UX spec §8.1).
  useEffect(() => clearSelection(), [location.pathname]);

  // Settings takes the whole window, and its Back returns to the last page outside it.
  const inSettings = location.pathname.startsWith("/settings");
  useEffect(() => {
    if (!inSettings) rememberPage(location.pathname + location.search);
  }, [inSettings, location.pathname, location.search]);

  // Round 20 §6: *Add photos* rails the sidebar by itself; the saved setting is never written by it.
  // `peek` is the person opening the sidebar while picking and resets only when picking flips.
  // Already railed by the setting, the toggle still writes it (a peek could not open it).
  const setting = settings?.appearance.sidebarCollapsed ?? false;
  const picking = pickAlbumId(location.search) !== null;
  const [peek, setPeek] = useState(false);
  useEffect(() => setPeek(false), [picking]);
  const collapsed = sidebarRail(setting, picking, peek);
  const toggle = () =>
    picking && !setting
      ? setPeek((p) => !p)
      : update((s) => ({ ...s, appearance: { ...s.appearance, sidebarCollapsed: !s.appearance.sidebarCollapsed } }));

  return (
    <AppShell
      devices={devices.map((d) => ({ id: d.id, name: d.displayName, kind: d.kind, group: d.group }))}
      sidebarCollapsed={collapsed}
      onToggleSidebar={toggle}
      sidebar={!inSettings}
    >
      <AnimatePresence mode="wait" initial={false}>
        <motion.div
          key={pageKey(location.pathname)}
          className={fixedPage(location.pathname) ? "route-transition route-transition--fixed" : "route-transition"}
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: -6 }}
          transition={{ duration: 0.18, ease: [0.2, 0.8, 0.2, 1] }}
        >
          <Outlet />
        </motion.div>
      </AnimatePresence>
      <SelectionBar />
      <Viewer />
      <QuickLook />
      <ImportDropzone />
      <Toaster />
      <CommandPalette />
      <ContextMenuHost />
      <AlbumPickerHost />
      <CollectionPickerHost />
      <CoverEditorHost />
      <ExportSheetHost />
      <ExportLossesHost />
      <FilmPlayerHost />
      <ConfirmDialogHost />
      <CloseGuard />
      <PluginInstallHost />
      <StarBurstHost />
      <PresetNotesHost />
    </AppShell>
  );
}
