// Tauri events → query invalidation and UI stores. The frontend never polls.

import type { ModelsProgress } from "@/types/generated/ModelsProgress";
import type { PeopleProgress } from "@/types/generated/PeopleProgress";
import type { QueryClient } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { revealPath } from "@/lib/dialogs";
import { exportFinishedMessage, openExportLosses } from "@/components/export-sheet";
import { formatTimeLeft } from "@/utils/format";
import { importFinishedMessage, noteImportFinished } from "@/lib/import-messages";
import { toast } from "@/components/toast";
import type { ExportFinished } from "@/types/generated/ExportFinished";
import type { ExportProgress } from "@/types/generated/ExportProgress";
import type { ImportProgress } from "@/types/generated/ImportProgress";
import type { ImportSummary } from "@/types/generated/ImportSummary";
import type { JobsUpdate } from "@/types/generated/JobsUpdate";
import type { LibraryChanged } from "@/types/generated/LibraryChanged";
import type { LibraryFound } from "@/types/generated/LibraryFound";
import type { ProxyFinished } from "@/types/generated/ProxyFinished";
import type { IphonesUpdate } from "@/types/generated/IphonesUpdate";
import type { MacDiskInfo } from "@/types/generated/MacDiskInfo";
import type { VolumeInfo } from "@/types/generated/VolumeInfo";
import type { ProxyProgress } from "@/types/generated/ProxyProgress";
import type { SlideshowFinished } from "@/types/generated/SlideshowFinished";
import type { SlideshowProgress } from "@/types/generated/SlideshowProgress";
import { invoke } from "./ipc";
import { countdown, finishJob, jobsStore, removeJob, selectRunning, upsertJob } from "./jobs-store";
import { ocrProgressJob, peopleProgressJob, petsProgressJob } from "./people-progress";
import { activityJob, BACKGROUND_ID, backgroundJob } from "./activity-lines";
import type { ActivityUpdate } from "@/types/generated/ActivityUpdate";
import type { PetsProgress } from "@/types/generated/PetsProgress";
import type { OcrProgress } from "@/types/generated/OcrProgress";
import { backupFinishedMessage, backupProgressJob } from "./backup-progress";
import { checkFinishedMessage, checkProgressJob } from "./check-progress";
import type { CheckFinished } from "@/types/generated/CheckFinished";
import type { OriginalChanged } from "@/types/generated/OriginalChanged";
import type { RepairFinished } from "@/types/generated/RepairFinished";
import type { RecoverFinished } from "@/types/generated/RecoverFinished";
import type { RepairStatus } from "@/types/generated/RepairStatus";
import type { RepairResult } from "@/types/generated/RepairResult";
import { REPAIR_JOB, repairImportedMessage, repairProgressJob } from "./repair-progress";
import type { CheckProgressEvent } from "@/types/generated/CheckProgressEvent";
import type { BackupProgress } from "@/types/generated/BackupProgress";
import type { BackupRun } from "@/types/generated/BackupRun";
import { clearProxyProgress, proxyFailureMessage, setProxyProgress } from "./proxy-store";
import { listenFacesRadar } from "./face-radar";
import { queryKeys } from "./query-keys";

export const IMPORT_PROGRESS = "import://progress";
export const IMPORT_FINISHED = "import://finished";
export const JOBS_UPDATE = "jobs://update";
export const ACTIVITY_UPDATE = "activity://update";
export const LIBRARY_CHANGED = "library://changed";
export const LIBRARY_FOUND = "library://found";
export const DEVICE_VOLUMES = "device://volumes";
export const DEVICE_IPHONES = "device://iphones";
export const DEVICE_MACDISKS = "device://macdisks";
export const MODELS_PROGRESS = "models://progress";
export const MODELS_READY = "models://ready";
export const CLOSE_REQUESTED = "app://close-requested";
/** Round 5: closing with Develop edits waiting; the session writes them, then closes again. */
export const FLUSH_REQUESTED = "app://flush-requested";
/** Round 17: the updater's state changed (`UpdateStatus`). */
export const UPDATES_STATE = "updates://state";
export const PROXY_PROGRESS = "proxy://progress";
export const PROXY_FINISHED = "proxy://finished";
export const EXPORT_PROGRESS = "export://progress";
export const EXPORT_FINISHED = "export://finished";
export const SLIDESHOW_PROGRESS = "slideshow://progress";
export const SLIDESHOW_FINISHED = "slideshow://finished";
export const DENOISE_PROGRESS = "denoise://progress";
export const DENOISE_FINISHED = "denoise://finished";
export const AI_PROGRESS = "ai://progress";
export const AI_FINISHED = "ai://finished";
export const DEVICE_BROWSER_CHANGED = "device-browser://changed";
export const PEOPLE_PROGRESS = "people://progress";
export const PETS_PROGRESS = "pets://progress";
export const OCR_PROGRESS = "ocr://progress";
export const BACKUP_PROGRESS = "backup://progress";
export const BACKUP_FINISHED = "backup://finished";
export const CHECK_PROGRESS = "check://progress";
export const CHECK_FINISHED = "check://finished";
export const REPAIR_PROGRESS = "repair://progress";
export const REPAIR_FINISHED = "repair://finished";
/** Round 22 lane C: an original handed to an app was changed behind Lumen's back. */
export const ORIGINAL_CHANGED = "external://original-changed";
export const REPAIR_IMPORTED = "repair://imported";
export const REPAIR_RECOVERED = "repair://recovered";

/**
 * The scheduler's four lanes, named for a person reading the activity ring. Each lane
 * becomes its own line, so "something is happening" becomes "rendering thumbnails, 40
 * left". The scheduler reports queue depth, not totals, so progress is measured against
 * the deepest the queue has been since it last emptied — which is what a person means by
 * "how far along is it".
 */
// Work Rust reports as *N left*, counted up so a line never starts over (Round 9B).
const peopleCount = countdown();
const petsCount = countdown();
const ocrCount = countdown();
const backgroundCount = countdown();
/** The named activities on show, so one that ends is finished rather than left behind. */
let activityIds = new Set<string>();

/**
 * Resolves once the active refetches have settled, so a caller holding optimistic state
 * knows when the catalogue has caught up and its override can be dropped.
 */
export function invalidateLibraryData(qc: QueryClient): Promise<void> {
  const keys = [
    queryKeys.stats,
    ["asset"],
    ["neighbors"],
    ["recent"],
    ["albums"],
    ["album"],
    ["favorites"],
    ["devices"],
    ["collections"],
    ["people"],
    // Saving or reverting an edit moves the photo's version, which every image URL carries.
    queryKeys.editVersions,
    // A photo's stored recipe: Develop fetches its neighbours ahead, and a paste, sync, reset
    // or restore must not leave it opening with the settings it had before (Round 4A).
    ["edit"],
    ["develop-draft"],
    // Snapshots an AI assistant saves from Rust (`draft_variant`) reach Develop's sidebar here.
    ["develop-history"],
    ["tag-suggestions"],
    // Memories are regenerated after imports and when one is hidden.
    ["memories"],
    ["memory"],
    ["slideshows"],
    // A search's chips and sections name people, albums and memories (Round 4K).
    ["search-overview"],
    ["slideshow"],
    // Trashing or restoring photos changes which imports the history lists.
    queryKeys.batches,
    // Develop's filmstrip shows marks, ratings and whether a photo is edited.
    ["develop-strip"],
    ["develop-strip-ids"],
    // The Hidden folder's count moves when photos go in or out.
    ["vault"],
    // An import or a deletion changes which photos are duplicates.
    ["duplicates"],
    // A file that can't be opened leaves the grids and joins Settings' list, or comes back.
    queryKeys.unreadable,
  ];
  return Promise.all([invalidateLayouts(qc), ...keys.map((queryKey) => qc.invalidateQueries({ queryKey }))]).then(() => undefined);
}

/**
 * The layout summaries first, then their rows: rows are cached per arrangement
 * (`layoutFingerprint`), so a grid whose arrangement changed asks for new rows under the
 * new one, and only rows still on show under the same arrangement are refetched in place.
 */
export async function invalidateLayouts(qc: QueryClient): Promise<void> {
  await qc.invalidateQueries({ queryKey: ["layout"] });
  await qc.invalidateQueries({ queryKey: ["layout-rows"] });
}

export function invalidateEverything(qc: QueryClient) {
  qc.invalidateQueries();
}

export async function startEventBridge(qc: QueryClient): Promise<() => void> {
  const unlisteners = await Promise.all([
    listen<ImportProgress>(IMPORT_PROGRESS, (e) => {
      const p = e.payload;
      upsertJob({
        id: `import:${p.batchId}`,
        kind: "import",
        label: `Importing ${p.total} items`,
        progress: p.total ? p.done / p.total : 0,
        done: p.done,
        total: p.total,
        state: "running",
        // The file now, and once it can be said, how long is left.
        detail: [p.current, p.etaMs != null ? formatTimeLeft(p.etaMs) : null].filter(Boolean).join(" · ") || undefined,
      });
      qc.invalidateQueries({ queryKey: queryKeys.batches });
    }),
    listen<ImportSummary>(IMPORT_FINISHED, (e) => {
      const s = e.payload;
      finishJob(`import:${s.batchId}`);
      qc.invalidateQueries({ queryKey: queryKeys.batches });
      // A device page's new-item count; "seen" is recorded before this event is sent.
      qc.invalidateQueries({ queryKey: ["connected"] });
      qc.invalidateQueries({ queryKey: ["photos-library"] });
      invalidateLibraryData(qc);
      noteImportFinished(s);
      toast({ message: importFinishedMessage(s) });
    }),
    // Queue depths stay inside the app (Round 9B): only maintenance nobody named becomes a
    // line, *Tidying up the library*, and only while nothing named is running.
    listen<JobsUpdate>(JOBS_UPDATE, (e) => {
      const named = selectRunning(jobsStore.get()).some((j) => j.id !== BACKGROUND_ID && j.state === "running");
      const job = backgroundJob(e.payload, backgroundCount, named);
      if (job) upsertJob(job);
      else if (e.payload.maintenance <= 0) finishJob(BACKGROUND_ID);
      else removeJob(BACKGROUND_ID);
    }),
    // Named background work from Rust (`photos_jobs::activity`): *Preparing videos · 41 of 322*.
    listen<ActivityUpdate>(ACTIVITY_UPDATE, (e) => {
      const now = new Set<string>();
      for (const line of e.payload.items) {
        const job = activityJob(line);
        now.add(job.id);
        upsertJob(job);
      }
      for (const id of activityIds) if (!now.has(id)) finishJob(id);
      activityIds = now;
    }),
    listen<LibraryChanged>(LIBRARY_CHANGED, () => {
      invalidateLibraryData(qc);
      qc.invalidateQueries({ queryKey: queryKeys.mlStatus });
    }),
    // Model downloads are background work, so they report through the activity ring.
    listen<ModelsProgress>(MODELS_PROGRESS, (e) => {
      const p = e.payload;
      upsertJob({
        id: "models",
        kind: "maintenance",
        label: "Downloading smart features",
        progress: p.total ? p.received / p.total : 0,
        done: p.received / 1e6,
        total: p.total / 1e6,
        state: "running",
        detail: `${(p.received / 1e6).toFixed(0)} of ${(p.total / 1e6).toFixed(0)} MB`,
      });
    }),
    // The face scan reports after each batch; new people appear while it is still running.
    listen<PeopleProgress>(PEOPLE_PROGRESS, (e) => {
      const job = peopleProgressJob(e.payload.left, peopleCount);
      if (job) upsertJob(job);
      else finishJob("people");
      qc.invalidateQueries({ queryKey: ["people"] });
    }),
    // Round 5A: a backup runs in the background like an import, and says how it ended.
    listen<BackupProgress>(BACKUP_PROGRESS, (e) => upsertJob(backupProgressJob(e.payload))),
    listen<BackupRun>(BACKUP_FINISHED, (e) => {
      finishJob("backup");
      toast({ message: backupFinishedMessage(e.payload) });
      qc.invalidateQueries({ queryKey: queryKeys.backup });
    }),
    // Round 19 C4: *Check before deleting* runs in the background and says how it ended.
    listen<CheckProgressEvent>(CHECK_PROGRESS, (e) => upsertJob(checkProgressJob(e.payload))),
    listen<CheckFinished>(CHECK_FINISHED, (e) => {
      finishJob("check");
      toast({ message: checkFinishedMessage(e.payload) });
      qc.invalidateQueries({ queryKey: queryKeys.check });
      qc.invalidateQueries({ queryKey: queryKeys.repairHealth });
    }),
    // Round 22 B4: a recovery ends on its own event; the page's lists follow.
    listen<RecoverFinished>(REPAIR_RECOVERED, (e) => {
      finishJob(REPAIR_JOB);
      const { recovered, nothing, cancelled } = e.payload;
      const parts = [`${recovered.toLocaleString("en-US")} recovered`, `${nothing.toLocaleString("en-US")} with nothing readable`];
      toast({ message: (cancelled ? ["Stopped", ...parts] : parts).join(" · ") });
      qc.invalidateQueries({ queryKey: ["repair"] });
      qc.invalidateQueries({ queryKey: ["unreadable"] });
    }),
    // Round 21 B4: Import → Repair checks a source in the background and says how it ended.
    listen<RepairStatus>(REPAIR_PROGRESS, (e) => {
      qc.setQueryData(queryKeys.repairStatus, e.payload);
      upsertJob(repairProgressJob(e.payload));
    }),
    listen<RepairFinished>(REPAIR_FINISHED, (e) => {
      finishJob(REPAIR_JOB);
      if (e.payload.error) toast({ message: e.payload.error });
      qc.invalidateQueries({ queryKey: queryKeys.repairStatus });
      qc.invalidateQueries({ queryKey: ["repair", "items"] });
      qc.invalidateQueries({ queryKey: queryKeys.repairHealth });
      qc.invalidateQueries({ queryKey: queryKeys.repairOutcomes });
      qc.invalidateQueries({ queryKey: queryKeys.repairRecovered });
      qc.invalidateQueries({ queryKey: ["unreadable"] });
    }),
    // Round 21 B5: a re-import's new files arrive as an import batch; the ring's import line ends here.
    listen<RepairResult>(REPAIR_IMPORTED, (e) => {
      const r = e.payload;
      if (r.batchId != null) finishJob(`import:${r.batchId}`);
      qc.setQueryData(queryKeys.repairImported, r);
      toast({ message: repairImportedMessage(r) });
      qc.invalidateQueries({ queryKey: queryKeys.batches });
      invalidateLibraryData(qc);
    }),
    // Round 22 lane C: an app that should not write the original did.
    listen<OriginalChanged>(ORIGINAL_CHANGED, (e) => {
      toast({ message: `${e.payload.app} changed ${e.payload.name}. The original was modified outside Lumen.` });
    }),
    // Round 4C: the pet scan, as the face scan (*Looking for pets · 1,204 left*).
    listen<PetsProgress>(PETS_PROGRESS, (e) => {
      const job = petsProgressJob(e.payload.left, petsCount);
      if (job) upsertJob(job);
      else finishJob("pets");
      qc.invalidateQueries({ queryKey: ["pets"] });
    }),
    // Round 13: the text scan (*Reading text · 1,204 left*).
    listen<OcrProgress>(OCR_PROGRESS, (e) => {
      const job = ocrProgressJob(e.payload.left, ocrCount);
      if (job) upsertJob(job);
      else finishJob("ocr");
    }),
    listen(MODELS_READY, () => {
      finishJob("models");
      qc.invalidateQueries({ queryKey: queryKeys.mlStatus });
      // A model that arrived (Facet) changes the Experimental rows and the Masks panel; one that
      // failed puts its reason on its row (Round 18 C1).
      qc.invalidateQueries({ queryKey: queryKeys.experimentalCatalog });
    }),
    listen<VolumeInfo[]>(DEVICE_VOLUMES, (e) => {
      qc.setQueryData(queryKeys.volumes, e.payload);
      qc.invalidateQueries({ queryKey: ["connected", "volume"] });
      // A drive that came or went may hold an Apple Photos library (Round 9B).
      qc.invalidateQueries({ queryKey: queryKeys.appleLibraries });
      // …or a Lightroom catalog (Round 11).
      qc.invalidateQueries({ queryKey: queryKeys.lightroomCatalogs });
      qc.invalidateQueries({ queryKey: queryKeys.repairSources });
    }),
    listen<IphonesUpdate>(DEVICE_IPHONES, (e) => {
      qc.setQueryData(queryKeys.iphones, e.payload);
      qc.invalidateQueries({ queryKey: queryKeys.repairSources });
    }),
    listen<MacDiskInfo[]>(DEVICE_MACDISKS, (e) => {
      qc.setQueryData(queryKeys.macDisks, e.payload);
      qc.invalidateQueries({ queryKey: ["connected", "mac"] });
      qc.invalidateQueries({ queryKey: queryKeys.appleLibraries });
      qc.invalidateQueries({ queryKey: queryKeys.repairSources });
    }),
    // Exports report through the activity ring and end with a toast that can reveal them.
    listen<ExportProgress>(EXPORT_PROGRESS, (e) => {
      const p = e.payload;
      upsertJob({
        id: `export:${p.batch}`,
        kind: "export",
        label: `Exporting ${p.total} ${p.total === 1 ? "photo" : "photos"}`,
        progress: p.total ? (p.done + p.failed) / p.total : 0,
        done: p.done + p.failed,
        total: p.total,
        state: "running",
        detail: `${p.done + p.failed} of ${p.total}`,
      });
    }),
    listen<ExportFinished>(EXPORT_FINISHED, (e) => {
      const f = e.payload;
      finishJob(`export:${f.batch}`);
      // Losses outrank Reveal: the user should see what did not carry over. A batch added to
      // the library has no folder to reveal; it is in the grid.
      const action =
        f.losses.length > 0
          ? { label: "Details", run: () => openExportLosses(f.losses) }
          : f.done > 0 && !f.library
            ? { label: "Reveal", run: () => void revealPath(f.dir).catch(() => {}) }
            : undefined;
      toast({ message: exportFinishedMessage(f), action, durationMs: f.losses.length > 0 ? 12000 : undefined });
    }),
    // One slideshow export is a single long job: the ring counts it, the toast reveals it.
    listen<SlideshowProgress>(SLIDESHOW_PROGRESS, (e) => {
      const p = e.payload;
      // Round 6: a memory's film exports without a slideshow, under the memory's id.
      upsertJob({ id: `${p.memory ? "memory-film" : "slideshow"}:${p.id}`, kind: "export", label: p.memory ? "Exporting memory" : "Exporting slideshow", progress: p.progress, state: "running", detail: `${Math.round(p.progress * 100)}%` });
    }),
    listen<SlideshowFinished>(SLIDESHOW_FINISHED, (e) => {
      const f = e.payload;
      // A failed export goes at once; a finished one completes the ring first.
      if (f.ok) finishJob(`${f.memory ? "memory-film" : "slideshow"}:${f.id}`);
      else removeJob(`${f.memory ? "memory-film" : "slideshow"}:${f.id}`);
      qc.invalidateQueries({ queryKey: queryKeys.slideshows });
      const what = f.memory ? "Memory" : "Slideshow";
      if (f.ok && f.path) {
        const path = f.path;
        toast({ message: `${what} exported`, action: { label: "Reveal", run: () => void revealPath(path).catch(() => {}) } });
      } else if (f.error !== "cancelled") {
        toast({ message: f.error ? `${what} export failed: ${f.error}` : `${what} export failed.` });
      }
    }),
    listenFacesRadar(qc),
    listen<ProxyProgress>(PROXY_PROGRESS, (e) => setProxyProgress(e.payload.assetId, e.payload.progress)),
    listen<ProxyFinished>(PROXY_FINISHED, (e) => {
      clearProxyProgress(e.payload.assetId);
      qc.invalidateQueries({ queryKey: queryKeys.playback(e.payload.assetId) });
      const failed = proxyFailureMessage(e.payload);
      if (failed) toast({ message: failed });
    }),
    listen<LibraryFound>(LIBRARY_FOUND, (e) => {
      const f = e.payload;
      toast({
        message: `Found library “${f.name}” on a connected drive`,
        durationMs: 15000,
        action: {
          label: "Open",
          run: () => {
            invoke("locate_library", { id: f.id, path: f.path })
              .then(() => invalidateEverything(qc))
              .catch((err: { message?: string }) => toast({ message: err?.message ?? "Could not open the library." }));
          },
        },
      });
    }),
  ]);
  return () => unlisteners.forEach((u) => u());
}
