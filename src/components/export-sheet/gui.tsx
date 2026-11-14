import { Button, Modal, Select, Slider, Switch, TextInput } from "@mantine/core";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Segmented } from "@/components/segmented";
import { toast } from "@/components/toast";
import { useAlbumCollections } from "@/hooks/use-album-collections";
import { useAlbums } from "@/hooks/use-albums";
import { useFfmpegStatus } from "@/hooks/use-media-status";
import { albumLabel } from "@/lib/album-collections";
import { askExportFolder, lastExportFolder } from "@/lib/export-folder";
import { invoke, isAppError } from "@/lib/ipc";
import { queryKeys } from "@/lib/query-keys";
import type { ExifToolStatus } from "@/types/generated/ExifToolStatus";
import type { ExportFinished } from "@/types/generated/ExportFinished";
import type { ExportFormat } from "@/types/generated/ExportFormat";
import type { ExportLoss } from "@/types/generated/ExportLoss";
import type { ExportOptions } from "@/types/generated/ExportOptions";
import type { ExportSummary } from "@/types/generated/ExportSummary";
import type { VideoCut } from "@/types/generated/VideoCut";
import { closeExportLosses, closeExportSheet, exportLossesStore, exportSheetStore } from "./store";
import { clearIfSelected } from "@/hooks/use-selection";
import "./styles.scss";

type Size = "full" | "4096" | "2048";
type Dest = "folder" | "library" | "album";
/** The album select's value for making one on the spot. */
const NEW_ALBUM = "new";

const count = (n: number, one: string, many = `${one}s`) => (n === 1 ? `1 ${one}` : `${n.toLocaleString()} ${many}`);
const photos = (n: number) => count(n, "photo");

/** What the selection is, in words: "3 photos and 1 video". */
export function selectionLabel(ids: number, s: ExportSummary | undefined) {
  if (!s || s.photos + s.videos === 0) return photos(ids);
  if (s.videos === 0) return photos(s.photos);
  if (s.photos === 0) return count(s.videos, "video");
  return `${photos(s.photos)} and ${count(s.videos, "video")}`;
}

/** The toast when an export batch is done. */
export function exportFinishedMessage(f: ExportFinished) {
  const failed = f.failed > 0 ? `, ${f.failed.toLocaleString()} could not be exported` : "";
  if (f.library?.album) {
    const { added, duplicates, album } = f.library;
    const dup = duplicates > 0 ? `, ${duplicates.toLocaleString()} ${duplicates === 1 ? "was" : "were"} already in the library` : "";
    return `Added ${(added + duplicates).toLocaleString()} to “${album}”${dup}${failed}`;
  }
  if (f.library) {
    const dup = f.library.duplicates > 0 ? `, ${f.library.duplicates.toLocaleString()} ${f.library.duplicates === 1 ? "was" : "were"} already there` : "";
    return `Added ${f.library.added.toLocaleString()} to the library${dup}${failed}`;
  }
  const base = `Exported ${photos(f.done)}${failed}`;
  if (f.done === 0) return base;
  if (f.losses.length > 0) return `${base} · some metadata did not carry over`;
  if (f.metadata === "verified") return `${base} · metadata verified`;
  if (f.metadata === "unavailable") return `${base} · metadata not carried (ExifTool is not installed)`;
  return base;
}

const FORMAT_HELP: Record<ExportFormat, string> = {
  jpeg: "Opens everywhere. Edits are applied.",
  png: "Lossless and larger. Edits are applied.",
  webp: "Smaller than JPEG at the same quality, or lossless. Edits are applied.",
  avif: "The smallest files; newer apps only. Edits are applied.",
  heic: "What iPhones save; opens in Apple's apps and most others. Edits are applied.",
  // Not offered in this dialog yet (Round 22 §8): copies for an external editor only.
  tiff: "Uncompressed, for editing in another app. Edits are applied.",
  original: "The imported files, untouched. Edited photos are exported as JPEG, because an original cannot carry an edit.",
};

/** The sheet's choices, remembered per viewer between exports. */
interface Choices {
  format: ExportFormat;
  quality: number;
  lossless: boolean;
  size: Size;
  metadata: boolean;
  videoCut: VideoCut;
}

const CHOICES_KEY = "lumen.export.choices";
const DEFAULT_CHOICES: Choices = { format: "jpeg", quality: 90, lossless: false, size: "full", metadata: true, videoCut: "precise" };

function readChoices(): Choices {
  try {
    const saved = JSON.parse(localStorage.getItem(CHOICES_KEY) ?? "null") as Partial<Choices> | null;
    return { ...DEFAULT_CHOICES, ...(saved ?? {}) };
  } catch {
    return DEFAULT_CHOICES;
  }
}

function saveChoices(c: Choices) {
  try {
    localStorage.setItem(CHOICES_KEY, JSON.stringify(c));
  } catch {
    // A private window: remembered for this session only.
  }
}

const LOSSY: ExportFormat[] = ["jpeg", "webp", "avif", "heic"];

/**
 * Export: the photos as Lumen shows them (edits applied) in any of five formats, or the
 * original files untouched; trimmed videos as their kept segments. The original's metadata
 * is carried into every file and read back (spec §5). Runs in the background after the sheet
 * closes; the activity ring shows progress and a toast reports what happened.
 */
export function ExportSheet({ ids, onClose }: { ids: number[]; onClose: () => void }) {
  const [c, setChoices] = useState<Choices>(readChoices);
  const set = (patch: Partial<Choices>) => setChoices((prev) => ({ ...prev, ...patch }));
  const [dest, setDest] = useState<Dest>("folder");
  // Round 5: into an album — an album photos can be put in, or a new one named here.
  const { albums: albumList, create: createAlbum } = useAlbums();
  const { collections } = useAlbumCollections();
  const [making, setMaking] = useState(false);
  const [album, setAlbum] = useState<string | null>(null);
  const [newAlbum, setNewAlbum] = useState("");
  const albumOptions = [...(albumList ?? []).filter((a) => a.kind === 0).map((a) => ({ value: String(a.id), label: albumLabel(collections, a) })), { value: NEW_ALBUM, label: "New album…" }];
  const albumReady = album !== null && (album !== NEW_ALBUM || newAlbum.trim() !== "");
  const ffmpeg = useFfmpegStatus().status;
  const exiftool = useQuery({ queryKey: queryKeys.exiftool, queryFn: () => invoke<ExifToolStatus>("get_exiftool_status"), staleTime: 60_000 });
  const summary = useQuery({ queryKey: ["export-summary", ids], queryFn: () => invoke<ExportSummary>("export_summary", { ids }) }).data;

  const noFfmpeg = ffmpeg?.available === false;
  const noExiftool = exiftool.data?.available === false;
  // A format that needs FFmpeg cannot stay chosen once FFmpeg is known to be missing.
  useEffect(() => {
    if (noFfmpeg && (c.format === "avif" || c.format === "heic")) setChoices((prev) => ({ ...prev, format: "jpeg" }));
  }, [noFfmpeg, c.format]);

  // `null`: the folder dialog was cancelled, so nothing is exported.
  const destination = async (): Promise<ExportOptions["destination"] | null> => {
    if (dest === "folder") {
      const dir = await askExportFolder();
      return dir === null ? null : { kind: "folder", dir };
    }
    if (dest === "library" || album === null) return { kind: "library" };
    if (album !== NEW_ALBUM) return { kind: "library", albumId: Number(album) };
    setMaking(true);
    try {
      return { kind: "library", albumId: (await createAlbum(newAlbum.trim())).id };
    } finally {
      setMaking(false);
    }
  };
  const start = async () => {
    let to: ExportOptions["destination"] | null;
    try {
      to = await destination();
    } catch {
      return; // the album could not be made; useAlbums has said so
    }
    if (to === null) return;
    const options: ExportOptions = {
      format: c.format,
      quality: c.quality,
      lossless: c.format === "webp" && c.lossless,
      maxLongEdge: c.format === "original" || c.size === "full" ? null : Number(c.size),
      metadata: c.metadata && !noExiftool,
      destination: to,
      videoCut: c.videoCut,
    };
    try {
      await invoke<number>("export_assets", { ids, options });
      clearIfSelected(ids);
      saveChoices(c);
      onClose();
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "The export could not start." });
    }
  };

  const ffmpegReason = noFfmpeg ? "Needs FFmpeg, which is not installed" : undefined;
  const what = selectionLabel(ids.length, summary);
  const onlyVideos = !!summary && summary.photos === 0 && summary.videos > 0;
  const showQuality = LOSSY.includes(c.format) && !(c.format === "webp" && c.lossless);

  return (
    <div className="export-sheet">
      {!onlyVideos && (
        <div className="export-sheet__field">
          <span className="export-sheet__label">Format</span>
          <Segmented<ExportFormat>
            label="Format"
            value={c.format}
            onChange={(format) => set({ format })}
            data={[
              { value: "jpeg", label: "JPEG" },
              { value: "png", label: "PNG" },
              { value: "webp", label: "WebP" },
              { value: "avif", label: "AVIF", disabled: noFfmpeg, title: ffmpegReason },
              { value: "heic", label: "HEIC", disabled: noFfmpeg, title: ffmpegReason },
              { value: "original", label: "Original" },
            ]}
          />
          <span className="export-sheet__help">{FORMAT_HELP[c.format]}</span>
        </div>
      )}
      {!onlyVideos && c.format === "webp" && (
        <Switch
          className="export-sheet__switch"
        size="sm"
          checked={c.lossless}
          onChange={(e) => set({ lossless: e.currentTarget.checked })}
          label="Lossless"
          description="Every pixel exactly, in a larger file."
        />
      )}
      {!onlyVideos && showQuality && (
        <div className="export-sheet__field">
          <span className="export-sheet__label">
            Quality <span className="export-sheet__value">{c.quality}</span>
          </span>
          <Slider value={c.quality} onChange={(quality) => set({ quality })} min={60} max={100} step={1} label={null} thumbLabel="Quality" />
        </div>
      )}
      {!onlyVideos && c.format !== "original" && (
        <div className="export-sheet__field">
          <span className="export-sheet__label">Size</span>
          <Segmented<Size>
            label="Size"
            value={c.size}
            onChange={(size) => set({ size })}
            data={[
              { value: "full", label: "Full size" },
              { value: "4096", label: "4096 px" },
              { value: "2048", label: "2048 px" },
            ]}
          />
        </div>
      )}
      {(summary?.trimmed ?? 0) > 0 && (
        <div className="export-sheet__field">
          <span className="export-sheet__label">Trimmed videos</span>
          <Segmented<VideoCut>
            label="Trimmed videos"
            value={c.videoCut}
            onChange={(videoCut) => set({ videoCut })}
            data={[
              { value: "precise", label: "Precise" },
              { value: "fast", label: "Fast" },
            ]}
          />
          <span className="export-sheet__help">
            {c.videoCut === "precise"
              ? "Re-encoded, so every cut lands exactly where you made it."
              : "Copied without re-encoding: nothing lost and nearly instant, but each part starts at the nearest key frame before your cut."}
          </span>
        </div>
      )}
      <div className="export-sheet__field">
        <span className="export-sheet__label">Destination</span>
        <Segmented<Dest>
          label="Destination"
          value={dest}
          onChange={setDest}
          data={[
            { value: "folder", label: "Folder" },
            { value: "library", label: "Add to library" },
            { value: "album", label: "Add to album" },
          ]}
        />
        {dest === "folder" ? (
          <div className="export-sheet__dest">
            <span className="export-sheet__help">{lastExportFolder() ? `Export asks where to save, starting in ${lastExportFolder()}.` : "Export asks where to save."}</span>
          </div>
        ) : dest === "album" ? (
          <div className="export-sheet__album">
            <Select size="xs" aria-label="Album" placeholder="Choose an album" data={albumOptions} value={album} onChange={setAlbum} searchable nothingFoundMessage="No album by that name" />
            {album === NEW_ALBUM && (
              <TextInput size="xs" aria-label="New album name" placeholder="Album name" value={newAlbum} onChange={(e) => setNewAlbum(e.currentTarget.value)} data-autofocus />
            )}
            <span className="export-sheet__help">New photos in the library and in that album, each linked to the one it was made from.</span>
          </div>
        ) : (
          <span className="export-sheet__help">New photos beside the originals, each linked to the one it was made from.</span>
        )}
      </div>
      <Switch
        className="export-sheet__switch"
        size="sm"
        checked={c.metadata && !noExiftool}
        disabled={noExiftool}
        onChange={(e) => set({ metadata: e.currentTarget.checked })}
        label="Keep metadata"
        description={
          noExiftool
            ? "Install ExifTool to carry metadata (Settings → Playback & codecs)."
            : c.format === "original" && !summary?.edited
              ? "Originals are copied byte for byte, metadata and all."
              : "Dates, places, camera details, keywords, ratings and labels go into every file and are checked afterwards."
        }
      />
      <div className="export-sheet__actions">
        <Button variant="subtle" onClick={onClose}>
          Cancel
        </Button>
        <Button onClick={() => void start()} disabled={ids.length === 0 || (dest === "album" && !albumReady)} loading={making}>
          {dest === "library" ? `Add ${what} to library` : dest === "album" ? `Add ${what} to album` : `Export ${what}`}
        </Button>
      </div>
    </div>
  );
}

/** Mounted once in the root layout; opened with `openExportSheet(ids)`. */
export function ExportSheetHost() {
  const { ids } = exportSheetStore.use();
  return (
    <Modal opened={ids !== null} onClose={closeExportSheet} title={ids && ids.length === 1 ? "Export" : `Export ${ids?.length.toLocaleString() ?? ""} items`} centered>
      {ids && <ExportSheet ids={ids} onClose={closeExportSheet} />}
    </Modal>
  );
}

/** Which files lost what, from an export's read-back. */
export function ExportLosses({ losses }: { losses: ExportLoss[] }) {
  return (
    <div className="export-losses">
      <p className="export-losses__intro">
        Each file was read back after it was written. These details are in the original but not, or not the same, in the export. Everything else carried over.
      </p>
      <ul className="export-losses__files">
        {losses.map((l) => (
          <li key={`${l.assetId}:${l.file}`} className="export-losses__file">
            <span className="export-losses__name">{l.file}</span>
            <dl className="export-losses__tags">
              {l.lost.map((t) => (
                <div key={t.tag} className="export-losses__tag">
                  <dt>{t.tag}</dt>
                  <dd>
                    <span className="export-losses__before">{t.before}</span>
                    <span aria-hidden> → </span>
                    <span className="export-losses__after">{t.after || "missing"}</span>
                  </dd>
                </div>
              ))}
            </dl>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** Mounted once in the root layout; opened from the finished toast's *Details*. */
export function ExportLossesHost() {
  const { losses } = exportLossesStore.use();
  return (
    <Modal opened={losses !== null} onClose={closeExportLosses} title="Some metadata did not carry over" centered size="lg">
      {losses && <ExportLosses losses={losses} />}
    </Modal>
  );
}
