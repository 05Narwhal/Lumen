import { Anchor, Button, Select, PasswordInput, Switch } from "@mantine/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  IconDatabase,
  IconAperture,
  IconBrain,
  IconCalendar,
  IconBrandDiscord,
  IconExternalLink,
  IconSparkles,
  IconCloudUpload,
  IconFlask,
  IconInfoCircle,
  IconLicense,
  IconKeyboard,
  IconLibraryPhoto,
  IconMap2,
  IconMovie,
  IconMusic,
  IconPalette,
  IconPuzzle,
  IconRefresh,
  IconUsers,
} from "@tabler/icons-react";
import { useQuery } from "@tanstack/react-query";
import { useState, type ReactNode } from "react";
import { Link } from "react-router";
import { CodecCard } from "@/components/codec-card";
import { KeybindList } from "@/components/keybind-list";
import { BrandMark } from "@/components/brand-mark";
import { LicenceList } from "@/components/licence-list";
import { LibraryList } from "@/components/library-list";
import { Segmented } from "@/components/segmented";
import { ThemePicker } from "@/components/theme-picker";
import { toast } from "@/components/toast";
import { isMac } from "@/components/window-controls";
import { useBackup } from "@/hooks/use-backup";
import { resolveDateOrder, systemDateOrderQuery } from "@/hooks/use-date-order";
import { useDevices } from "@/hooks/use-devices";
import { useFfmpegStatus, useProxyCache } from "@/hooks/use-media-status";
import { useRemovedPeople } from "@/hooks/use-people";
import { useSettings, useSettingValue } from "@/hooks/use-settings";
import { invoke, isAppError } from "@/lib/ipc";
import { backupFinishedMessage } from "@/lib/backup-progress";
import { jobsStore } from "@/lib/jobs-store";
import { formatRelative } from "@/utils/date";
import type { DateOrder } from "@/types/generated/DateOrder";
import type { DateOrderSetting } from "@/types/generated/DateOrderSetting";
import type { BackupStatus } from "@/types/generated/BackupStatus";
import type { Schedule } from "@/types/generated/Schedule";
import { queryKeys } from "@/lib/query-keys";
import type { AppInfo } from "@/types/generated/AppInfo";
import type { ExifToolStatus } from "@/types/generated/ExifToolStatus";
import type { MlStatus } from "@/types/generated/MlStatus";
import type { Settings } from "@/types/generated/Settings";
import type { TileSize } from "@/types/generated/TileSize";
import { themeName } from "@/types/theme";
import { usePluginThemes } from "@/lib/plugin-theme";
import { formatBytes } from "@/utils/format";
import { AiAssistantsSection } from "./ai-assistants";
import { DiscordSection } from "./discord";
import { ExternalAppsSection, externalAppsLine } from "./external-apps";
import { ExperimentalSection } from "./experimental";
import { NotFaces, RemovedPeople } from "./people";
import { UpdatesSection } from "./updates";
import { useExternalApps } from "@/lib/external-apps";
import { PluginsSection } from "./plugins";
import { RawFormatsSection, rawFormatsLine, rawFormatsQuery } from "./raw-formats";
import { StorageSection } from "./storage";
import { SettingGroup, SettingRow } from "./row";

type Update = ReturnType<typeof useSettings>["update"];

/**
 * One Settings section (Round 5). The list is the whole of Settings: a new section (Backup,
 * say) is one more entry here. `status` is the line the list shows under the label.
 */
export interface Section {
  id: string;
  label: string;
  /** One sentence under the section's title. */
  intro: string;
  icon: ReactNode;
  Body: (props: { settings: Settings; update: Update }) => ReactNode;
  /**
   * `full`: the section's content takes the page's whole width and no inspector. `tiles`: its
   * content is not rows (People's faces), so search skips it, but it keeps the inspector's
   * state card. Either way search leaves it out.
   */
  layout?: "columns" | "full" | "tiles";
}

/**
 * One sidebar entry (Round 18 B1): a group of sections, which show as tabs at the top of its
 * page. `tabs` are section ids, in tab order; a section's id stays its address
 * (`?section=raw-formats`).
 */
export interface SectionGroup {
  id: string;
  label: string;
  icon: ReactNode;
  tabs: string[];
}

const ICON = { size: 17, stroke: 1.6 };

export const appInfoQuery = { queryKey: queryKeys.appInfo, queryFn: () => invoke<AppInfo>("get_app_info"), staleTime: Infinity };
export const mlStatusQuery = { queryKey: queryKeys.mlStatus, queryFn: () => invoke<MlStatus>("get_ml_status") };

function Appearance({ settings, update }: { settings: Settings; update: Update }) {
  const a = settings.appearance;
  // Every control holds its own value so the change lands inside the click, where a
  // transition can start. Settings persist through an async round trip, which React cannot
  // flush as a discrete update; the control would otherwise jump two frames later.
  const [tile, setTile] = useSettingValue(a.tileSize ?? "m");
  const [reduce, setReduce] = useSettingValue(a.reduceMotion ?? false);
  const pluginThemes = usePluginThemes();
  return (
    <>
      <SettingGroup title="Theme">
        <SettingRow wide>
          <ThemePicker value={{ theme: a.theme, scheme: a.scheme }} pluginThemes={pluginThemes} onChange={(v) => update((s) => ({ ...s, appearance: { ...s.appearance, theme: v.theme, scheme: v.scheme } }))} />
        </SettingRow>
      </SettingGroup>
      <SettingGroup title="Layout and motion">
        <SettingRow label="Default tile size" hint="How big photos are in the library grid.">
          <Segmented<TileSize>
            label="Default tile size"
            value={tile}
            onChange={(v) => {
              setTile(v);
              update((s) => ({ ...s, appearance: { ...s.appearance, tileSize: v } }));
            }}
            data={[
              { label: "S", value: "s" },
              { label: "M", value: "m" },
              { label: "L", value: "l" },
            ]}
          />
        </SettingRow>
        <SettingRow label="Reduce motion" hint="Cuts instead of slides and fades everywhere.">
          <Switch
            aria-label="Reduce motion"
            checked={reduce}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              // Turning motion back on must take effect before this very toggle is painted,
              // or the thumb animates at the zero duration still in force and snaps across.
              if (!v) document.documentElement.dataset.reducedMotion = "false";
              setReduce(v);
              update((s) => ({ ...s, appearance: { ...s.appearance, reduceMotion: v } }));
            }}
          />
        </SettingRow>
      </SettingGroup>
    </>
  );
}

function Library({ settings, update }: { settings: Settings; update: Update }) {
  const [hideRaw, setHideRaw] = useSettingValue(settings.appearance.hideRawWithJpeg ?? false);
  const [rebuilding, setRebuilding] = useState(false);
  const { hidden, setHidden } = useDevices();
  const rebuild = async () => {
    setRebuilding(true);
    try {
      const n = await invoke<number>("rebuild_search_index");
      toast({ message: `Search index rebuilt for ${n.toLocaleString()} items` });
    } catch (e) {
      toast({ message: isAppError(e) ? e.message : "Could not rebuild the search index." });
    } finally {
      setRebuilding(false);
    }
  };
  return (
    <>
      <SettingGroup title="Libraries">
        <SettingRow label="Your libraries" hint="Each is one folder of originals, thumbnails and the catalogue; it can live on an external drive." wide>
          <LibraryList />
        </SettingRow>
      </SettingGroup>
      <SettingGroup title="Showing photos">
        <SettingRow label="Hide RAW files" hint="When a camera saved RAW and JPEG, the Library shows only the JPEG. Searches, filters and albums show everything.">
          <Switch
            aria-label="Hide RAW files"
            checked={hideRaw}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setHideRaw(v);
              update((s) => ({ ...s, appearance: { ...s.appearance, hideRawWithJpeg: v } }));
            }}
          />
        </SettingRow>
        <SettingRow label="Search index" hint="Rebuild it if search ever misses something.">
          <Button size="xs" variant="default" loading={rebuilding} onClick={() => void rebuild()}>
            Rebuild
          </Button>
        </SettingRow>
      </SettingGroup>
      {hidden.length > 0 && (
        <SettingGroup title="Hidden devices">
          {hidden.map((d) => (
            <SettingRow key={d.id} label={d.displayName} hint={`${d.itemCount.toLocaleString()} items`}>
              <Button size="xs" variant="default" onClick={() => void setHidden({ id: d.id, hidden: false })}>
                Unhide
              </Button>
            </SettingRow>
          ))}
        </SettingGroup>
      )}
    </>
  );
}

/** Whether a backup is running: the ring hears of it before the status is fetched again. */
function useBackupRunning(status: BackupStatus | undefined) {
  const inRing = jobsStore.use().jobs.some((j) => j.id === "backup" && j.state === "running");
  return !!status?.running || inRing;
}

/** Backup (Round 5A): where, how often, and how the last run went. */
function Backup() {
  const { status, chooseDestination, clearDestination, setSchedule, start, cancel } = useBackup();
  const running = useBackupRunning(status);
  const [schedule, setScheduleValue] = useSettingValue<Schedule>(status?.config.schedule ?? "off");
  if (!status) return null;
  const { config, target, available } = status;
  if (!config.destination) {
    return (
      <SettingGroup title="Where">
        <SettingRow label="Back up to" hint="A folder or drive outside the library. Lumen copies your originals, albums and edits there, then only what changes.">
          <Button size="xs" onClick={() => void chooseDestination()}>
            Choose a folder or drive…
          </Button>
        </SettingRow>
      </SettingGroup>
    );
  }
  const last = config.last;
  return (
    <>
      <SettingGroup title="Where">
        <SettingRow
          label="Back up to"
          hint={
            <>
              <span className="settings__value">{target}</span>
              {!available && <span className="settings__warning"> · Not connected. Connect the drive and the backup runs.</span>}
            </>
          }
        >
          <Button size="xs" variant="default" onClick={() => void chooseDestination()}>
            Change…
          </Button>
          <Button size="xs" variant="subtle" onClick={() => void clearDestination()}>
            Stop backing up
          </Button>
        </SettingRow>
      </SettingGroup>
      <SettingGroup title="When">
        <SettingRow label="Schedule" hint="Runs by itself whenever the destination is connected.">
          <Segmented<Schedule>
            label="Schedule"
            value={schedule}
            onChange={(v) => {
              setScheduleValue(v);
              void setSchedule(v);
            }}
            data={[
              { value: "off", label: "Off" },
              { value: "daily", label: "Daily" },
              { value: "weekly", label: "Weekly" },
            ]}
          />
        </SettingRow>
        <SettingRow
          label="Last backup"
          hint={running ? "Backing up now…" : last ? `${backupFinishedMessage(last)} · ${formatRelative(last.at)}` : "Not backed up yet."}
        >
          {running ? (
            <Button size="xs" variant="default" onClick={() => void cancel()}>
              Stop
            </Button>
          ) : (
            <Button size="xs" disabled={!available} onClick={() => void start()}>
              Back up now
            </Button>
          )}
        </SettingRow>
      </SettingGroup>
      <p className="settings__note">
        Photos removed from the library stay in the backup for 30 days. To restore, open the backup as a library once the original is gone.
      </p>
    </>
  );
}

function Playback({ settings, update }: { settings: Settings; update: Update }) {
  const exiftool = useQuery({ queryKey: queryKeys.exiftool, queryFn: () => invoke<ExifToolStatus>("get_exiftool_status"), staleTime: Infinity });
  const proxyCache = useProxyCache();
  const [loop, setLoop] = useSettingValue(settings.playback.loopVideos ?? false);
  const [mute, setMute] = useSettingValue(settings.playback.muteByDefault ?? false);
  const [hwEncode, setHwEncode] = useSettingValue(settings.playback.hardwareEncoding ?? true);
  const [prepare, setPrepare] = useSettingValue(settings.playback.prepareVideos ?? true);
  const [watermark, setWatermark] = useSettingValue(settings.export?.watermark ?? true);
  return (
    <>
      <SettingGroup title="Helpers">
        <SettingRow label="FFmpeg and device access" wide>
          <CodecCard />
        </SettingRow>
        <SettingRow label="ExifTool" hint="Carries dates, places, camera details and keywords into every export.">
          <span className="settings__value">{exiftool.isPending ? "…" : exiftool.data?.available ? `ExifTool ${exiftool.data.version}` : "Not installed"}</span>
          <Button size="xs" variant="default" loading={exiftool.isFetching} onClick={() => void exiftool.refetch()}>
            Check again
          </Button>
        </SettingRow>
      </SettingGroup>
      <SettingGroup title="Videos">
        <SettingRow label="Loop videos">
          <Switch
            aria-label="Loop videos"
            checked={loop}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setLoop(v);
              update((s) => ({ ...s, playback: { ...s.playback, loopVideos: v } }));
            }}
          />
        </SettingRow>
        <SettingRow label="Mute by default">
          <Switch
            aria-label="Mute by default"
            checked={mute}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setMute(v);
              update((s) => ({ ...s, playback: { ...s.playback, muteByDefault: v } }));
            }}
          />
        </SettingRow>
        <SettingRow
          label="Prepare videos in the background"
          hint="Videos this computer cannot play directly are converted after import, so they open at once. Large ones get a 1080p copy. Pauses while you work and keeps 10 GB free."
        >
          <Switch
            aria-label="Prepare videos in the background"
            checked={prepare}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setPrepare(v);
              update((s) => ({ ...s, playback: { ...s.playback, prepareVideos: v } }));
            }}
          />
        </SettingRow>
        <SettingRow label="Playback proxies" hint="Converted copies of videos this computer cannot play directly.">
          <span className="settings__value">{formatBytes(proxyCache.bytes)}</span>
          <Button size="xs" variant="default" disabled={proxyCache.bytes === 0} onClick={() => void proxyCache.clear()}>
            Clear
          </Button>
        </SettingRow>
      </SettingGroup>
      <SettingGroup title="Slideshows">
        <SettingRow label="Use the graphics card for slideshow exports" hint="Several times faster; the processor takes over if it can't.">
          <Switch
            aria-label="Use the graphics card for slideshow exports"
            checked={hwEncode}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setHwEncode(v);
              update((s) => ({ ...s, playback: { ...s.playback, hardwareEncoding: v } }));
            }}
          />
        </SettingRow>
        <SettingRow label="Add the Lumen mark to videos Lumen makes" hint="Slideshows and memory films. Never your photos or your own videos.">
          <Switch
            aria-label="Add the Lumen mark to videos Lumen makes"
            checked={watermark}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setWatermark(v);
              update((s) => ({ ...s, export: { ...s.export, watermark: v } }));
            }}
          />
        </SettingRow>
      </SettingGroup>
    </>
  );
}

const ORDER_NAMES: Record<DateOrder, string> = { dmy: "Day-month-year", mdy: "Month-day-year", ymd: "Year-month-day" };
/** How `4.6.2026` reads in each order: a year last is never year first, so YMD reads it day first. */
const EXAMPLE: Record<DateOrder, string> = { dmy: "4 June 2026", mdy: "6 April 2026", ymd: "4 June 2026" };

function SearchDates({ settings, update }: { settings: Settings; update: Update }) {
  const system = useQuery(systemDateOrderQuery).data;
  const [setting, setSetting] = useSettingValue<DateOrderSetting>(settings.search?.dateOrder ?? "auto");
  const order = resolveDateOrder(setting, system);
  return (
    <SettingGroup>
      <SettingRow label="Date order" hint="Which way round a date in numbers is read when you type it, and written wherever Lumen shows one.">
        <Select
          aria-label="Date order"
          value={setting}
          data={[
            { value: "auto", label: `Automatic (${ORDER_NAMES[system ?? "dmy"]})` },
            { value: "dmy", label: ORDER_NAMES.dmy },
            { value: "mdy", label: ORDER_NAMES.mdy },
            { value: "ymd", label: ORDER_NAMES.ymd },
          ]}
          onChange={(picked) => {
            if (picked === null) return;
            const v = picked as DateOrderSetting;
            setSetting(v);
            update((s) => ({ ...s, search: { ...s.search, dateOrder: v } }));
          }}
        />
      </SettingRow>
      <SettingRow hint={`4.6.2026 means ${EXAMPLE[order]}`} />
    </SettingGroup>
  );
}

function MapSection({ settings, update }: { settings: Settings; update: Update }) {
  const [online, setOnline] = useSettingValue(settings.map.online ?? true);
  const [exact, setExact] = useSettingValue(settings.exactPlaces ?? false);
  return (
    <SettingGroup>
      <SettingRow
        label="Online map detail"
        hint="Streets, place names and satellite imagery from OpenFreeMap and Esri. They see which areas you look at, never your photos. Off, the map is a built-in world outline that needs no connection."
      >
        <Switch
          aria-label="Online map detail"
          checked={online}
          onChange={(e) => {
            const v = e.currentTarget.checked;
            setOnline(v);
            update((s) => ({ ...s, map: { ...s.map, online: v } }));
          }}
        />
      </SettingRow>
      <SettingRow
        label="Exact place names"
        hint="Sends each photo's location, never the photo, to OpenStreetMap to name its town and village exactly. Off: Lumen's built-in atlas names them, less exactly near borders. Names © OpenStreetMap contributors (ODbL)."
      >
        <Switch
          aria-label="Exact place names"
          checked={exact}
          onChange={(e) => {
            const v = e.currentTarget.checked;
            setExact(v);
            update((s) => ({ ...s, exactPlaces: v }));
          }}
        />
      </SettingRow>
    </SettingGroup>
  );
}

function Music({ settings, update }: { settings: Settings; update: Update }) {
  const [key, setKey] = useSettingValue(settings.music.jamendoClientId ?? "");
  return (
    <SettingGroup>
      <SettingRow
        label="Jamendo client id"
        hint={
          <>
            A free id from <code className="settings__code">devportal.jamendo.com</code> turns on Creative Commons music for slideshows. Without one, Lumen asks Jamendo for nothing; with one, every request goes through Lumen, so the key stays on this computer.
          </>
        }
      >
        <PasswordInput
          className="settings__field"
          aria-label="Jamendo client id"
          placeholder="Not set"
          value={key}
          onChange={(e) => setKey(e.currentTarget.value)}
          onBlur={(e) => update((s) => ({ ...s, music: { ...s.music, jamendoClientId: e.currentTarget.value.trim() || null } }))}
        />
      </SettingRow>
    </SettingGroup>
  );
}

function Smart({ settings, update }: { settings: Settings; update: Update }) {
  const info = useQuery(appInfoQuery);
  const ml = useQuery(mlStatusQuery);
  const [smart, setSmart] = useSettingValue(settings.smart.enabled ?? true);
  return (
    <>
      <SettingGroup>
        <SettingRow label="On-device models" hint={`They pick a memory's best moments, shape slideshows, find faces and the beat of a track, all on this computer. ${mlLine(smart, ml.data)}`}>
          <Switch
            aria-label="Smart features"
            checked={smart}
            onChange={(e) => {
              const v = e.currentTarget.checked;
              setSmart(v);
              update((s) => ({ ...s, smart: { ...s.smart, enabled: v } }));
              if (v) void invoke("fetch_models");
            }}
          />
        </SettingRow>
        {info.data?.platform === "windows" && (
          <SettingRow label="Use the graphics card for Denoise" hint="Seconds instead of minutes. Turn it off if Denoise fails on this computer.">
            <Switch
              aria-label="Use the graphics card for Denoise"
              checked={settings.smart.gpu ?? true}
              disabled={!smart}
              onChange={(e) => {
                const v = e.currentTarget.checked;
                update((s) => ({ ...s, smart: { ...s.smart, gpu: v } }));
              }}
            />
          </SettingRow>
        )}
      </SettingGroup>
    </>
  );
}

function People() {
  return (
    <>
      <RemovedPeople />
      <NotFaces />
    </>
  );
}

function Keyboard() {
  return (
    <SettingGroup>
      <SettingRow label="Shortcuts" hint={`The command palette (${isMac() ? "⌘K" : "Ctrl+K"}) lists them too.`} wide>
        <KeybindList />
      </SettingRow>
    </SettingGroup>
  );
}

const SOURCE_URL = "https://github.com/05Narwhal/lumen";

function About() {
  const info = useQuery(appInfoQuery);
  return (
    <SettingGroup>
      <SettingRow label="Version">
        <span className="settings__about">
          <BrandMark size={32} />
          <span className="settings__value">Lumen {info.data?.version ?? "…"}</span>
        </span>
      </SettingRow>
      <SettingRow label="Made by" hint="Free and open source. Nothing here is paid, and nothing leaves your computer.">
        <span className="settings__about-credit">
          <span className="settings__value">Created by Julian · JL Studios</span>
          <span className="settings__value">Free and open source · GPL-3.0-or-later</span>
          <Anchor
            href={SOURCE_URL}
            size="sm"
            onClick={(e) => {
              e.preventDefault();
              void openUrl(SOURCE_URL).catch(() => window.open(SOURCE_URL, "_blank", "noopener"));
            }}
          >
            Source code
          </Anchor>
        </span>
      </SettingRow>
      <SettingRow label="App data" hint="Settings, the list of libraries and the downloaded models.">
        <code className="settings__code">{info.data?.appDataDir ?? "…"}</code>
      </SettingRow>
      <SettingRow label="Setup" hint="Theme, library and helpers, from the start.">
        <Button size="xs" variant="default" component={Link} to="/setup">
          Run setup again
        </Button>
      </SettingRow>
    </SettingGroup>
  );
}

/** Every third-party work Lumen ships or shows, each with its licence (`yarn run licenses`). */
function Licences() {
  return <LicenceList />;
}

/** One line on what the models are doing: off, still to download, or how far they have got. */
export function mlLine(enabled: boolean, status: MlStatus | undefined): string {
  if (!enabled) return "Off: memories, music and slideshows use Lumen's own rules.";
  if (!status) return "";
  const missing = status.models.filter((m) => !m.ready).reduce((sum, m) => sum + m.bytes, 0);
  if (missing > 0) return `${(missing / 1e6).toFixed(1)} MB to download, fetched in the background.`;
  const people = status.people > 0 ? `, ${status.people.toLocaleString()} ${status.people === 1 ? "person" : "people"} found` : "";
  return `Ready: ${status.embedded.toLocaleString()} of ${status.photos.toLocaleString()} photos analysed${people}.`;
}

export const SECTIONS: Section[] = [
  { id: "appearance", label: "Appearance", intro: "How Lumen looks and moves.", icon: <IconPalette {...ICON} />, Body: Appearance },
  { id: "library", label: "Library", intro: "Where your photos live and what the grid shows.", icon: <IconLibraryPhoto {...ICON} />, Body: Library },
  { id: "search", label: "Search & dates", intro: "How dates are read when you type them and written wherever they are shown.", icon: <IconCalendar {...ICON} />, Body: SearchDates },
  { id: "storage", label: "Storage", intro: "Your disks and memory cards, how full they are, and what your library and Lumen take up.", icon: <IconDatabase {...ICON} />, Body: StorageSection, layout: "full" },
  { id: "backup", label: "Backup", intro: "A copy of the library on another drive, kept up to date.", icon: <IconCloudUpload {...ICON} />, Body: Backup },
  { id: "playback", label: "Playback & codecs", intro: "Videos, the helpers Lumen uses for them, and slideshow exports.", icon: <IconMovie {...ICON} />, Body: Playback },
  { id: "raw-formats", label: "RAW formats", intro: "How Lumen opens RAW files its own reader can't.", icon: <IconAperture {...ICON} />, Body: RawFormatsSection },
  { id: "map", label: "Map", intro: "What the Map draws and where it gets it from.", icon: <IconMap2 {...ICON} />, Body: MapSection },
  { id: "music", label: "Music", intro: "Music for slideshows from Jamendo's Creative Commons catalogue.", icon: <IconMusic {...ICON} />, Body: Music },
  { id: "smart", label: "Models", intro: "Models that run on this computer and never send your photos anywhere.", icon: <IconBrain {...ICON} />, Body: Smart },
  { id: "experimental", label: "Experimental", intro: "Models Lumen is still training. They run on this computer.", icon: <IconFlask {...ICON} />, Body: ExperimentalSection },
  {
    id: "ai-assistants",
    label: "AI assistants",
    intro: "Let an AI assistant such as Claude or Codex retouch the photos you open in Develop. Its edits are drafts you review; it may also set ratings, colour labels and flags when you ask.",
    icon: <IconSparkles {...ICON} />,
    Body: AiAssistantsSection,
  },
  {
    id: "external-apps",
    label: "External apps",
    intro: "The apps Lumen can hand a photo or video to.",
    icon: <IconExternalLink {...ICON} />,
    Body: ExternalAppsSection,
  },
  {
    id: "discord",
    label: "Discord status",
    intro: "Show “Competing in Lumen” in your Discord status. Off by default; it never includes anything about your photos.",
    icon: <IconBrandDiscord {...ICON} />,
    Body: DiscordSection,
  },
  {
    id: "plugins",
    label: "Plugins",
    intro: "Themes, presets, importers and tools that add to Lumen. Each one runs on this computer with only the access it asks for.",
    icon: <IconPuzzle {...ICON} />,
    Body: PluginsSection,
  },
  { id: "people", label: "People", intro: "People you removed and faces Lumen set aside.", icon: <IconUsers {...ICON} />, Body: People, layout: "tiles" },
  { id: "keyboard", label: "Keyboard", intro: "Every shortcut, by where it works.", icon: <IconKeyboard {...ICON} />, Body: Keyboard, layout: "full" },
  { id: "updates", label: "Updates", intro: "New versions of Lumen, downloaded in the background and installed when you close it.", icon: <IconRefresh {...ICON} />, Body: UpdatesSection },
  { id: "about", label: "About", intro: "This copy of Lumen.", icon: <IconInfoCircle {...ICON} />, Body: About },
  {
    id: "licences",
    label: "Licences",
    intro: "Lumen is built on the work of others: the programs, models, data and libraries it ships or uses, each with its licence.",
    icon: <IconLicense {...ICON} />,
    Body: Licences,
    layout: "full",
  },
];

/** The sidebar: six groups, each with the sections that show as its tabs (Round 18 B1). */
export const GROUPS: SectionGroup[] = [
  { id: "general", label: "General", icon: <IconPalette {...ICON} />, tabs: ["appearance", "search", "keyboard"] },
  { id: "library", label: "Library", icon: <IconLibraryPhoto {...ICON} />, tabs: ["library", "storage", "backup", "raw-formats", "playback"] },
  { id: "smart-features", label: "Smart features", icon: <IconBrain {...ICON} />, tabs: ["smart", "experimental", "people", "ai-assistants"] },
  { id: "connections", label: "Connections", icon: <IconMap2 {...ICON} />, tabs: ["map", "music", "external-apps", "discord"] },
  { id: "plugins", label: "Plugins", icon: <IconPuzzle {...ICON} />, tabs: ["plugins"] },
  { id: "about-lumen", label: "About", icon: <IconInfoCircle {...ICON} />, tabs: ["updates", "about", "licences"] },
];

/** The group a section belongs to. */
export const groupOf = (sectionId: string): SectionGroup | undefined => GROUPS.find((g) => g.tabs.includes(sectionId));

/** Each section's line in the list: its state at a glance. */
export function useSectionStatus(settings: Settings | undefined): Record<string, string | null> {
  const ffmpeg = useFfmpegStatus().status;
  const info = useQuery(appInfoQuery);
  const ml = useQuery(mlStatusQuery);
  const { removed } = useRemovedPeople();
  const backup = useBackup().status;
  const backingUp = useBackupRunning(backup);
  const raw = useQuery(rawFormatsQuery).data;
  const externalApps = useExternalApps().apps;
  const pluginThemes = usePluginThemes();
  if (!settings) return {};
  const a = settings.appearance;
  const theme = themeName(a.theme, pluginThemes);
  const smartOn = settings.smart.enabled ?? true;
  const missing = ml.data?.models.some((m) => !m.ready);
  return {
    appearance: `${theme}, ${a.scheme === "auto" ? "automatic" : a.scheme}`,
    library: settings.appearance.hideRawWithJpeg ? "RAW hidden beside JPEG" : null,
    backup: backupLine(backup, backingUp),
    playback: ffmpeg ? (ffmpeg.available ? "FFmpeg ready" : "FFmpeg missing") : null,
    search: settings.search?.dateOrder && settings.search.dateOrder !== "auto" ? ORDER_NAMES[settings.search.dateOrder] : "Automatic",
    "raw-formats": rawFormatsLine(raw),
    map: (settings.map.online ?? true) ? "Online detail" : "Offline outline",
    music: settings.music.jamendoClientId ? "Jamendo on" : "Off",
    smart: !smartOn ? "Off" : missing ? "Downloading models" : "On",
    experimental: smartOn ? experimentalStatus(settings) : "Off",
    "ai-assistants": settings.aiAssistants?.enabled ? "On" : "Off",
    "external-apps": externalApps.length ? externalAppsLine(externalApps) : null,
    discord: settings.discord?.enabled ? "On" : "Off",
    people: removed.length > 0 ? `${removed.length.toLocaleString()} removed` : null,
    keyboard: null,
    updates: settings.updates ? `${settings.updates.channel[0].toUpperCase()}${settings.updates.channel.slice(1)}${settings.updates.automatic ? ", automatic" : ""}` : null,
    about: info.data ? `Lumen ${info.data.version}` : null,
  };
}

/** The Experimental line in the list: which trained models are switched on. */
function experimentalStatus(settings: Settings): string {
  const ex = settings.smart.experimental;
  const on = [ex?.autoModel && "Ray", ex?.retouch && "Prism"].filter(Boolean);
  return on.length > 0 ? `${on.join(", ")} on` : "Off";
}

/** The Backup line in the list: not set up, running, needing a look, or when it last ran. */
export function backupLine(status: BackupStatus | undefined, running: boolean): string | null {
  if (!status) return null;
  const c = status.config;
  if (!c.destination) return "Not set up";
  if (running) return "Backing up";
  if (c.last?.error) return "Needs attention";
  if (!status.available) return "Drive not connected";
  return c.lastSuccessAt ? `Backed up ${formatRelative(c.lastSuccessAt)}` : "Not backed up yet";
}
