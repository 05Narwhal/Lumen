import { Button, Menu, Progress, Switch } from "@mantine/core";
import { importingLabel, useRunningImport } from "@/hooks/use-running-import";
import { IconAlertTriangle, IconBooks, IconBrandApple, IconChevronDown, IconDeviceMobile, IconDeviceSdCard, IconDownload } from "@tabler/icons-react";
import { pickFile, pickFiles as pickMediaFiles, pickFolder, revealPath } from "@/lib/dialogs";
import { appleDeviceIcon } from "@/lib/device-icons";
import { useState, type ReactNode } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { CodecCard, UsbmuxdCard } from "@/components/codec-card";
import { PluginImportCards } from "@/components/plugin-import-card";
import { Segmented } from "@/components/segmented";
import { useContextMenu } from "@/components/context-menu";
import { PageHeader } from "@/components/page-header";
import { toast } from "@/components/toast";
import { useVolumes } from "@/hooks/use-devices";
import { appleLibraryLink, useAppleLibraries } from "@/hooks/use-apple-libraries";
import { startLightroomImport, useLightroomCatalogs } from "@/hooks/use-lightroom-catalogs";
import type { CatalogRef } from "@/types/generated/CatalogRef";
import type { FoundPhotosLibrary } from "@/types/generated/FoundPhotosLibrary";
import { useIphones } from "@/hooks/use-iphones";
import {
  BATCH_CANCELLED,
  BATCH_COMPLETE,
  BATCH_FAILED,
  BATCH_INTERRUPTED,
  BATCH_RUNNING,
  useImport,
  useImportFailures,
  useImportSkips,
} from "@/hooks/use-import";
import { dismissedImports, importRowItems, SOURCE_FOLDER } from "@/lib/import-row-actions";
import { invoke, isAppError } from "@/lib/ipc";
import { jobsStore } from "@/lib/jobs-store";
import { MEDIA_EXTENSIONS } from "@/lib/media-extensions";
import type { ImportBatchSummary } from "@/types/generated/ImportBatchSummary";
import type { IphoneInfo } from "@/types/generated/IphoneInfo";
import type { VolumeInfo } from "@/types/generated/VolumeInfo";
import { formatRelative } from "@/utils/date";
import { isMac } from "@/utils/media";
import { RepairScreen } from "./repair";
import "./styles.scss";

const STATE_LABEL: Record<number, string> = {
  [BATCH_RUNNING]: "Importing",
  [BATCH_COMPLETE]: "Imported",
  [BATCH_FAILED]: "Failed",
  [BATCH_CANCELLED]: "Cancelled",
  [BATCH_INTERRUPTED]: "Interrupted",
};

function Failures({ batchId }: { batchId: number }) {
  const q = useImportFailures(batchId);
  return (
    <ul className="import-route__failures">
      {(q.data ?? []).map((f, i) => (
        <li key={i}>
          <code>{f.path}</code>
          <span>{f.reason}</span>
        </li>
      ))}
    </ul>
  );
}

const SKIP_REASON: Record<string, string> = {
  unsupported: "Not a photo or video",
  duplicate: "Already in the library",
  unreadable: "Could not be read",
  burst_frame: "Burst frame",
  not_downloaded: "Not downloaded from iCloud",
  failed_verify: "The copy did not match the original",
};

/** Every file the import left out and why, each with a way to find it (Round 19). */
function Skips({ batch }: { batch: ImportBatchSummary }) {
  const q = useImportSkips(batch.id);
  const fromFolder = batch.sourceKind === SOURCE_FOLDER && batch.sourcePath;
  return (
    <ul className="import-route__failures import-route__skips" aria-label="Files not imported">
      {(q.data ?? []).map((s, i) => (
        <li key={i}>
          <code>{s.path}</code>
          <span>{SKIP_REASON[s.reason] ?? s.reason}</span>
          {s.assetId !== null ? (
            <Button variant="subtle" size="compact-xs" onClick={() => void invoke("reveal_asset", { id: s.assetId }).catch(() => {})}>
              Show library copy
            </Button>
          ) : fromFolder ? (
            <Button variant="subtle" size="compact-xs" onClick={() => void revealPath(`${batch.sourcePath}/${s.path}`).catch(() => {})}>
              Show in folder
            </Button>
          ) : (
            <span />
          )}
        </li>
      ))}
    </ul>
  );
}

/** One line of a source list: icon, name and a line under it, then its actions. */
function SourceRow({
  icon,
  name,
  sub,
  extra,
  children,
  ...data
}: { icon: ReactNode; name: string; sub: string; extra?: ReactNode; children?: ReactNode } & Record<`data-${string}`, string>) {
  return (
    <li className="import-route__source" {...data}>
      <span className="import-route__source-icon">{icon}</span>
      <div className="import-route__source-text">
        <strong>{name}</strong>
        <span>{sub}</span>
        {extra}
      </div>
      <div className="import-route__source-actions">{children}</div>
    </li>
  );
}

/** The first few rows of a long list, the rest one click away. */
function CompactList({ label, rows, limit = 3 }: { label: string; rows: ReactNode[]; limit?: number }) {
  const [all, setAll] = useState(false);
  const hidden = all ? 0 : Math.max(0, rows.length - limit);
  return (
    <div className="import-route__compact">
      <ul className="import-route__list" aria-label={label}>
        {all ? rows : rows.slice(0, limit)}
      </ul>
      {hidden > 0 && (
        <Button variant="subtle" size="compact-xs" className="import-route__more" onClick={() => setAll(true)}>
          Show {hidden} more
        </Button>
      )}
    </div>
  );
}

function BatchRow({ batch, onCancel }: { batch: ImportBatchSummary; onCancel: (id: number) => void }) {
  const [showFailures, setShowFailures] = useState(false);
  const [showSkips, setShowSkips] = useState(false);
  const job = jobsStore.use().jobs.find((j) => j.id === `import:${batch.id}`);
  const running = batch.state === BATCH_RUNNING;
  const { open } = useContextMenu();
  const qc = useQueryClient();
  return (
    <>
      <tr
        className="import-route__batch"
        data-state={batch.state}
        // A running import has its own Cancel; its menu waits until it has finished.
        onContextMenu={running ? undefined : (e) => open(e, importRowItems(batch, qc))}
      >
        <td className="import-route__cell-source" title={batch.sourcePath ?? undefined}>
          <span className="import-route__batch-label">{batch.sourceLabel}</span>
          {batch.sourcePath && <span className="import-route__batch-path">{batch.sourcePath}</span>}
        </td>
        {running ? (
          <td colSpan={3}>
            <span className="import-route__status" data-state={batch.state}>
              {STATE_LABEL[batch.state]}
            </span>
            <span className="import-route__current"> · {job?.detail ?? "Scanning…"}</span>
            <Progress value={Math.round((job?.progress ?? 0) * 100)} size="xs" radius="xl" aria-label="Import progress" animated striped />
          </td>
        ) : (
          <>
            <td>{formatRelative(batch.startedAt)}</td>
            <td>
              <span className="import-route__status" data-state={batch.state}>
                {STATE_LABEL[batch.state]}
              </span>
              {batch.skipCount > 0 ? (
                <button type="button" className="import-route__failed" onClick={() => setShowSkips((v) => !v)} aria-expanded={showSkips}>
                  {batch.skipCount} not imported <IconChevronDown size={14} />
                </button>
              ) : (
                batch.skippedCount > 0 && <span className="import-route__note">{batch.skippedCount} skipped</span>
              )}
              {batch.failedCount > 0 && (
                <button type="button" className="import-route__failed" onClick={() => setShowFailures((v) => !v)} aria-expanded={showFailures}>
                  <IconAlertTriangle size={14} /> {batch.failedCount} failed <IconChevronDown size={14} />
                </button>
              )}
            </td>
            <td className="import-route__num">{batch.itemCount.toLocaleString()}</td>
          </>
        )}
        <td className="import-route__num">
          {running ? (
            <Button size="compact-xs" variant="default" onClick={() => onCancel(batch.id)}>
              Cancel
            </Button>
          ) : (
            <Button size="compact-xs" variant="default" component={Link} to="/library" aria-label="Open in Library">
              Open
            </Button>
          )}
        </td>
      </tr>
      {showFailures && (
        <tr className="import-route__failures-row">
          <td colSpan={5}>
            <Failures batchId={batch.id} />
          </td>
        </tr>
      )}
      {showSkips && (
        <tr className="import-route__failures-row">
          <td colSpan={5}>
            <Skips batch={batch} />
          </td>
        </tr>
      )}
    </>
  );
}

function VolumeRow({ volume, onImport, onForget }: { volume: VolumeInfo; onImport: () => Promise<unknown>; onForget: () => Promise<unknown> }) {
  const [busy, setBusy] = useState(false);
  const importing = useRunningImport(volume.key);
  const run = async () => {
    setBusy(true);
    try {
      await onImport();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };
  return (
    <SourceRow
      data-volume={volume.mount}
      icon={<IconDeviceSdCard size={22} />}
      name={volume.label}
      sub={
        volume.newItems === 0
          ? `Nothing new · ${volume.totalItems.toLocaleString()} items already imported`
          : `${volume.newItems.toLocaleString()} new of ${volume.totalItems.toLocaleString()} items in DCIM`
      }
      extra={
        volume.newItems < volume.totalItems && (
          <button type="button" className="import-route__again" onClick={() => void onForget()}>
            Import everything again
          </button>
        )
      }
    >
      <Button variant="default" size="compact-sm" component={Link} to={`/connected/volume/${encodeURIComponent(volume.mount)}`} aria-label="Show what is on it">
        Show
      </Button>
      <Button size="compact-sm" onClick={run} loading={busy} disabled={volume.newItems === 0 || importing.batchId !== null}>
        {importing.batchId !== null
          ? importingLabel(importing.progress)
          : volume.newItems === 0
            ? "Up to date"
            : `Import ${volume.newItems.toLocaleString()} new`}
      </Button>
    </SourceRow>
  );
}

/** An Apple Photos library Lumen found (Round 9B): what it is, where, and the way in. */
function AppleLibraryRow({ library }: { library: FoundPhotosLibrary }) {
  const place = library.disk || (library.place !== "Pictures" && library.place !== "Home folder") ? `on ${library.place}` : `in ${library.place}`;
  return (
    <SourceRow data-apple-library={library.path} icon={<IconBrandApple size={20} />} name={library.name} sub={place}>
      <Button variant="default" size="compact-sm" component={Link} to={appleLibraryLink(library)} aria-label="Show what is in it">
        Show
      </Button>
    </SourceRow>
  );
}

/** A Lightroom Classic catalog Lumen found (Round 11): its name, its photos, and the way in. */
function LightroomCatalogRow({ catalog, onImport }: { catalog: CatalogRef; onImport: () => Promise<unknown> }) {
  const [busy, setBusy] = useState(false);
  const photos = catalog.photoCount === null ? "Lightroom Classic catalog" : `${catalog.photoCount.toLocaleString()} ${catalog.photoCount === 1 ? "photo" : "photos"}`;
  return (
    <SourceRow data-lightroom-catalog={catalog.path} icon={<IconBooks size={20} />} name={catalog.name} sub={photos}>
      <Button
        variant="default"
        size="compact-sm"
        loading={busy}
        aria-label={`Import ${catalog.name}`}
        onClick={async () => {
          setBusy(true);
          try {
            await onImport();
          } finally {
            setBusy(false);
          }
        }}
      >
        Import
      </Button>
    </SourceRow>
  );
}

function IphoneRow({ phone, onImport }: { phone: IphoneInfo; onImport: () => Promise<unknown> }) {
  const [busy, setBusy] = useState(false);
  const importing = useRunningImport(`ios:${phone.udid}`);
  const PhoneIcon = appleDeviceIcon(phone.model);
  const run = async () => {
    setBusy(true);
    try {
      await onImport();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };
  return (
    <SourceRow
      data-udid={phone.udid}
      icon={<PhoneIcon size={22} />}
      name={phone.name}
      sub={phone.paired ? "New photos in Camera Roll" : "Unlock it and tap Trust to read it"}
    >
      {phone.paired && (
        <Button variant="default" size="compact-sm" component={Link} to={`/connected/iphone/${encodeURIComponent(phone.udid)}`} aria-label="Show what is on it">
          Show
        </Button>
      )}
      <Button size="compact-sm" onClick={run} loading={busy} disabled={!phone.paired || importing.batchId !== null}>
        {importing.batchId !== null ? importingLabel(importing.progress) : phone.paired ? "Import new photos" : "Waiting for Trust"}
      </Button>
    </SourceRow>
  );
}

const HISTORY_SHOWN = 10;

/** The `limit` newest batches (the list is newest first); a running import is always kept. */
function pickNewest<T extends { state: number }>(batches: T[], limit: number): T[] {
  let room = limit - batches.filter((b) => b.state === BATCH_RUNNING).length;
  return batches.filter((b) => {
    if (b.state === BATCH_RUNNING) return true;
    return room-- > 0;
  });
}

/** Import · Repair (`?mode=repair`, Round 21 B4): one page, two jobs, a switch under the title. */
export function ImportRoute() {
  const [params, setParams] = useSearchParams();
  const mode = params.get("mode") === "repair" ? "repair" : "import";
  const setMode = (m: "import" | "repair") =>
    setParams(
      (p) => {
        const next = new URLSearchParams(p);
        if (m === "repair") next.set("mode", "repair");
        else next.delete("mode");
        return next;
      },
      { replace: true },
    );
  return (
    <div className="import-route">
      <PageHeader title="Import" search={false} />
      <div className="import-route__body import-route__body--modes">
        <Segmented
          label="Import or repair"
          value={mode}
          onChange={setMode}
          data={[
            { label: "Import", value: "import" },
            { label: "Repair", value: "repair" },
          ]}
        />
      </div>
      {mode === "repair" ? (
        <div className="import-route__body">
          <RepairScreen />
        </div>
      ) : (
        <ImportBody />
      )}
    </div>
  );
}

function ImportBody() {
  const { batches, startFolderImport, startFilesImport, cancelImport } = useImport();
  const dismissed = dismissedImports.use();
  const { volumes, startImport: startVolumeImport, forget } = useVolumes();
  const appleLibraries = useAppleLibraries().data ?? [];
  const catalogs = useLightroomCatalogs().data ?? [];
  const { devices: iphones, usbmuxdAvailable, startImport: startIphoneImport } = useIphones();
  const [busy, setBusy] = useState(false);
  const [subfolders, setSubfolders] = useState(true);
  const run = async (start: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await start();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not start the import." });
    } finally {
      setBusy(false);
    }
  };
  const pick = async (asAlbums = false) => {
    const dir = await pickFolder({ title: asAlbums ? "Choose a folder to import as albums" : "Choose a folder to import" });
    if (typeof dir !== "string") return;
    await run(() => startFolderImport({ path: dir, recursive: subfolders, asAlbums }));
  };
  const navigate = useNavigate();
  // A library is a folder, except to macOS's file dialog, which treats it as one file.
  const pickLibrary = async () => {
    const title = "Choose an Apple Photos library";
    const picked = isMac()
      ? await pickFile({ title, filters: [{ name: "Photos library", extensions: ["photoslibrary"] }] })
      : await pickFolder({ title });
    if (typeof picked !== "string") return;
    if (!/\.photoslibrary[\\/]?$/i.test(picked)) {
      toast({ message: "That folder is not an Apple Photos library." });
      return;
    }
    navigate(`/connected/photos/${encodeURIComponent(picked.replace(/[\\/]$/, ""))}`);
  };
  const pickCatalog = async () => {
    const picked = await pickFile({ title: "Choose a Lightroom Classic catalog", filters: [{ name: "Lightroom catalog", extensions: ["lrcat"] }] });
    if (typeof picked !== "string") return;
    await run(() => startLightroomImport(picked));
  };
  const pickFiles = async () => {
    const files = await pickMediaFiles({
      title: "Choose photos and videos to import",
      filters: [{ name: "Photos and videos", extensions: MEDIA_EXTENSIONS }],
    });
    const list = Array.isArray(files) ? files : typeof files === "string" ? [files] : [];
    if (list.length === 0) return;
    await run(() => startFilesImport(list));
  };

  const [showAllHistory, setShowAllHistory] = useState(false);
  const history = batches.filter((b) => b.state === BATCH_RUNNING || !dismissed.has(b.id));
  // Ten newest; a running import is always shown and counts as one of the ten.
  const shownHistory = showAllHistory || history.length <= HISTORY_SHOWN ? history : pickNewest(history, HISTORY_SHOWN);
  const noDevices = volumes.length === 0 && iphones.length === 0;

  return (
    <>
      <div className="import-route__body">
        <CodecCard compact />

        {/* One obvious way in, stated once; the other ways sit behind the chevron. */}
        <div className="import-route__drop">
          <span className="import-route__drop-icon">
            <IconDownload size={22} />
          </span>
          <span className="import-route__drop-text">
            <strong>Drop photos, videos or folders anywhere in the window</strong>
            <span>Files are copied into your library. Nothing on the source changes.</span>
          </span>
          <Switch size="xs" label="Include subfolders" checked={subfolders} onChange={(e) => setSubfolders(e.currentTarget.checked)} />
          <Button.Group>
            <Button onClick={pickFiles} loading={busy}>
              Choose…
            </Button>
            <Menu position="bottom-end" withinPortal>
              <Menu.Target>
                <Button px="xs" aria-label="More ways to import" disabled={busy}>
                  <IconChevronDown size={16} />
                </Button>
              </Menu.Target>
              <Menu.Dropdown>
                <Menu.Item onClick={() => void pickFiles()}>Photos and videos…</Menu.Item>
                <Menu.Item onClick={() => void pick()}>Folder…</Menu.Item>
                <Menu.Item onClick={() => void pick(true)} title="Each folder with photos becomes an album, each folder with subfolders a collection">
                  Folder as albums…
                </Menu.Item>
                <Menu.Item
                  onClick={() => void pickLibrary()}
                  rightSection={appleLibraries.length > 0 ? <span className="import-route__found">found {appleLibraries.length}</span> : undefined}
                >
                  Apple Photos library…
                </Menu.Item>
                <Menu.Item onClick={() => void pickCatalog()}>Lightroom catalog file…</Menu.Item>
              </Menu.Dropdown>
            </Menu>
          </Button.Group>
        </div>

        <div className="import-route__cols">
          <div className="import-route__stack">
            <section className="import-route__section" aria-label="Devices">
              <h2>Devices</h2>
              {!usbmuxdAvailable && <UsbmuxdCard />}
              {!noDevices && (
                <ul className="import-route__list" aria-label="Devices found">
                  {volumes.map((v) => (
                    <VolumeRow key={v.mount} volume={v} onImport={() => startVolumeImport(v.mount)} onForget={() => forget(v.key)} />
                  ))}
                  {iphones.map((ph) => (
                    <IphoneRow key={ph.udid} phone={ph} onImport={() => startIphoneImport(ph.udid)} />
                  ))}
                </ul>
              )}
              {usbmuxdAvailable && noDevices && (
                <p className="import-route__hint">
                  <IconDeviceMobile size={18} />
                  Plug in a memory card or an unlocked iPhone and it shows up here.
                </p>
              )}
            </section>

            {appleLibraries.length > 0 && (
              <section className="import-route__section">
                <h2>
                  Apple Photos libraries <small>{appleLibraries.length}</small>
                </h2>
                <CompactList label="Apple Photos libraries" rows={appleLibraries.map((lib) => <AppleLibraryRow key={`${lib.disk ?? "local"}:${lib.path}`} library={lib} />)} />
              </section>
            )}

            <PluginImportCards />

            {catalogs.length > 0 && (
              <section className="import-route__section">
                <h2>
                  Lightroom catalogs <small>{catalogs.length}</small>
                </h2>
                <CompactList
                  label="Lightroom catalogs"
                  rows={catalogs.map((c) => (
                    <LightroomCatalogRow key={c.path} catalog={c} onImport={() => run(() => startLightroomImport(c.path))} />
                  ))}
                />
              </section>
            )}
          </div>

          <section className="import-route__section" aria-label="Imports">
            <h2>
              History {history.length > 0 && <small>{history.length === 1 ? "1 import" : `${history.length} imports`}</small>}
            </h2>
            {history.length === 0 ? (
              <p className="import-route__none">Nothing imported yet.</p>
            ) : (
              <table className="import-route__table">
                <thead>
                  <tr>
                    <th>Source</th>
                    <th>When</th>
                    <th>Result</th>
                    <th className="import-route__num">Photos</th>
                    <th aria-label="Actions" />
                  </tr>
                </thead>
                <tbody>
                  {shownHistory.map((b) => (
                    <BatchRow key={b.id} batch={b} onCancel={(id) => void cancelImport(id)} />
                  ))}
                </tbody>
              </table>
            )}
            {history.length > HISTORY_SHOWN && (
              <Button variant="subtle" color="gray" size="compact-sm" onClick={() => setShowAllHistory((v) => !v)}>
                {showAllHistory ? "Show fewer" : `Show all ${history.length}`}
              </Button>
            )}
          </section>
        </div>
      </div>
    </>
  );
}
