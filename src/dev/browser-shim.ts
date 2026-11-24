// Dev-only. Lets the UI run in a plain browser (`yarn dev` without Tauri) against an
// in-memory backend, so screens can be previewed and screenshotted without the host.
// Loaded from main.tsx only when `import.meta.env.DEV` and no Tauri runtime is present.
//
// Query params: ?library=1 (skip setup) ?theme=glass&scheme=light ?nosplash=1 ?items=200

import type { AssetDetail } from "@/types/generated/AssetDetail";
import type { LayoutRow } from "@/types/generated/LayoutRow";
import type { PluginInfo } from "@/types/generated/PluginInfo";
import type { ReviewItem } from "@/types/generated/ReviewItem";
import type { ReviewState } from "@/types/generated/ReviewState";
import type { LayoutSummary } from "@/types/generated/LayoutSummary";
import type { LibraryInfo } from "@/types/generated/LibraryInfo";
import type { LibraryListing } from "@/types/generated/LibraryListing";
import type { Section } from "@/types/generated/Section";
import type { SlideshowDoc } from "@/types/generated/SlideshowDoc";
import type { Settings } from "@/types/generated/Settings";
import type { Tile } from "@/types/generated/Tile";
import type { UpdateChannel } from "@/types/generated/UpdateChannel";
import type { PluginControl } from "@/types/generated/PluginControl";
import type { UpdateState } from "@/types/generated/UpdateState";
import "./shim-themes"; // Round 26 B2: ?shimtheme=top lists a theme that moves the sidebar to the top

const params = new URLSearchParams(window.location.search);
const LIB: LibraryInfo = { id: "dev-lib", name: "Lumen Library", path: "C:/Users/you/Pictures/Lumen Library", schemaVersion: 2 };

const shimPlugin = (id: string, name: string, kinds: PluginInfo["kinds"], trust: PluginInfo["trust"], dev = false): PluginInfo => ({
  id,
  name,
  version: "1.0.0",
  author: trust.type === "author" ? trust.name : "JL Studios",
  description: `${name} for the browser preview.`,
  license: "MIT",
  trust,
  kinds,
  permissions: kinds.includes("importer") ? ["importReadChosen"] : [],
  enabled: true,
  dev,
  state: { type: "ready" },
  iconUrl: null,
  settings: [],
  sections: [],
  contributions: [],
  homepage: null, authorUrl: null, repository: null, screenshots: [], hasReadme: false, detectIds: [], assets: [],
});
const shimControl = (kind: PluginControl["kind"], key: string | null, label: string): PluginControl => ({
  kind, key, label, help: null, value: null, choices: [], min: null, max: null, step: null, requires: null, app: null, action: null, body: null,
});
const shimPlugins: PluginInfo[] = [
  shimPlugin("studio.jl.nord", "Nord", ["presets"], { type: "jlStudios" }),
  shimPlugin("com.example.importer", "Example importer", ["importer"], { type: "author", name: "Ada Author", fingerprint: "ab12 cd34" }),
  shimPlugin("local.devtools", "Dev tools", ["tools"], { type: "unsigned" }, true),
  // Round 24 lane D: a plugin that needs an app (its page shows Locate… and a disabled switch).
  {
    ...shimPlugin("studio.jl.atlas", "Disk Atlas", [], { type: "jlStudios" }),
    homepage: "https://example.com/disk-atlas",
    detectIds: ["disk-atlas"],
    settings: [
      { key: "use_picker", label: "Use Disk Atlas for file dialogs", kind: "bool", value: true, choices: [], group: "Dialogs", requires: "disk-atlas" },
      { key: "use_reveal", label: "Use Disk Atlas for Show in folder", kind: "bool", value: true, choices: [], group: "Dialogs", requires: "disk-atlas" },
    ],
    sections: [
      {
        title: "Dialogs",
        description: null,
        controls: [
          { ...shimControl("toggle", "use_picker", "Use Disk Atlas for file dialogs"), value: true, app: "disk-atlas" },
          { ...shimControl("toggle", "use_reveal", "Use Disk Atlas for Show in folder"), value: true, app: "disk-atlas" },
        ],
      },
    ],
  },
  // Round 25 P3: every control kind, synthetic (browser preview only).
  {
    ...shimPlugin("local.sampler", "Settings sampler", ["tools"], { type: "unsigned" }, true),
    sections: [
      {
        title: "Scanning",
        description: "How a scan runs. Nothing here touches your photos.",
        controls: [
          { ...shimControl("toggle", "deep", "Deep scan"), value: false, help: "Reads every file instead of a sample." },
          { ...shimControl("slider", "depth", "Depth"), value: 3, min: 1, max: 8, step: 1, requires: "deep" },
          { ...shimControl("segmented", "unit", "Units"), value: "mb", choices: ["mb", "gb"] },
          { ...shimControl("select", "order", "Order"), value: "name", choices: ["name", "size", "date"] },
          { ...shimControl("number", "limit", "Largest files to list"), value: 50, min: 1, max: 500, step: 1 },
          { ...shimControl("text", "label", "Report title"), value: "", help: "Shown at the top of the report." },
        ],
      },
      {
        title: "Last run",
        description: null,
        controls: [
          { ...shimControl("markdown", null, "About"), body: "Reads sizes only. **No file is changed.**" },
          { ...shimControl("stat", "last_scan", "Last scan") },
          { ...shimControl("list", "biggest", "Largest folders") },
          { ...shimControl("button", null, "Scan now"), action: "scan", help: "Runs once, in the background." },
        ],
      },
    ],
  },
];

// Round 24 lane W: fake place suggestions for the Place review screen.
type ShimReviewItem = ReviewItem;
const shimMoments = [
  { id: "m1", label: "12 Mar 14:02–14:40 · Ljubljana", lat: 46.0569, lon: 14.5058 },
  { id: "m2", label: "12 Mar 17:10–18:00 · Bled", lat: 46.3683, lon: 14.1146 },
  { id: "m3", label: "13 Mar 09:00–11:30 · Piran", lat: 45.5283, lon: 13.5683 },
  { id: "m4", label: "14 Mar 12:00–12:20 · Unknown", lat: 45.9, lon: 13.65 },
];
const shimReviewItems: ShimReviewItem[] = Array.from({ length: 40 }, (_, n) => {
  const m = shimMoments[n % 4];
  const state: ReviewItem["state"] = n % 7 === 0 ? "guess" : n % 3 === 0 ? "inferred" : "anchor";
  return {
    asset: 1000 + n,
    group: m.id,
    state,
    flag: n % 11 === 0 ? "far-from-group" : null,
    candidates: [{ lat: m.lat + (n % 5) * 0.004, lon: m.lon + (n % 3) * 0.004, confidence: 0.4 + (n % 6) / 10, label: m.label.split(" · ")[1] }],
    status: "pending",
  };
});

// Round 25 P4b-2b: fake proposals for the generic review page, one plugin, every kind.
interface ShimProposal {
  proposal: { id: number; pluginId: string; assetId: number; proposed: unknown; confidence: number; reason: string | null; status: string };
  filename: string;
  current: unknown;
}
const shimProposals: ShimProposal[] = (() => {
  const mk = (id: number, kind: string, proposed: unknown, current: unknown, confidence: number, reason: string | null = null): ShimProposal => ({
    proposal: { id, pluginId: "local.sampler", assetId: 1 + (id % 12), proposed: { kind, value: proposed }, confidence, reason, status: "pending" },
    filename: `IMG_${4000 + id}.JPG`,
    current: current === null ? null : { kind, value: current },
  });
  const out: ShimProposal[] = [];
  const places = [
    [46.0569, 14.5058],
    [46.3683, 14.1146],
    [45.5283, 13.5683],
    [45.9, 13.65],
    [46.55, 15.65],
  ];
  places.forEach(([lat, lon], n) => out.push(mk(100 + n, "location", { lat, lon }, null, 0.55 + n * 0.1, n === 0 ? "The folder is called Ljubljana 2024" : null)));
  for (let n = 0; n < 5; n++) out.push(mk(200 + n, "date", `2021-05-0${n + 1}T1${n}:30:00`, n % 2 ? "2020-01-01T00:00:00" : null, 0.6 + n * 0.08, n === 0 ? "The file name has a date in it" : null));
  for (let n = 0; n < 5; n++) out.push(mk(300 + n, "tags", n % 2 ? ["beach", "sea"] : ["mountain", "snow", "winter"], n === 1 ? ["beach"] : null, 0.7 + n * 0.05));
  for (let n = 0; n < 5; n++) out.push(mk(400 + n, "label", ["Red", "Yellow", "Green", "Blue", "Purple"][n], n === 0 ? "Blue" : null, 0.5 + n * 0.1));
  for (let n = 0; n < 5; n++) out.push({ ...mk(500 + n, "person", { face: 7 + n, name: ["Ana", "Ben", "Cleo", "Dan", "Eva"][n] }, null, 0.6 + n * 0.08, n === 0 ? "Looks like the person in 12 other photos" : null), current: n === 2 ? { kind: "person", value: { face: 9, name: "Clea" } } : null });
  for (let n = 0; n < 5; n++) out.push(mk(600 + n, "title", ["Sunset over the bay", "Market day", "First snow", "Birthday dinner", "Harbour at dawn"][n], n === 0 ? "IMG_4001" : null, 0.65 + n * 0.07));
  for (let n = 0; n < 5; n++) out.push(mk(700 + n, "caption", ["A quiet evening on the water, the light going orange behind the hills.", "Crowded stalls and a lot of cheese.", "The first real snow of the year.", "Everyone around one long table.", "Fog on the water and a few boats."][n], null, 0.6 + n * 0.08));
  return out;
})();

function shimReview(): ReviewState {
  const live = shimReviewItems.filter((i) => i.status !== "rejected");
  const check = (i: ReviewItem) => i.status === "pending" && (i.state === "guess" || i.flag !== null);
  const groups = shimMoments
    .map((m) => ({ id: m.id, label: m.label, count: live.filter((i) => i.group === m.id).length, check: live.filter((i) => i.group === m.id && check(i)).length }))
    .filter((g) => g.count > 0);
  return {
    groups,
    items: shimReviewItems.map((i) => ({ ...i })),
    counts: {
      check: live.filter(check).length,
      moments: groups.length,
      done: live.filter((i) => i.status === "confirmed" || (i.status === "pending" && !check(i))).length,
      rejected: shimReviewItems.length - live.length,
    },
  };
}

const state: { settings: Settings; library: LibraryInfo | null; listing: LibraryListing } = {
  settings: {
    version: 1,
    appearance: { theme: "gallery", scheme: "auto", tileSize: "m", libraryGroupBy: "month", hideRawWithJpeg: false, reduceMotion: false, sidebarCollapsed: false },
    import: { duplicates: "skip", promptOnVolume: true },
    playback: { loopVideos: false, muteByDefault: false },
    map: { online: true, style: "map" },
    music: { jamendoClientId: null },
    smart: { enabled: true, gpu: true, experimental: { autoModel: false, retouch: false, readText: false, aiMasks: true, tasks: { auto: { model: null, tier: "auto" }, retouch: { model: null, tier: "auto" } } } },
    export: { watermark: true },
    search: { dateOrder: "auto" },
    window: { width: 1280, height: 800, x: null, y: null, maximized: false },
    aiAssistants: { enabled: false, port: 47820, token: "" },
    updates: { automatic: params.get("automatic") !== "0", channel: (["alpha", "beta"].includes(params.get("channel") ?? "") ? params.get("channel") : "stable") as UpdateChannel, security: params.get("security") !== "0" },
    discord: { enabled: false, showActivity: true, showElapsed: true },
    externalApps: { custom: [], defaults: { photo: "photoshop", raw: "photoshop", video: null }, send: {} }, // Round 22 lane C
    exactPlaces: false,
    keybinds: {},
  },
  library: null,
  listing: { libraries: [], current: null },
};

/**
 * The updater as the browser plays it (Round 17): `?update=<state>` starts it in any state
 * (`ready`, `available`, `downloading`, `failed`, `off`), otherwise up to date; *Check* then finds
 * 26.1.1-alpha on Beta or Alpha.
 */
const NOTES = "Faster grid scrolling.\nThe map remembers where you were.";
const updateStart: Record<string, UpdateState> = {
  ready: { state: "ready", version: "26.1.1-alpha", notes: NOTES, attempts: 0 },
  available: { state: "available", version: "26.1.1-alpha", notes: NOTES },
  downloading: { state: "downloading", version: "26.1.1-alpha", done: 42, total: 100 },
  failed: { state: "failed", why: "offline", at: Math.floor(Date.now() / 1000) - 600 },
  off: { state: "off" },
};
let updateState: UpdateState = updateStart[params.get("update") ?? ""] ?? { state: "upToDate", checkedAt: Math.floor(Date.now() / 1000) - 3600 };
/** `?update=ready&fix=1` shows the staged security fix's banner. */
const updateStatus = () => ({ version: "26.1.0-alpha", state: updateState, whatsNew: null, security: updateState.state === "ready" && params.get("fix") === "1" });
/** The screen the page last reported to the Discord status (Round 17). */
let shimScreen = "browsing";

/** A stand-in MCP key, 64 hex characters like Rust's. */
const shimToken = () => Array.from({ length: 64 }, () => Math.floor(Math.random() * 16).toString(16)).join("");

if (params.has("theme")) {
  state.settings.appearance.theme = params.get("theme") as Settings["appearance"]["theme"];
  state.settings.appearance.scheme = (params.get("scheme") as Settings["appearance"]["scheme"]) ?? "auto";
}
if (params.has("nosplash")) {
  (window as unknown as { __LUMEN_NO_SPLASH__: boolean }).__LUMEN_NO_SPLASH__ = true;
}

function adopt(path: string): LibraryInfo {
  const name = path.split(/[\\/]/).filter(Boolean).pop() ?? "Library";
  state.library = { ...LIB, name, path };
  state.listing = {
    libraries: [
      { entry: { id: LIB.id, name, path, lastOpenedAt: Date.now() / 1000 }, available: true },
      { entry: { id: "ext-lib", name: "Travel SSD", path: "E:/Lumen Library", lastOpenedAt: Date.now() / 1000 - 86400 * 3 }, available: false },
    ],
    current: LIB.id,
  };
  return state.library;
}
if (params.has("library")) adopt(LIB.path);

// ---- fake catalogue -------------------------------------------------------------

// `?jamendo=1` pretends a client id is set, so the music browser's second tab can be seen
// without one. The shim answers the search itself; nothing here reaches the network.
if (params.get("jamendo")) {
  state.settings.music.jamendoClientId = "shim-client-id";
}
if (params.get("tiles")) {
  state.settings.appearance.tileSize = params.get("tiles") as Settings["appearance"]["tileSize"];
}
if (params.get("group")) {
  state.settings.appearance.libraryGroupBy = params.get("group") as Settings["appearance"]["libraryGroupBy"];
}
const N = Number(params.get("items") ?? 120);
const seed = (n: number) => {
  const x = Math.sin(n * 9973) * 10000;
  return x - Math.floor(x);
};
const NOW = 1_758_000_000;
interface Item {
  id: number;
  aspect: number;
  ts: number;
  video: boolean;
}
/** Every ninth still is a NEF and every thirteenth a Live Photo, so badges and collections show. */
/** A flat square standing in for an app's icon (Round 22 lane C). */
const appIcon = (color: string) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" rx="3" fill="${color}"/></svg>`)}`;
const isRaw = (it: Item) => !it.video && it.id % 9 === 0;
const isLive = (it: Item) => !it.video && !isRaw(it) && it.id % 13 === 0;
const extOf = (it: Item) => (it.video ? "mp4" : isRaw(it) ? "nef" : isLive(it) ? "heic" : "jpg");
const mediaTypeOf = (it: Item) => (it.video ? 1 : isLive(it) ? 2 : 0);
type Kind = "live" | "raw" | "hdr" | "screenshot" | "panorama" | "selfie";
const hasKind = (it: Item, k: Kind | null | undefined) =>
  !k ||
  (k === "live" && isLive(it)) ||
  (k === "raw" && isRaw(it)) ||
  (k === "hdr" && it.id % 6 === 0) ||
  (k === "screenshot" && it.id % 17 === 0) ||
  (k === "panorama" && it.aspect >= 1.777 && it.id % 4 === 0) ||
  (k === "selfie" && it.aspect < 1 && it.id % 5 === 0);
const ASPECTS = [1.5, 0.75, 1.333, 1, 1.777, 0.66];
const items: Item[] = Array.from({ length: N }, (_, i) => ({
  id: i + 1,
  aspect: ASPECTS[Math.floor(seed(i + 1) * ASPECTS.length)],
  ts: NOW - Math.floor(seed(i + 7) * 120) * 86_400 - i * 60,
  video: seed(i + 3) < 0.08,
})).sort((a, b) => b.ts - a.ts);

const shimResolved = new Set<number>();
const shimDismissed = new Set<string>();
/** Every seventh still with the one after it, plus a burst of four: enough to see the screen. */
function shimDuplicateSets() {
  const stills = items.filter((i) => !i.video);
  const member = (it: Item, i: number) => ({
    id: it.id,
    width: it.aspect >= 1 ? 4032 : Math.round(4032 * it.aspect),
    height: it.aspect >= 1 ? Math.round(4032 / it.aspect) : 4032,
    bytes: 1_800_000 + i * 90_000,
    capturedAt: it.ts,
    sharpness: 0.3 + 0.1 * i,
    rating: it.id % 5 === 0 ? 3 : 0,
    favorite: it.id % 11 === 0,
    edited: false,
    albums: 0,
    format: extOf(it).toUpperCase(),
  });
  const groups: Item[][] = [stills.slice(2, 6)];
  for (let i = 10; i + 1 < stills.length && groups.length < 9; i += 7) groups.push([stills[i], stills[i + 1]]);
  return groups
    .map((g, n) => {
      const members = g.filter((it) => !shimResolved.has(it.id)).map(member);
      const key = members.map((m) => m.id).sort((a, b) => a - b).join(",");
      return { key, likeness: n % 3 === 1 ? "exact" : "similar", members, keep: members[0]?.id ?? 0 };
    })
    .filter((s) => s.members.length > 1 && !shimDismissed.has(s.members.map((m) => m.id).join(",")));
}

const monthOf = (ts: number) => {
  const d = new Date(ts * 1000);
  return `${d.toLocaleString(undefined, { month: "long" })} ${d.getFullYear()}`;
};

/** Section titles as photos-core::layout writes them for each grouping. */
const sectionOf = (ts: number, groupBy: GroupBy) => {
  const d = new Date(ts * 1000);
  if (groupBy === "year") return String(d.getFullYear());
  if (groupBy === "day")
    return `${d.toLocaleString("en-GB", { weekday: "long" })}, ${d.getDate()} ${d.toLocaleString("en-GB", { month: "long" })} ${d.getFullYear()}`;
  return monthOf(ts);
};

/** Flags and colour labels, seeded so a cull is visible on first load and changed by the setters. */
const marks = new Map<number, { label: number; flag: number }>();
const markOf = (id: number) => marks.get(id) ?? { label: id % 17 === 0 ? 1 : id % 10 === 0 ? 3 : id % 23 === 0 ? 4 : 0, flag: id % 9 === 0 ? 1 : id % 13 === 0 ? -1 : 0 };
function setMark(ids: number[], field: "label" | "flag", value: number) {
  const changed = [];
  for (const id of ids) {
    const m = markOf(id);
    if (m[field] === value) continue;
    changed.push({ id, previous: m[field] });
    marks.set(id, { ...m, [field]: value });
  }
  layoutCache.clear();
  return changed;
}

function layout(width: number, rowHeight: number, gap: number, mediaType: number | null, favorites = false, trash = false, albumId: number | null = null, kind: Kind | null = null, groupBy: GroupBy = "month", labels: number[] = [], flag: number | null = null) {
  const rows: LayoutRow[] = [];
  const sections: Section[] = [];
  let list = items.filter((i) => mediaType === null || (mediaType === 1) === i.video);
  if (favorites) list = list.filter((i) => i.id % 7 === 0);
  if (trash) list = list.filter((i) => i.id % 11 === 0);
  if (albumId !== null) list = list.filter((i) => i.id % (albumId + 2) === 0);
  if (kind) list = list.filter((i) => hasKind(i, kind));
  if (labels.length) list = list.filter((i) => labels.includes(markOf(i.id).label));
  if (flag != null) list = list.filter((i) => markOf(i.id).flag === flag);
  let y = 0;
  let i = 0;
  const monthOf = (ts: number) => sectionOf(ts, groupBy);
  const tileOf = (it: Item, x: number, w: number, h: number): Tile => ({
    id: it.id,
    x: Math.round(x),
    w: Math.round(w),
    h,
    thumbhash: null,
    color: `hsl(${(it.id * 47) % 360} 30% 60%)`,
    mediaType: mediaTypeOf(it),
    durationMs: it.video ? 12_000 + it.id * 300 : null,
    renderable: !it.video,
    extension: extOf(it),
    isFavorite: it.id % 7 === 0,
    trashedAt: trash ? NOW - 86_400 * (it.id % 29) : null,
    rating: it.id % 11 === 0 ? 5 : it.id % 5 === 0 ? 3 : 0,
    hdr: it.id % 6 === 0 ? 1 : 0,
    raw: isRaw(it),
    colorLabel: markOf(it.id).label,
    flag: markOf(it.id).flag,
    // Round 22 lane C: some photos have a copy made for Photoshop.
    ...(!it.video && it.id % 17 === 0 ? { editedIn: "photoshop" } : {}),
  });
  while (i < list.length) {
    const title = monthOf(list[i].ts);
    const year = new Date(list[i].ts * 1000).getFullYear();
    const month = new Date(list[i].ts * 1000).getMonth() + 1;
    const firstRow = rows.length;
    const sectionY = y;
    rows.push({ index: rows.length, kind: "header", y, height: 44, section: sections.length, title, tiles: [] });
    y += 44;
    let count = 0;
    // A year is a square grid, as in photos-core::layout.
    if (groupBy === "year") {
      const cols = Math.max(1, Math.floor((width + gap) / (rowHeight + gap)));
      const side = (width - (cols - 1) * gap) / cols;
      const si = Math.round(side);
      while (i < list.length && monthOf(list[i].ts) === title) {
        const start = i;
        while (i < list.length && monthOf(list[i].ts) === title && i - start < cols) i++;
        const tiles = list.slice(start, i).map((it, k) => tileOf(it, k * (side + gap), si, si));
        rows.push({ index: rows.length, kind: "tiles", y, height: si, section: sections.length, title: null, tiles });
        y += si + gap;
        count += i - start;
      }
    }
    while (i < list.length && monthOf(list[i].ts) === title) {
      const start = i;
      let sum = 0;
      while (i < list.length && monthOf(list[i].ts) === title) {
        const next = sum + list[i].aspect;
        if (start !== i && next * rowHeight + (i - start) * gap > width) break;
        sum = next;
        i++;
      }
      // Mirrors photos-core::layout: taking the item that would overflow often lands
      // closer to the target than stopping before it, so rows sit either side of it.
      if (i < list.length && monthOf(list[i].ts) === title) {
        const rowH = (count: number, total: number) => (width - Math.max(0, count - 1) * gap) / total;
        const without = rowH(i - start, sum);
        const withNext = rowH(i - start + 1, sum + list[i].aspect);
        if (Math.abs(withNext - rowHeight) < Math.abs(without - rowHeight)) {
          sum += list[i].aspect;
          i++;
        }
      }
      const n = i - start;
      const last = !(i < list.length && monthOf(list[i].ts) === title);
      let h = (width - (n - 1) * gap) / sum;
      const stretch = !(last && h > rowHeight * 1.35);
      h = stretch ? Math.min(Math.max(h, rowHeight * 0.7), rowHeight * 1.35) : rowHeight;
      const hi = Math.round(h);
      let x = 0;
      const tiles: Tile[] = list.slice(start, i).map((it, k) => {
        let w = it.aspect * h;
        if (stretch && k === n - 1) w = width - x;
        const t = tileOf(it, x, w, hi);
        x += w + gap;
        return t;
      });
      rows.push({ index: rows.length, kind: "tiles", y, height: hi, section: sections.length, title: null, tiles });
      y += hi + gap;
      count += n;
    }
    sections.push({ index: sections.length, year, month, title, y: sectionY, height: y - sectionY, firstRow, rowCount: rows.length - firstRow, itemCount: count, albumId: null });
  }
  const summary: LayoutSummary = { totalHeight: Math.max(0, y - gap), itemCount: list.length, rowHeights: rows.map((r) => r.height), sections };
  return { summary, rows };
}

const layoutCache = new Map<string, ReturnType<typeof layout>>();

/** A collection's photos by album (Round 19 §10): one titled section per album, a photo in two albums in both. */
function collectionLayout(width: number, rowHeight: number, gap: number, collectionId: number) {
  const inside = new Set([collectionId]);
  for (let grew = true; grew; ) {
    grew = false;
    for (const c of albumCollections) if (c.parentId != null && inside.has(c.parentId) && !inside.has(c.id)) (inside.add(c.id), (grew = true));
  }
  const rows: LayoutRow[] = [];
  const sections: Section[] = [];
  let y = 0;
  let count = 0;
  for (const al of albums.filter((x) => x.kind === 0 && x.collectionId != null && inside.has(x.collectionId))) {
    const tileRows = layout(width, rowHeight, gap, null, false, false, al.id).rows.filter((r) => r.kind === "tiles");
    if (tileRows.length === 0) continue;
    const sectionY = y;
    const first = rows.length;
    rows.push({ index: first, kind: "header", y, height: 44, section: sections.length, title: al.name, tiles: [] });
    y += 44;
    for (const r of tileRows) {
      rows.push({ ...r, index: rows.length, y, section: sections.length });
      y += r.height + gap;
      count += r.tiles.length;
    }
    sections.push({ index: sections.length, year: 0, month: 1, title: al.name, y: sectionY, height: y - sectionY, firstRow: first, rowCount: rows.length - first, itemCount: tileRows.reduce((n, r) => n + r.tiles.length, 0), albumId: al.id });
  }
  const summary: LayoutSummary = { totalHeight: Math.max(0, y - gap), itemCount: count, rowHeights: rows.map((r) => r.height), sections };
  return { summary, rows };
}

const layoutFor = (a: Record<string, unknown>) => {
  const collectionId = (a.scope as { collectionId?: number | null } | null)?.collectionId;
  if (collectionId != null) {
    const key = `c${collectionId}:${a.width}:${a.rowHeight}:${a.gap}`;
    if (!layoutCache.has(key)) layoutCache.set(key, collectionLayout(Number(a.width), Number(a.rowHeight), Number(a.gap), collectionId));
    return layoutCache.get(key)!;
  }
  const scope =
    (a.scope as {
      mediaType: number | null;
      favorites?: boolean;
      trash?: boolean;
      albumId?: number | null;
      query?: { kind?: Kind | null; favorites?: boolean; mediaType?: number | null; colorLabels?: number[]; flag?: number | null } | null;
    } | null) ?? null;
  // The query's kind, favorites and media type filter too, so collection cards work here.
  const filter = scope?.mediaType ?? scope?.query?.mediaType ?? null;
  const favorites = !!scope?.favorites || !!scope?.query?.favorites;
  const kind = scope?.query?.kind ?? null;
  const groupBy = (a.groupBy as GroupBy | undefined) ?? "month";
  const labels = scope?.query?.colorLabels ?? [];
  const flag = scope?.query?.flag ?? null;
  const key = `${a.width}:${a.rowHeight}:${a.gap}:${filter}:${favorites ? 1 : 0}:${scope?.trash ? 1 : 0}:${scope?.albumId ?? ""}:${kind ?? ""}:${groupBy}:${labels.join(",")}:${flag ?? ""}`;
  if (!layoutCache.has(key))
    layoutCache.set(key, layout(Number(a.width), Number(a.rowHeight), Number(a.gap), filter, favorites, !!scope?.trash, scope?.albumId ?? null, kind, groupBy, labels, flag));
  return layoutCache.get(key)!;
};

const detail = (id: number): AssetDetail => {
  const it = items.find((i) => i.id === id) ?? items[0];
  const ext = extOf(it);
  return {
    id,
    filename: `${isRaw(it) ? "DSC" : "IMG"}_${String(id).padStart(4, "0")}.${ext}`,
    extension: ext,
    mediaType: mediaTypeOf(it),
    byteSize: 3_200_000 + id * 1000,
    width: Math.round(4000 * (it.aspect > 1 ? 1 : it.aspect)),
    height: Math.round(4000 * (it.aspect > 1 ? 1 / it.aspect : 1)),
    orientation: 1,
    capturedAt: it.ts,
    importedAt: NOW,
    durationMs: it.video ? 12_000 + id * 300 : null,
    deviceName: id % 3 ? "iPhone 17 Pro" : "Sony ILCE-7M4",
    gpsLat: id % 2 ? 46.05 : null,
    gpsLon: id % 2 ? 14.5 : null,
    locality: id % 2 ? "Idrija, Slovenia" : null,
    placeName: id % 2 ? "Idrija, Slovenia" : null,
    placeSource: id % 2,
    isFavorite: false,
    rating: id % 7 === 0 ? 5 : id % 5 === 0 ? 4 : 0,
    hdr: id % 6 === 0 ? 1 : 0,
    relativePath: `2026/09/17/${id}.${ext}`,
    contentHash: "deadbeef".repeat(8),
    metadata: { exposure: "1/250", aperture: 2.8, iso: 200, focal_length_mm: 35, lens: "FE 35mm F1.4 GM" },
    renderable: !it.video,
    raw: isRaw(it),
    liveVideoId: isLive(it) ? 100_000 + id : null,
    colorLabel: markOf(id).label,
    flag: markOf(id).flag,
    // Every eleventh photo pretends to be an export added back to the library.
    derivedFrom: id % 11 === 0 && id > 1 ? { id: id - 1, filename: `IMG_${String(id - 1).padStart(4, "0")}.jpg` } : null,
  };
};

import type { AlbumSummary } from "@/types/generated/AlbumSummary";
import type { AlbumCollection } from "@/types/generated/AlbumCollection";
import type { CollectionUndo } from "@/types/generated/CollectionUndo";
import type { BackupStatus } from "@/types/generated/BackupStatus";
import type { GroupBy } from "@/types/generated/GroupBy";

const shimVault = { password: "lumen" as string | null, unlocked: false, ids: new Set<number>([3, 7, 11]) };

const albums: AlbumSummary[] = [
  { id: 1, name: "Trip to Lisbon", kind: 0, parentId: null, hidesItems: false, coverAssetId: 2, itemCount: 34, createdAt: NOW - 86400, updatedAt: NOW - 3600, query: null, collectionId: 1, coverStyle: null, previewIds: [2, 3, 4] },
  { id: 2, name: "Family", kind: 0, parentId: null, hidesItems: false, coverAssetId: 9, itemCount: 120, createdAt: NOW - 86400 * 9, updatedAt: NOW - 86400 * 2, query: null, collectionId: 1, coverStyle: { kind: "icon", shape: "book", icon: "users", color: "swatch-4" }, previewIds: [9, 10, 11] },
  {
    id: 3,
    name: "Best videos",
    kind: 1,
    parentId: null,
    hidesItems: false,
    coverAssetId: 5,
    itemCount: 18,
    createdAt: NOW - 86400 * 3,
    updatedAt: NOW - 86400,
    query: { text: null, dateFrom: null, dateTo: null, deviceIds: [], mediaType: 1, favorites: false, minRating: 4, hasGps: null, hasText: false, noText: false, inText: [], camera: null, lens: null, kind: null, extension: null, includeRawPairs: false, colorLabels: [], flag: null, personIds: [], petIds: [], places: [], tags: [], months: [], albumIds: [], collectionIds: [], anyOf: [], searchTags: [] },
    collectionId: null,
    coverStyle: null,
    previewIds: [5, 6],
  },
];

const shimBackup: BackupStatus = {
  config: {
    destination: "E:/",
    schedule: "weekly",
    lastSuccessAt: NOW - 86400,
    last: { at: NOW - 86400, error: null, report: { copied: 42, copiedBytes: 1.3e9, unchanged: 5100, retired: 2, failed: [], cancelled: false } },
  },
  target: "E:/Lumen Library",
  available: true,
  running: false,
};

// Collections (Round 5A): Trips holds Lisbon and a nested Portugal collection.
const albumCollections: AlbumCollection[] = [
  { id: 1, name: "Trips", parentId: null, createdAt: NOW - 86400 * 4, updatedAt: NOW - 86400, coverStyle: null },
  { id: 2, name: "Portugal", parentId: 1, createdAt: NOW - 86400 * 4, updatedAt: NOW - 86400, coverStyle: null },
];

// Round 20 lane B: albums inside the nested Portugal collection and a few loose ones, so *Add to
// album* has more than six albums (Recent) and a tree three levels deep.
for (const [id, name, collectionId, cover] of [
  [20, "Porto", 2, 12],
  [21, "Sintra", 2, 13],
  [22, "School work", null, 14],
  [23, "To edit", null, 15],
  [24, "Birthday", null, 16],
  [25, "Garden", null, 17],
] as const) {
  albums.push({ id, name, kind: 0, parentId: null, hidesItems: false, coverAssetId: cover, itemCount: id * 3, createdAt: NOW - 86400 * 5, updatedAt: NOW - 86400, query: null, collectionId, coverStyle: null, previewIds: [cover] });
}

const batches = [
  { id: 2, sourceKind: 0, sourceLabel: "Screenshots", sourcePath: "C:/Users/you/Pictures/Screenshots", startedAt: NOW - 3600, finishedAt: NOW - 3400, itemCount: 80, skippedCount: 4, failedCount: 1, state: 1, skipCount: 0 },
  { id: 1, sourceKind: 0, sourceLabel: "DCIM", sourcePath: "D:/DCIM", startedAt: NOW - 86400 * 2, finishedAt: NOW - 86400 * 2 + 900, itemCount: 40, skippedCount: 0, failedCount: 0, state: 1, skipCount: 0 },
];

// Three memories over the fake catalogue: one of each kind.
const shimMemories = [
  { id: 1, kind: 2, title: "On this day", subtitle: "2024 · 2 years ago", startAt: 1_710_230_400, endAt: 1_710_266_400, coverAssetId: 5, itemCount: 14, curatedCount: 8, hidden: false },
  { id: 2, kind: 0, title: "Trip to Lisbon", subtitle: "12–19 March 2024", startAt: 1_710_230_400, endAt: 1_710_835_200, coverAssetId: 42, itemCount: 186, curatedCount: 40, hidden: false },
  { id: 3, kind: 1, title: "Ljubljana", subtitle: "Saturday, 4 May 2024", startAt: 1_714_816_800, endAt: 1_714_845_600, coverAssetId: 77, itemCount: 23, curatedCount: 8, hidden: false },
];

// Slideshows over the fake catalogue, built like `SlideshowDoc::new_for`.
const PANS: [number, number][] = [
  [0.02, 0.01],
  [-0.02, 0.01],
  [0.015, -0.01],
  [-0.01, -0.015],
];

const shimDoc = (assetIds: number[]): SlideshowDoc => ({
  version: 1,
  resolution: [1920, 1080],
  fps: 30,
  audio: null,
  clips: assetIds.map((assetId, i) => {
    const [dx, dy] = PANS[i % PANS.length];
    const inwards = i % 2 === 0;
    return {
      asset_id: assetId,
      extra: [],
      frame: "fill",
      duration_ms: 4000,
      transition: { kind: "crossfade", duration_ms: 600 },
      motion: { kind: "ken_burns", from: inwards ? [0, 0, 1] : [dx, dy, 1.12], to: inwards ? [dx, dy, 1.12] : [0, 0, 1] },
    };
  }),
});

const shimShows = [{ id: 1, name: "Trip to Lisbon", doc: shimDoc(items.slice(0, 8).map((i) => i.id)), updatedAt: NOW - 3600 * 30 }];
// The second show demonstrates the frames: a fill, a blurred portrait and a three-up grid.
shimShows.push({
  id: 9,
  name: "Portraits",
  doc: (() => {
    // Real shapes from the fake catalogue, so every frame is worth looking at.
    const tall = items.filter((i) => i.aspect < 1).slice(0, 8);
    const wide = items.filter((i) => i.aspect >= 1.4).slice(0, 3);
    const fallback = items[0].id;
    const at = (list: typeof items, n: number) => list[n]?.id ?? fallback;
    const clip = (id: number) => shimDoc([id]).clips[0];
    return {
      ...shimDoc([]),
      clips: [
        { ...clip(at(tall, 0)), frame: "blur" as const, transition: { kind: "wipe" as const, duration_ms: 700 } },
        {
          ...clip(at(tall, 1)),
          frame: "grid" as const,
          extra: [at(tall, 2), at(tall, 3)],
          transition: { kind: "slices" as const, duration_ms: 700 },
        },
        {
          ...clip(at(tall, 4)),
          frame: "quad" as const,
          extra: [at(tall, 5), at(tall, 6), at(tall, 7)],
          transition: { kind: "vslices" as const, duration_ms: 700 },
        },
        {
          ...clip(at(wide, 0)),
          frame: "feature" as const,
          extra: [at(tall, 0), at(tall, 1)],
          transition: { kind: "corner" as const, duration_ms: 700 },
        },
        {
          ...clip(at(wide, 1)),
          frame: "stack" as const,
          extra: [at(wide, 2)],
          transition: { kind: "vertopen" as const, duration_ms: 700 },
        },
        { ...clip(at(wide, 0)), frame: "fill" as const },
      ],
    };
  })(),
  updatedAt: NOW - 600,
});
let nextShowId = 2;

const summarize = (s: (typeof shimShows)[number]) => ({
  id: s.id,
  name: s.name,
  clipCount: s.doc.clips.length,
  durationMs: s.doc.clips.reduce((t, c, i) => t + c.duration_ms - (i < s.doc.clips.length - 1 ? Math.min(c.transition.duration_ms, c.duration_ms / 2) : 0), 0),
  coverAssetId: s.doc.clips[0]?.asset_id ?? null,
  updatedAt: s.updatedAt,
});

// Three Creative Commons tracks, so the music picker has something to show offline.
const shimMusic = [
  { id: "1886711", title: "Sunny Afternoon", artist: "Jasmine Jordan", durationMs: 184_000, licenceName: "CC BY 3.0", licenceUrl: "http://creativecommons.org/licenses/by/3.0/", shareUrl: "https://www.jamendo.com/track/1886711", previewUrl: "", downloadUrl: "", tags: ["happy", "pop"], derivativesAllowed: true },
  { id: "1755327", title: "Lantern Song", artist: "Marta Vidal", durationMs: 232_000, licenceName: "CC BY-SA 4.0", licenceUrl: "http://creativecommons.org/licenses/by-sa/4.0/", shareUrl: "https://www.jamendo.com/track/1755327", previewUrl: "", downloadUrl: "", tags: ["dreamy", "folk"], derivativesAllowed: true },
  { id: "1421772", title: "Long Way Home", artist: "Kite Season", durationMs: 205_000, licenceName: "CC BY 4.0", licenceUrl: "http://creativecommons.org/licenses/by/4.0/", shareUrl: "https://www.jamendo.com/track/1421772", previewUrl: "", downloadUrl: "", tags: ["calm", "ambient"], derivativesAllowed: true },
];
type ShimSaved = { id: string; title: string; artist: string; durationMs: number; licenceName: string; licenceUrl: string; shareUrl: string; credit: string; path: string };
/** Two of them already saved, so the picker's first tab has something to show. */
const shimSaved: ShimSaved[] = [shimMusic[0], shimMusic[2]].map((t) => ({
  ...t,
  credit: `“${t.title}” by ${t.artist} — ${t.licenceName} — ${t.shareUrl}`,
  path: `C:/Lumen Library/music/${t.id}.mp3`,
}));

/** Develop's filmstrip over the fake catalogue: photos only, the mark filters, a sort. */
const ratingOf = (id: number) => (id % 7 === 0 ? 5 : id % 5 === 0 ? 4 : 0);
type StripArgs = { scope?: { query?: { flag?: number | null; colorLabels?: number[]; minRating?: number | null } | null }; only?: number[] | null; sort?: string };
function stripItems(a: StripArgs) {
  const q = a.scope?.query;
  let pool = items.filter((i) => !i.video);
  if (a.only) {
    const set = new Set(a.only);
    pool = pool.filter((i) => set.has(i.id));
  }
  if (q?.flag != null) pool = pool.filter((i) => markOf(i.id).flag === q.flag);
  if (q?.colorLabels?.length) pool = pool.filter((i) => q.colorLabels!.includes(markOf(i.id).label));
  if (q?.minRating) pool = pool.filter((i) => ratingOf(i.id) >= q.minRating!);
  const name = (i: Item) => `${isRaw(i) ? "DSC" : "IMG"}_${String(i.id).padStart(4, "0")}`;
  const sorted = [...pool];
  if (a.sort === "rating") sorted.sort((x, y) => ratingOf(y.id) - ratingOf(x.id) || x.ts - y.ts || x.id - y.id);
  else if (a.sort === "filename") sorted.sort((x, y) => name(x).localeCompare(name(y)) || x.id - y.id);
  else sorted.sort((x, y) => x.ts - y.ts || x.id - y.id);
  return sorted.map((i) => ({
    id: i.id,
    filename: `${name(i)}.${extOf(i)}`,
    flag: markOf(i.id).flag,
    colorLabel: markOf(i.id).label,
    rating: ratingOf(i.id),
    edited: i.id % 4 === 0,
    width: Math.round(1000 * Math.min(1, i.aspect)),
    height: Math.round(1000 * Math.min(1, 1 / i.aspect)),
  }));
}

/** A plausible histogram for a photo: a lumpy midtone hill, a little different per id. */
function shimHistogram(id: number) {
  const bins = (shift: number) => Array.from({ length: 256 }, (_, x) => Math.round(4000 * Math.exp(-(((x - 110 - shift - (id % 40)) / 55) ** 2)) + 600 * Math.exp(-(((x - 30) / 18) ** 2))));
  return { r: bins(12), g: bins(0), b: bins(-14), luma: bins(0), clippedShadows: 0, clippedHighlights: id % 3 === 0 ? 120 : 0 };
}

const shimHistory = new Map<number, { id: number; label: string; recipe: unknown; snapshotName: string | null; createdAt: number }[]>();
let shimStep = 1;
const shimUserPresets: { id: number; name: string; develop: unknown; panels: string[]; builtin: boolean }[] = [];


/** The built-ins as Rust defines them, abridged: enough for the panel to show and hover. */
function shimBuiltinPresets() {
  const d = (basic: Record<string, number>) => ({ basic: { temperature: 0, tint: 0, exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, texture: 0, clarity: 0, dehaze: 0, vibrance: 0, saturation: 0, ...basic } });
  return [
    { id: -1, key: "builtin:1", source: "builtin", name: "Natural", develop: d({ contrast: 0.1, clarity: 0.1, vibrance: 0.15 }), panels: ["basic"], builtin: true },
    { id: -2, key: "builtin:2", source: "builtin", name: "Vivid", develop: d({ contrast: 0.25, clarity: 0.15, dehaze: 0.05, vibrance: 0.4, saturation: 0.1 }), panels: ["basic"], builtin: true },
    { id: -3, key: "builtin:3", source: "builtin", name: "Soft film", develop: d({ contrast: -0.15, highlights: -0.2, shadows: 0.15, blacks: 0.15, vibrance: -0.1 }), panels: ["basic"], builtin: true },
    { id: -4, key: "builtin:4", source: "builtin", name: "B&W", develop: d({ saturation: -1, contrast: 0.2, clarity: 0.15 }), panels: ["basic"], builtin: true },
    { id: -5, key: "builtin:5", source: "builtin", name: "Warm evening", develop: d({ temperature: 25, tint: 5, highlights: -0.15, vibrance: 0.15 }), panels: ["basic"], builtin: true },
    { id: -6, key: "builtin:6", source: "builtin", name: "Cool morning", develop: d({ temperature: -20, tint: -3, exposure: 0.1, contrast: -0.05, shadows: 0.1 }), panels: ["basic"], builtin: true },
  ];
}
type DeviceFilter = { from: number | null; to: number | null; kinds: string[]; newOnly: boolean; includeUndated: boolean };
const DEVICE_END = Math.floor(Date.UTC(2026, 8, 30, 18) / 1000);
const deviceItems = Array.from({ length: 6000 }, (_, i) => {
  const live = i % 12 === 5;
  const undated = i % 997 === 996;
  return {
    index: i,
    token: String(items[i % items.length].id),
    fileName: `IMG_${String(9999 - i).padStart(4, "0")}.${live ? "HEIC" : "CR3"}`,
    size: live ? 6_000_000 : 24_000_000,
    files: live ? 2 : 1,
    date: undated ? null : DEVICE_END - Math.floor(i / 40) * 86_400 - (i % 40) * 600,
    dateSource: undated ? null : "exif",
    kind: live ? "live" : i % 9 === 0 ? "video" : "raw",
    imported: i >= 5500,
  };
});
const deviceMatches = (f: DeviceFilter) =>
  deviceItems.filter(
    (i) =>
      (!f.newOnly || !i.imported) &&
      (f.kinds.length === 0 || f.kinds.includes(i.kind)) &&
      (i.date === null ? f.includeUndated || (f.from === null && f.to === null) : (f.from === null || i.date >= f.from) && (f.to === null || i.date <= f.to)),
  );
const deviceSession = () => ({
  id: 1,
  name: "EOS_DIGITAL",
  total: deviceItems.length,
  new: deviceItems.filter((i) => !i.imported).length,
  totalBytes: deviceItems.reduce((n, i) => n + i.size, 0),
  dateMin: null,
  dateMax: null,
  knowsImported: true,
  dating: false,
});

// Installed and tried store themes (Round 26 C2): `theme_install` / `theme_try` read the sample index above.
const shimThemes: { installed: Record<string, unknown>[]; tried: Record<string, unknown> | null } = { installed: [], tried: null };
function shimThemeOf(id: string, install: boolean): Record<string, unknown> {
  const idx = (commands.store_index as (a: Record<string, unknown>) => { themes: { id: string; name: string; author: string; summary: string; tags: string[]; tokens: unknown; latest: { version: string } }[] })({});
  const t = idx.themes.find((x) => x.id === id);
  const meta = {
    id, name: t?.name ?? id, author: t?.author ?? "", version: t?.latest.version ?? "1.0.0", description: t?.summary ?? "", license: "CC-BY-4.0",
    base: t?.tags.includes("dark") ? "darkroom" : "gallery", defaultScheme: t?.tags.includes("dark") ? "dark" : "light",
    layout: t?.tags.includes("layout") ?? false, hasIcon: true, tokens: t?.tokens,
  };
  if (install && !shimThemes.installed.some((x) => x.id === id)) shimThemes.installed = [...shimThemes.installed, meta];
  return meta;
}

const commands: Record<string, (args: Record<string, unknown>) => unknown> = {
  find_lightroom_catalogs: () => ["Travel 2023", "Family", "Weddings", "Scans"].map((name, i) => ({ path: `C:/Users/j/Pictures/Lightroom/${name}.lrcat`, name, photoCount: [8412, 3090, 12266, 540][i], modified: 1 })),
  camera_raw_preset_folder: () => null,
  import_lightroom_presets: () => ({ imported: 0, skippedSameName: 0, failed: [], notConverted: [] }),
  system_date_order: () => (navigator.language === "en-US" ? "mdy" : "dmy"),
  get_date_order: () => (state.settings.search.dateOrder === "auto" ? (navigator.language === "en-US" ? "mdy" : "dmy") : state.settings.search.dateOrder),
  get_app_info: () => ({ version: "26.1.0-alpha", platform: "browser", appDataDir: "C:/Users/you/AppData/Roaming/JL-Studios/Lumen", defaultLibraryDir: "C:/Users/you/Pictures/Lumen Library", isDebug: true }),
  get_free_space: () => 256 * 1024 ** 3,
  get_settings: () => state.settings,
  update_settings: (a) => {
    const next = a.settings as Settings;
    // As Rust does (Round 16): the key is kept, and made on first enable.
    const token = state.settings.aiAssistants.token || (next.aiAssistants.enabled ? shimToken() : "");
    return (state.settings = { ...next, aiAssistants: { ...next.aiAssistants, token } });
  },
  list_libraries: () => state.listing,
  get_current_library: () => state.library,
  get_missing_library: () => null,
  get_library_problem: () => null,
  library_exists_at: () => false,
  create_library: (a) => adopt(a.path as string),
  open_library: (a) => adopt(a.path as string),
  locate_library: (a) => adopt(a.path as string),
  forget_library: (a) => {
    state.listing.libraries = state.listing.libraries.filter((l) => l.entry.id !== a.id);
    return state.listing;
  },
  register_probe: () => 1,
  get_layout: (a) => layoutFor(a).summary,
  get_layout_rows: (a) => layoutFor(a).rows.slice(Number(a.start), Number(a.start) + Number(a.count)),
  layout_ids: (a) => layoutFor(a).rows.flatMap((r: { tiles: { id: number }[] }) => r.tiles.map((t) => t.id)),
  scope_ids: (a) => layoutFor({ ...a, width: 1000, rowHeight: 200, gap: 4 }).rows.flatMap((r: { tiles: { id: number }[] }) => r.tiles.map((t) => t.id)).slice(0, Number(a.limit ?? 10000)),
  update_status: updateStatus,
  check_for_updates: () => {
    const now = Math.floor(Date.now() / 1000);
    const { channel, automatic } = state.settings.updates;
    updateState =
      channel === "stable"
        ? { state: "upToDate", checkedAt: now }
        : automatic
          ? { state: "ready", version: "26.1.1-alpha", notes: NOTES, attempts: 0 }
          : { state: "available", version: "26.1.1-alpha", notes: NOTES };
    return updateStatus();
  },
  download_update: () => {
    updateState = { state: "ready", version: "26.1.1-alpha", notes: NOTES, attempts: 0 };
    return updateStatus();
  },
  install_update_now: () => null,
  take_update_receipt: () => (params.get("updated") ? { version: "26.1.0-alpha", notes: NOTES } : null),
  mcp_status: () => ({ state: state.settings.aiAssistants.enabled ? "waiting" : "off", client: null, message: null, exe: "C:/Program Files/Lumen/Lumen.exe" }),
  discord_set_screen: (a) => {
    shimScreen = a.screen as string;
    return null;
  },
  discord_info: () => {
    const d = state.settings.discord;
    const line = ({ browsing: "Browsing the library", viewing: "Looking through photos", developing: "Developing a photo", slideshow: "Watching a slideshow" } as Record<string, string>)[shimScreen];
    const preview = ["Competing in Lumen", d.showActivity ? line : null, d.showElapsed ? "with elapsed time" : null].filter(Boolean).join(" · ");
    return { available: true, state: d.enabled ? "waiting" : "off", preview };
  },
  // Round 22 lane C: the installed apps, as the browser plays them.
  open_in_app: (a) => ({ opened: (a.assetIds as number[]).length, made: (a.assetIds as number[]).length }),
  list_external_apps: (a) => {
    const all = [
      { id: "photoshop", name: "Adobe Photoshop", kind: "editor", accepts: ["photo", "raw"], iconUrl: appIcon("#31a8ff"), custom: false, send: "automatic", writes: "inPlace", addedBy: null },
      { id: "darktable", name: "darktable", kind: "developer", accepts: ["photo", "raw"], iconUrl: appIcon("#c0734a"), custom: false, send: "automatic", writes: "sidecar" },
      { id: "resolve", name: "DaVinci Resolve", kind: "video", accepts: ["video"], iconUrl: appIcon("#6d5bd0"), custom: false, send: "automatic", writes: "never" },
      ...state.settings.externalApps.custom.map((c) => ({ id: c.id, name: c.name, kind: "editor", accepts: ["photo", "raw", "video"], iconUrl: appIcon("#7a7a7a"), custom: true, send: "automatic", writes: "inPlace" })),
    ].map((x) => ({ ...x, send: state.settings.externalApps.send[x.id] ?? "automatic" }));
    const media = a.media as string | null | undefined;
    return media ? all.filter((x) => x.accepts.includes(media)) : all;
  },
  add_custom_app: (a) => {
    const path = String(a.path);
    const name = (path.split(/[\\/]/).pop() ?? path).replace(/\.[^.]+$/, "");
    const entry = { id: `custom-${name.toLowerCase().replace(/\W+/g, "").slice(0, 12)}`, name, path };
    const ext = state.settings.externalApps;
    if (!ext.custom.some((c) => c.id === entry.id)) ext.custom = [...ext.custom, entry];
    return { id: entry.id, name, kind: "editor", accepts: ["photo", "raw", "video"], iconUrl: appIcon("#7a7a7a"), custom: true, send: "automatic", writes: "inPlace" };
  },
  remove_custom_app: (a) => {
    const ext = state.settings.externalApps;
    ext.custom = ext.custom.filter((c) => c.id !== a.id);
    return null;
  },
  get_default_apps: () => state.settings.externalApps.defaults,
  // Round 23 lane B1: no plugins in the browser.
  list_plugin_themes: () => [],
  plugin_theme_css: () => "",
  // Round 23 lane C2: three plugins and an inspect result per install-sheet state.
  list_plugins: () => shimPlugins,
  // Round 24 lane W: ~40 fake suggestions in four moments, answered by the shim's own state.
  places_review: () => shimReview(),
  // Find locations…: models already there, one `working` answer then done.
  plugin_models_status: () => [{ id: "image", size: 610526312, licence: "CC-BY-NC-4.0", downloaded: true }],
  plugin_tool_run: () => ({ status: "done", suggested: shimReviewItems.length }),
  places_review_event: (a) => {
    const e = a.event as { kind: string; assets?: number[]; lat?: number; lon?: number };
    for (const i of shimReviewItems) {
      if (!e.assets?.includes(i.asset)) continue;
      if (e.kind === "confirm") i.status = "confirmed";
      if (e.kind === "reject") i.status = "rejected";
      if (e.kind === "move") {
        i.candidates = [{ lat: e.lat as number, lon: e.lon as number, confidence: 1, label: null }];
        i.status = "confirmed";
      }
    }
    return shimReview();
  },
  // Round 25 P4b-2b: the generic review page.
  proposals_list: (a) =>
    ({
      items: shimProposals.filter((p) => (p.proposal.proposed as { kind: string }).kind === a.kind && p.proposal.status === "pending"),
      nextCursor: null,
    }),
  proposals_accept: (a) => {
    const items = a.items as { id: number; value?: unknown }[];
    for (const i of items) {
      const p = shimProposals.find((x) => x.proposal.id === i.id);
      if (p) p.proposal.status = "accepted";
    }
    return { accepted: items.map((i) => i.id), skipped: [], undo: { proposals: items.map((i) => i.id), reversals: [] } };
  },
  proposals_undo_accept: (a) => {
    for (const id of (a.undo as { proposals: number[] }).proposals) {
      const p = shimProposals.find((x) => x.proposal.id === id);
      if (p) p.proposal.status = "pending";
    }
    return null;
  },
  proposals_reject: (a) => {
    for (const id of a.ids as number[]) {
      const p = shimProposals.find((x) => x.proposal.id === id);
      if (p) p.proposal.status = "rejected";
    }
    return (a.ids as number[]).length;
  },
  proposals_restore: (a) => {
    for (const id of a.ids as number[]) {
      const p = shimProposals.find((x) => x.proposal.id === id);
      if (p) p.proposal.status = "pending";
    }
    return (a.ids as number[]).length;
  },
  places_write: () => "places-write-dev",
  places_undo: () => ({ reverted: 0, keptInFile: [] }),
  // Round 24 lane D: no file dialogs or providers in the browser (a pick is a cancel).
  dialog_pick: () => null,
  reveal_path: () => null,
  plugin_provider_choices: () => [],
  set_provider_choice: () => null,
  plugin_detect: () => [{ id: "disk-atlas", name: "Disk Atlas", path: null, version: null, located: false }],
  plugin_locate: () => ({ id: "disk-atlas", name: "Disk Atlas", path: "C:/Apps/disk-atlas.exe", version: "0.4.2", located: true }),
  inspect_plugin_package: (a) => {
    const p = String(a.path ?? "");
    const base = shimPlugins[1];
    if (p.includes("unsigned")) return { info: shimPlugins[2], adds: ["Tools"], problems: [], notes: ["Unsigned: Developer mode only."], update: null, older: null };
    if (p.includes("update")) return { info: base, adds: ["Importer"], problems: [], notes: [], update: { from: "0.9.0", newPermissions: ["photosRead"] }, older: null };
    if (p.includes("older")) return { info: base, adds: ["Importer"], problems: [], notes: [], update: null, older: "2.0.0" };
    if (p.includes("broken")) return { info: base, adds: [], problems: ["plugin.toml is missing a name."], notes: [], update: null, older: null };
    return { info: base, adds: ["Importer"], problems: [], notes: [], update: null, older: null };
  },
  install_plugin: () => shimPlugins[1],
  set_plugin_enabled: () => null,
  remove_plugin: () => "token-1",
  undo_remove_plugin: () => null,
  set_plugin_setting: () => null,
  get_plugin_developer_mode: () => true,
  plugin_readme: () => null,
  set_plugin_developer_mode: () => null,
  load_dev_plugin: () => shimPlugins[2],
  reload_dev_plugin: () => shimPlugins[2],
  plugin_log: () => ["[info] started"],
  // Round 25 P3: the sampler's button, stat and list.
  plugin_ui_action: () => "Scanned 12 folders.",
  plugin_ui_value: (a) => ((a as { key: string }).key === "last_scan" ? "3 minutes ago" : JSON.stringify(["Photos", { text: "Videos", detail: "4.2 GB" }, "Documents"])),
  take_pending_plugin_package: () => null,
  // The plugin store (Round 24 lane C, Round 26 C1). `?store=offline` (nothing cached), `?store=cached` (offline with a list), `?store=empty`.
  store_index: () => {
    if (params.get("store") === "offline") throw { code: "store_offline", message: "The plugin store needs the internet." };
    const rel = (version: string, size: number) => ({ version, api: "1.2.0", url: "https://example.invalid/p.lumenplugin", blake3: "0".repeat(64), size, released: "2026-11-21" });
    // Synthetic art: flat SVGs as data URLs (the real app serves cached copies over photos://store-image/<hash>).
    const enc = (body: string, w: number, h: number) => `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}">${body}</svg>`)}`;
    const icon = (hue: number, glyph: string) =>
      enc(`<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 80% 62%)"/><stop offset="1" stop-color="hsl(${hue + 40} 70% 38%)"/></linearGradient></defs><rect width="128" height="128" rx="28" fill="url(#g)"/><text x="64" y="84" text-anchor="middle" font-size="58" font-weight="700" fill="white" font-family="sans-serif">${glyph}</text>`, 128, 128);
    // A miniature Lumen window: title bar, sidebar, a grid of photo tiles in the plugin's hue.
    const shot = (hue: number, seed: number) => {
      let tiles = "";
      for (let r = 0; r < 3; r++) for (let c = 0; c < 5; c++) tiles += `<rect x="${230 + c * 142}" y="${110 + r * 152}" width="132" height="142" rx="8" fill="hsl(${hue + ((r * 5 + c + seed) * 23) % 90} ${40 + ((c + seed) % 3) * 15}% ${30 + ((r + c + seed) % 4) * 10}%)"/>`;
      return enc(`<rect width="960" height="600" fill="hsl(${hue} 18% 12%)"/><rect width="960" height="38" fill="hsl(${hue} 18% 16%)"/><circle cx="24" cy="19" r="6" fill="hsl(${hue} 60% 60%)"/><rect x="0" y="38" width="210" height="562" fill="hsl(${hue} 18% 15%)"/>${[0, 1, 2, 3, 4, 5].map((i) => `<rect x="20" y="${70 + i * 40}" width="${120 + ((i * 37) % 50)}" height="12" rx="6" fill="hsl(${hue} 15% ${i === seed % 6 ? 55 : 30}%)"/>`).join("")}<rect x="230" y="58" width="300" height="22" rx="11" fill="hsl(${hue} 15% 22%)"/><rect x="820" y="56" width="110" height="28" rx="14" fill="hsl(${hue} 70% 55%)"/>${tiles}`, 960, 600);
    };
    const days = (n: number) => new Date(Date.now() - n * 86_400_000).toISOString().slice(0, 10);
    const entry = (id: string, name: string, author: string, jl: boolean, summary: string, kinds: string[], permissions: string[], hue: number | null, shots: number, released = "2026-09-01") => ({
      id, name, author, jl, summary, kinds, permissions,
      icon: hue === null ? null : icon(hue, name[0]),
      homepage: "https://github.com/05Narwhal/lumen-plugins", readme: jl ? "https://example.invalid/README.md" : null,
      screenshots: hue === null ? [] : Array.from({ length: shots }, (_, i) => shot(hue + i * 25, i)),
      authorUrl: null, repository: null, latest: { ...rel("1.2.0", 48_000 + id.length * 1_234), released },
    });
    // Theme sample data (Round 26 C2): the swatches the store's miniature is painted from.
    const sw = (bg: string, surface: string, sidebar: string, fg: string, fg2: string, accent: string, placeholder: string, radius = "10px", gap = "6px") => ({ bg, surface, sidebar, fg, fg2, accent, placeholder, radius, gap });
    const theme = (id: string, name: string, author: string, jl: boolean, summary: string, tags: string[], light: ReturnType<typeof sw>, dark: ReturnType<typeof sw>, hue: number | null, released = "2026-09-01") => ({
      id, name, author, jl, summary, tags, tokens: { light, dark },
      icon: hue === null ? null : icon(hue, name[0]),
      homepage: null, readme: jl ? "https://example.invalid/README.md" : null,
      screenshots: hue === null ? [] : [shot(hue, 1), shot(hue + 30, 2)],
      authorUrl: null, repository: null, latest: { ...rel("1.0.0", 36_000 + id.length * 777), released },
    });
    const themes = params.get("store") === "empty" ? [] : [
      theme("jl.nocturne", "Nocturne", "JL Studios", true, "Inky violet, quiet and deep, for editing late.", ["dark", "colourful"], sw("oklch(97% 0.01 300)", "oklch(100% 0 0)", "oklch(94% 0.02 300)", "oklch(22% 0.03 300)", "oklch(48% 0.03 300)", "oklch(52% 0.18 300)", "oklch(88% 0.03 300)"), sw("oklch(15% 0.03 300)", "oklch(19% 0.035 300)", "oklch(12% 0.03 300)", "oklch(94% 0.01 300)", "oklch(70% 0.03 300)", "oklch(72% 0.17 300)", "oklch(28% 0.05 300)"), 285, days(2)),
      theme("jl.coastline", "Coastline", "JL Studios", true, "Teal water and warm sand, with rounder corners.", ["light", "colourful"], sw("oklch(97% 0.02 90)", "oklch(99% 0.01 90)", "oklch(93% 0.035 190)", "oklch(24% 0.03 220)", "oklch(48% 0.03 220)", "oklch(56% 0.1 195)", "oklch(88% 0.04 85)", "16px", "8px"), sw("oklch(20% 0.03 220)", "oklch(25% 0.035 220)", "oklch(17% 0.03 220)", "oklch(94% 0.01 90)", "oklch(72% 0.03 200)", "oklch(74% 0.1 190)", "oklch(34% 0.05 200)", "16px", "8px"), 190, days(2)),
      theme("jl.paper-mill", "Paper Mill", "JL Studios", true, "Warm paper with the navigation along the top.", ["light", "layout"], sw("oklch(96% 0.02 85)", "oklch(99% 0.01 85)", "oklch(92% 0.03 80)", "oklch(26% 0.02 60)", "oklch(48% 0.03 60)", "oklch(52% 0.12 40)", "oklch(86% 0.04 80)", "4px", "4px"), sw("oklch(22% 0.02 60)", "oklch(26% 0.02 60)", "oklch(19% 0.02 60)", "oklch(92% 0.02 85)", "oklch(70% 0.03 75)", "oklch(72% 0.12 50)", "oklch(34% 0.03 60)", "4px", "4px"), 40, days(2)),
      theme("studio.nord", "Nord", "Arctic Ice Studio", false, "The arctic, north-bluish palette.", ["dark"], sw("oklch(96% 0.01 240)", "oklch(99% 0.005 240)", "oklch(92% 0.015 240)", "oklch(30% 0.03 255)", "oklch(50% 0.03 250)", "oklch(60% 0.09 225)", "oklch(86% 0.02 240)"), sw("oklch(28% 0.03 260)", "oklch(32% 0.03 260)", "oklch(24% 0.03 260)", "oklch(93% 0.01 240)", "oklch(72% 0.03 240)", "oklch(76% 0.08 215)", "oklch(40% 0.04 255)"), 230),
      theme("com.example.sandstone", "Sandstone", "Ada Author", false, "Soft desert tones with generous space between photos.", ["light"], sw("oklch(96% 0.02 70)", "oklch(99% 0.01 70)", "oklch(92% 0.03 65)", "oklch(28% 0.03 50)", "oklch(50% 0.04 55)", "oklch(58% 0.12 55)", "oklch(86% 0.05 70)", "12px", "12px"), sw("oklch(24% 0.02 50)", "oklch(28% 0.02 50)", "oklch(21% 0.02 50)", "oklch(92% 0.02 70)", "oklch(72% 0.04 65)", "oklch(74% 0.12 60)", "oklch(36% 0.04 55)", "12px", "12px"), 55),
      theme("com.example.neon", "Neon Arcade", "Bo Lindqvist", false, "Hot pink and cyan on near-black, for people who like it loud.", ["dark", "colourful"], sw("oklch(97% 0.02 340)", "oklch(100% 0 0)", "oklch(94% 0.04 330)", "oklch(24% 0.05 320)", "oklch(48% 0.07 320)", "oklch(58% 0.25 350)", "oklch(88% 0.06 340)"), sw("oklch(14% 0.04 290)", "oklch(18% 0.05 290)", "oklch(11% 0.04 290)", "oklch(95% 0.02 200)", "oklch(74% 0.1 210)", "oklch(72% 0.22 350)", "oklch(26% 0.08 300)"), 330),
      theme("com.example.gridline", "Gridline", "Kit Maker", false, "Square corners, hairlines and a sidebar that sits on top.", ["light", "layout"], sw("oklch(98% 0 0)", "oklch(100% 0 0)", "oklch(95% 0 0)", "oklch(20% 0 0)", "oklch(46% 0 0)", "oklch(48% 0.2 260)", "oklch(88% 0 0)", "0px", "2px"), sw("oklch(18% 0 0)", "oklch(22% 0 0)", "oklch(15% 0 0)", "oklch(94% 0 0)", "oklch(70% 0 0)", "oklch(70% 0.16 260)", "oklch(32% 0 0)", "0px", "2px"), null),
    ];
    return {
      version: 1,
      featured: "jl.wayfinder",
      featuredTheme: "jl.nocturne",
      themes,
      plugins: params.get("store") === "empty" ? [] : [
        entry("jl.wayfinder", "Wayfinder", "JL Studios", true, "Find where photos without GPS were taken, from the photos around them, and review every guess before it is written.", ["tools"], ["library.read", "places.write", "originals.gps"], 200, 3, days(3)),
        entry("jl.film-lab", "Film Lab", "JL Studios", true, "Thirty hand-made film-inspired looks for Develop: grain, halation and colour that feel like a roll you shot.", ["presets"], [], 28, 2, days(9)),
        entry("jl.disk-atlas", "Disk Atlas", "JL Studios", true, "See what fills your disks and pick a folder to import from a map of its sizes.", ["provider", "apps"], ["fs.read-chosen"], 150, 2),
        entry("studio.jl.takeout", "Google Takeout", "JL Studios", true, "Import a Google Takeout export with its albums, dates and places intact.", ["importer"], ["import.read-chosen", "storage"], 8, 1),
        entry("jl.clockwork", "Clockwork", "JL Studios", true, "Finds a camera whose clock was off and proposes corrected dates for its photos.", ["tools"], ["library.read", "dates.write"], 265, 1, days(1)),
        entry("jl.captioner", "Captioner", "JL Studios", true, "Suggests titles and captions from the people, places and text in each photo.", ["tools"], ["library.read", "captions.write"], 320, 0, days(1)),
        entry("com.example.warm", "Warm looks", "Ada Author", false, "Twelve warm presets for golden hour and indoor light.", ["presets"], [], 40, 1),
        entry("com.example.grain", "Fine grain", "Bo Lindqvist", false, "Monochrome looks with real scanned grain, from soft to gritty.", ["presets"], [], null, 0),
        entry("com.example.resolve", "Send to Resolve", "Kit Maker", false, "Open a video in DaVinci Resolve with its trim and markers.", ["apps"], [], 230, 1),
        entry("com.example.ollama", "Local captions", "Rin", false, "Use a model running on your own computer to describe photos.", ["provider"], ["library.read"], 95, 0),
      ],
    };
  },
  store_readme: () =>
    [
      "## What it does",
      "",
      "It looks at the photos taken around each one: the same day, the same people, the same trip. When those know where they were, it suggests a place, with how sure it is.",
      "",
      "## Nothing leaves your computer",
      "",
      "- Guesses are made on this computer, offline.",
      "- Nothing is written until you **review** it.",
      "- Every write is verified and can be undone.",
    ].join("\n"),
  store_status: () => ({ offline: params.get("store") === "cached", hasCache: true, checkedAt: null, autoUpdate: true, updates: [{ id: "jl.disk-atlas", name: "Disk Atlas", from: "1.0.0", to: "1.2.0", needsReview: false }], updated: [] }),
  store_install: () => ({ path: "C:/tmp/store.lumenplugin", installed: false }),
  store_update: () => ({ path: null, installed: true }),
  // Themes (Round 26): installing, trying and applying a store theme.
  themes_list: () => shimThemes.installed,
  theme_install: (a) => shimThemeOf(String(a.storeId), true),
  theme_remove: (a) => {
    shimThemes.installed = shimThemes.installed.filter((t) => t.id !== a.id);
    return null;
  },
  theme_try: (a) => (shimThemes.tried = shimThemeOf(String(a.storeId), false)),
  theme_try_end: () => {
    shimThemes.tried = null;
    return null;
  },
  theme_keep: () => shimThemeOf(String(shimThemes.tried?.id ?? ""), true),
  theme_apply: (a) => {
    state.settings.appearance.theme = { plugin: String(a.id), theme: "theme" } as unknown as Settings["appearance"]["theme"];
    return null;
  },
  store_set_auto_update: () => null,
  set_default_app: (a) => {
    state.settings.externalApps.defaults = { ...state.settings.externalApps.defaults, [String(a.media)]: (a.id as string | null) ?? null };
    return null;
  },
  set_app_send_mode: (a) => {
    const send = { ...state.settings.externalApps.send, [String(a.id)]: a.mode as never };
    if (a.mode === "automatic") delete send[String(a.id)];
    state.settings.externalApps.send = send;
    return null;
  },
  mcp_connect: () => "Connected. Start a new session to see Lumen.",
  mcp_regenerate_token: () => {
    state.settings.aiAssistants = { ...state.settings.aiAssistants, token: shimToken() };
    return state.settings;
  },
  get_ml_status: () => ({
    enabled: state.settings.smart.enabled,
    models: [
      { id: "clip-vit-b32-fp16@1", role: "image_embedding", bytes: 176_214_230, ready: true },
      { id: "beat-this-small@1", role: "beat_activation", bytes: 9_274_894, ready: true },
    ],
    embedded: Math.floor(items.length * 0.6),
    photos: items.length,
    faces: 69,
    people: 3,
  }),
  fetch_models: () => null,
  list_people: () => [
    { id: 1, name: "Anna", faceCount: 42, photoCount: 31, coverFaceId: 2 },
    { id: 2, name: "Marko", faceCount: 18, photoCount: 15, coverFaceId: 7 },
    { id: 3, name: null, faceCount: 9, photoCount: 8, coverFaceId: 11 },
    { id: 4, name: "Mama", faceCount: 60, photoCount: 52, coverFaceId: 5 },
    { id: 5, name: "Luka", faceCount: 12, photoCount: 10, coverFaceId: 9 },
    { id: 6, name: "Sara", faceCount: 7, photoCount: 6, coverFaceId: 13 },
  ],
  list_person_groups: () => [
    { id: 1, name: "Family", personIds: [1, 2, 4] },
    { id: 2, name: "Friends", personIds: [5, 6] },
    { id: 3, name: "Work", personIds: [] },
  ],
  create_person_group: () => 4,
  rename_person_group: () => null,
  delete_person_group: () => [],
  set_person_group_members: () => null,
  faces_for_asset: (a) =>
    Number(a.id) % 3 === 0
      ? [
          { id: 1, personId: 1, name: "Anna", x: 0.4, y: 0.3, w: 0.1, h: 0.14, manual: false },
          { id: 2, personId: null, name: null, x: 0.62, y: 0.28, w: 0.09, h: 0.13, manual: false },
        ]
      : [],
  tag_face: (a) => ({ face: Number(a.face), person: null, pinned: false }),
  create_person_with_face: (a) => [99, { face: Number(a.face), person: null, pinned: false }],
  undo_create_person: () => null,
  add_face_box: () => [500, null],
  untag_face: (a) => ({ place: { face: Number(a.face), person: 1, pinned: false }, manual: null }),
  undo_untag_face: () => null,
  name_person: () => null,
  merge_people: (a) => ({ faces: [], name: null, tookName: false, into: Number(a.into) }),
  restore_person: () => 4,
  // `?removed=N`: N people removed from People, for Settings → People's tiles.
  list_removed_people: () =>
    Array.from({ length: Number(params.get("removed") ?? 0) }, (_, i) => ({ id: 100 + i, name: i % 3 === 2 ? null : `Person ${i + 1}`, faceCount: 3 + i, photoCount: 2 + i, coverFaceId: (i % 13) + 1 })),
  // Round 14: Julian's dev folder today — the legacy Ray at High, timed; no Prism yet.
  experimental_catalog: () => {
    const row = (task: string, family: string, extra: object = {}) => ({
      task, family, switchable: task === "auto" || task === "retouch", builtin: null, models: [], model: null,
      recommended: "high", testing: false, tier: null, budgetMs: task === "auto" ? 1000 : task === "retouch" ? 1500 : null, ...extra,
    });
    const tier = (t: string, ms: number | null, extra: object = {}) => ({ tier: t, ready: true, reason: null, precision: "fp32", tta: t === "ultra" ? 5 : 1, valDe: null, ms, ep: ms == null ? null : "directml", failed: null, ...extra });
    const ray = { id: "lumen-ray-1.0.1-d", name: "Lumen Ray 1.0.1-d", version: "1.0.1-d", legacy: false, tiers: [tier("low", null, { precision: "int8", failed: "inference failed: 80070057 The parameter is incorrect" }), tier("high", 184, { valDe: 3.94 }), tier("ultra", 412, { valDe: 3.71, ep: "cpu" })] };
    const legacy = { id: "lumen-ray-1.0.1-d-high", name: "Lumen Ray-high 1.0.1-d", version: "1.0.1-d", legacy: true, tiers: [tier("high", 190, { valDe: 4.14 })] };
    return [
      row("auto", "ray", { models: [ray, legacy], model: ray.id, tier: "high" }),
      row("retouch", "prism"),
      row("search", "spectrum", { builtin: "Lumen Spectrum 1.0 (based on CLIP ViT-B/32)" }),
      row("text", "glyph"),
      row("masks", "facet", { switchable: true, builtin: "Lumen Facet 1.0.0-d (based on EoMT-S)" }),
    ];
  },
  retest_models: () => null,
  ai_masks_list: () => shimAiList(),
  ai_masks_detect: async () => {
    await new Promise((r) => setTimeout(r, 900));
    shimScanned = true;
    return shimAiList();
  },
  denoise_status: () => ({ available: true, reason: null, model: "nafnet-sidd-w32@1", ready: false, running: false, gpu: true }),
  start_denoise: () => null,
  cancel_denoise: () => null,
  develop_auto: () => ({ basic: { temperature: 4, tint: 0, exposure: 0.35, contrast: 0.12, highlights: -0.2, shadows: 0.15, whites: 0.05, blacks: -0.05, texture: 0, clarity: 0, dehaze: 0, vibrance: 0.15, saturation: 0 }, usedModel: true, scene: "sunset" }),
  list_rejected_faces: () => [],
  restore_face: () => null,
  remove_person: () => null,
  restore_removed_person: () => null,
  reject_person: (a) => ({ id: Number(a.id), name: null, coverFaceId: null, createdAt: 0, status: "listed", faces: [], rejections: [] }),
  undo_reject_person: () => null,
  set_person_cover: () => null,
  person_suggestions: () => [],
  answer_suggestion: (a) => ({ face: Number(a.face), person: Number(a.person), yes: Boolean(a.yes), before: { face: Number(a.face), person: null, pinned: false } }),
  undo_suggestion: () => null,
  find_layout_row: (a) => layoutFor(a).rows.find((r) => r.tiles.some((t) => t.id === Number(a.assetId)))?.index ?? null,
  get_asset: (a) => detail(Number(a.id)),
  get_asset_neighbors: (a) => {
    const i = items.findIndex((x) => x.id === Number(a.id));
    return { prev: items[i - 1]?.id ?? null, next: items[i + 1]?.id ?? null };
  },
  get_library_stats: () => ({ items: items.length, videos: items.filter((i) => i.video).length, bytes: items.length * 3_400_000 }),
  get_recent_asset_ids: (a) => items.slice(0, Number(a.limit)).map((i) => i.id),
  list_import_batches: () => batches,
  get_import_skips: () => [
    { path: "notes.txt", reason: "unsupported", assetId: null },
    { path: "2024/IMG_0001-copy.jpg", reason: "duplicate", assetId: 1 },
  ],
  get_import_failures: () => [{ path: "IMG_0042.jpg", reason: "decode failed: unexpected end of file" }],
  start_folder_import: () => 3,
  cancel_import: () => null,
  map_bounds: () => ({ minLat: 35.9, maxLat: 48.2, minLon: -9.6, maxLon: 16.4 }),
  map_clusters: () => {
    // A handful of European cities, so the map has something to show in the shim.
    const spots: [number, number, number][] = [
      [38.72, -9.14, 34],
      [40.42, -3.7, 12],
      [41.9, 12.5, 8],
      [46.06, 14.51, 1],
      [48.14, 11.58, 21],
      [45.46, 9.19, 3],
    ];
    return spots.map(([lat, lon, count], i) => ({
      lat,
      lon,
      count,
      coverAssetId: items[(i * 5) % items.length].id,
      assetId: count === 1 ? items[(i * 5) % items.length].id : null,
      bounds: { minLat: lat - 0.2, maxLat: lat + 0.2, minLon: lon - 0.2, maxLon: lon + 0.2 },
    }));
  },
  assets_in_bounds: () => items.slice(0, 24).map((i) => i.id),
  search_places: (a) => {
    const q = String(a.query ?? "").toLowerCase();
    return [
      { label: "Lisbon, Portugal", lat: 38.72, lon: -9.14 },
      { label: "Ljubljana, Slovenia", lat: 46.06, lon: 14.51 },
      { label: "London, United Kingdom", lat: 51.51, lon: -0.13 },
    ].filter((p) => p.label.toLowerCase().startsWith(q));
  },
  place_for: () => ({ city: "Lisbon", country: "Portugal" }),
  set_rating: () => 1,
  set_color_label: (a) => setMark(a.ids as number[], "label", Number(a.label)),
  set_flag: (a) => setMark(a.ids as number[], "flag", Number(a.flag)),
  copy_assets_to: (a) => (a.ids as number[]).length,
  count_scope: (a) => {
    const s = a.scope as { query?: { text?: string | null; kind?: Kind | null } } | undefined;
    const text = s?.query?.text?.trim();
    const pool = items.filter((i) => hasKind(i, s?.query?.kind));
    return text ? Math.max(1, Math.round(pool.length / (text.length + 1))) : pool.length;
  },
  // Round 4K: a pretend reading of the text. People by name, a few things, years and two
  // places, with "Florence" both a person and a city to show the alternatives menu.
  // Round 7: a few tags of each kind, matched by word prefix as Rust does.
  suggest_tags: (a) => {
    const words = String(a?.input ?? "").toLowerCase().split(/\s+/).filter(Boolean);
    // Round 15: an empty box offers starters.
    if (!words.length)
      return [
        { tag: { label: "Taken: 2026", value: { type: "dates", from: 1_767_225_600, to: 1_798_761_599, field: null } }, field: "Try", count: 1204 },
        { tag: { label: "File created: 2025", value: { type: "dates", from: 1_735_689_600, to: 1_767_225_599, field: "fileCreated" } }, field: "Try", count: 96 },
        { tag: { label: "Imported: 2026", value: { type: "dates", from: 1_767_225_600, to: 1_798_761_599, field: "imported" } }, field: "Try", count: 4818 },
        { tag: { label: "Longer than 1 min", value: { type: "range", measure: "duration", min: 60_000, max: null } }, field: "Try", count: null },
      ];
    const all = [
      { tag: { label: "Anna Berg", value: { type: "person", id: 1 } }, field: "Person", count: 412 },
      { tag: { label: "Lisbon, Portugal", value: { type: "place", place: { kind: "city", name: "Lisbon", country: "Portugal" } } }, field: "Place", count: 286 },
      { tag: { label: "Portugal", value: { type: "place", place: { kind: "country", name: "Portugal" } } }, field: "Place", count: 731 },
      { tag: { label: "Photos", value: { type: "media", media: 0 } }, field: "Kind", count: 4818 },
      { tag: { label: "Videos", value: { type: "media", media: 1 } }, field: "Kind", count: 236 },
      { tag: { label: "Live Photos", value: { type: "kind", kind: "live" } }, field: "Kind", count: 1904 },
      { tag: { label: "HDR", value: { type: "kind", kind: "hdr" } }, field: "Kind", count: 512 },
      { tag: { label: "Screenshots", value: { type: "kind", kind: "screenshot" } }, field: "Kind", count: 143 },
      { tag: { label: "iPhone 12", value: { type: "exif", field: "model", value: "iPhone 12" } }, field: "Camera", count: 3920 },
      { tag: { label: "ISO 100", value: { type: "exif", field: "iso", value: "100" } }, field: "ISO", count: 640 },
      { tag: { label: "f/1.6", value: { type: "exif", field: "aperture", value: "1.6" } }, field: "Aperture", count: 2210 },
      { tag: { label: "Adobe Photoshop Lightroom Classic 13.2 (Windows)", value: { type: "exif", field: "software", value: "Adobe Photoshop Lightroom Classic 13.2 (Windows)" } }, field: "Exported from", count: 260 },
      { tag: { label: "iOS 18.5", value: { type: "exif", field: "software", value: "18.5" } }, field: "Exported from", count: 736 },
      { tag: { label: "Beach", value: { type: "scene", tag: "beach" } }, field: "Scene", count: 158 },
      { tag: { label: "Favorites", value: { type: "favorites" } }, field: "Kind", count: 88 },
    ];
    const aliases: Record<string, string> = { "Exported from": "exported from software", Camera: "camera model", ISO: "iso", Aperture: "aperture" };
    const found = all.filter((s) => {
      const hay = `${s.tag.label} ${aliases[s.field] ?? ""}`.toLowerCase().split(/[^\p{L}\p{N}.]+/u);
      return words.every((w) => hay.some((h) => h.startsWith(w)));
    });
    return words.length ? found.slice(0, 12) : [];
  },
  // Round 20 lane A: the shim reads no sentence (the field then behaves as before); queries
  // carry `tagExpr` and the shim's counts and lists ignore it, as they ignore `searchTags`.
  // A flat reader for the look: words between and / or / not, no brackets, each term the first
  // suggestion that matches, else a text tag.
  read_search_sentence: (a) => {
    const text = String(a?.input ?? "").trim();
    if (!/(^|\s)(and|or|not)(\s|$)/i.test(text)) return null;
    const words = text.split(/\s+/);
    let lead: "and" | "or" | null = null;
    if (/^(and|or)$/i.test(words[0] ?? "") && words.length > 1) lead = words.shift()!.toLowerCase() as "and" | "or";
    const items: unknown[] = [];
    const joins: string[] = [];
    let join = "and";
    let not = false;
    let term: string[] = [];
    const flush = () => {
      if (!term.length) return;
      const t = term.join(" ");
      const hit = (commands.suggest_tags({ input: t }) as { tag: unknown }[])[0];
      if (items.length) joins.push(join);
      items.push({ type: "tag", ...(not ? { not: true } : {}), tag: hit?.tag ?? { label: t, value: { type: "text", text: t } } });
      term = [];
      not = false;
      join = "and";
    };
    for (const w of words) {
      const l = w.toLowerCase();
      if (l === "and" || l === "or") {
        flush();
        join = l;
      } else if (l === "not") {
        flush();
        not = true;
      } else term.push(w);
    }
    const tail = term.join(" ");
    flush();
    return { lead, seq: { items, joins }, term: tail };
  },
  search_overview: (a) => {
    const q = String(a?.q ?? "");
    const enc = new TextEncoder();
    const bytes = (i: number) => enc.encode(q.slice(0, i)).length;
    const people = (commands.list_people as () => { id: number; name: string | null; photoCount: number; coverFaceId: number }[])();
    const things = ["beach", "dog", "cat", "sunset", "snow", "food", "car", "train", "mountain"];
    const chips: unknown[] = [];
    const rest: string[] = [];
    const found = { people: [] as unknown[], places: [] as unknown[] };
    for (const m of q.matchAll(/\S+/g)) {
      const w = m[0].toLowerCase();
      const span = [bytes(m.index ?? 0), bytes((m.index ?? 0) + m[0].length)];
      const person = people.find((p) => p.name?.toLowerCase() === w);
      const chip = (kind: string, label: string, token: string, alternatives: unknown[] = []) => ({ kind, label, span, token, alternatives, path: [] as string[] });
      if (w === "florence") {
        const place = { kind: "city", name: "Florence", country: "Italy" };
        chips.push(chip("place", "Florence, Italy", 'place:"Florence, Italy"', [chip("person", "Florence", 'person:"Florence"')]));
        found.places.push({ place, label: "Florence, Italy", photoCount: 40, lat: 43.77, lon: 11.25, coverAssetId: items[3]?.id ?? null });
      } else if (w === "lisbon") {
        chips.push(chip("place", "Lisbon, Portugal", 'place:"Lisbon, Portugal"'));
        found.places.push({ place: { kind: "city", name: "Lisbon", country: "Portugal" }, label: "Lisbon, Portugal", photoCount: 120, lat: 38.72, lon: -9.14, coverAssetId: items[5]?.id ?? null });
      } else if (person) {
        chips.push(chip("person", person.name ?? "", `person:"${person.name}"`));
        found.people.push(person);
      } else if (things.includes(w)) {
        chips.push(chip("tag", w[0].toUpperCase() + w.slice(1), `tag:"${w}"`));
      } else if (/^(19|20)\d\d$/.test(w)) {
        chips.push(chip("date", w, w));
      } else rest.push(m[0]);
    }
    const words = q.toLowerCase().split(/\s+/).filter((w) => w.length >= 3);
    const named = (title: string) => words.some((w) => title.toLowerCase().includes(w));
    return {
      chips,
      rest: rest.join(" "),
      people: found.people,
      pets: [],
      places: found.places,
      albums: albums.filter((al) => named(al.name)).slice(0, 12),
      memories: shimMemories.filter((m) => !m.hidden && named(m.title)).slice(0, 12),
      total: q.trim() === "nothing" ? 0 : Math.max(1, Math.round(items.length / (q.length + 1))),
    };
  },
  rebuild_search_index: () => items.length,
  start_files_import: () => 44,
  list_edit_versions: () => [],
  get_edit: () => null,
  save_edit: () => Date.now(),
  get_develop_draft: () => null,
  save_develop_draft: () => Date.now(),
  restore_develop_drafts: () => null,
  revert_edit: () => null,
  export_assets: () => 1,
  // Develop (Phase 8c).
  develop_preview: (a) => {
    const it = items.find((i) => i.id === Number(a?.assetId)) ?? items[0];
    const w = it.aspect >= 1 ? 1600 : Math.round(1600 * it.aspect);
    const h = it.aspect >= 1 ? Math.round(1600 / it.aspect) : 1600;
    return { token: String(it.id), width: w, height: h, superseded: false, histogram: shimHistogram(it.id) };
  },
  // Duplicates (Round 4D): a few made-up sets of neighbouring stills, dismissals remembered.
  list_duplicate_sets: () => shimDuplicateSets(),
  count_duplicate_sets: () => {
    const sets = shimDuplicateSets();
    return [sets.length, sets.reduce((n, s) => n + s.members.length - 1, 0)];
  },
  resolve_duplicates: (a) => {
    const sets = a?.sets as { keep: number; remove: number[] }[];
    for (const s of sets) for (const id of s.remove) shimResolved.add(id);
    return sets.map((s) => ({ keep: s.keep, removed: s.remove, favorite: false, rating: 0, colorLabel: 0, flag: 0, keywords: null, title: null, caption: null, albumsAdded: [] }));
  },
  undo_resolve_duplicates: (a) => {
    for (const u of a?.undos as { removed: number[] }[]) for (const id of u.removed) shimResolved.delete(id);
    return null;
  },
  dismiss_duplicates: (a) => {
    shimDismissed.add((a?.ids as number[]).join(","));
    return null;
  },
  undo_dismiss_duplicates: (a) => {
    shimDismissed.delete((a?.ids as number[]).join(","));
    return null;
  },
  // The shim has one image per photo: the before is the same picture, which is enough to see
  // the split and the panes line up.
  develop_before: (a) => {
    const it = items.find((i) => i.id === Number(a?.assetId)) ?? items[0];
    const w = it.aspect >= 1 ? 1600 : Math.round(1600 * it.aspect);
    const h = it.aspect >= 1 ? Math.round(1600 / it.aspect) : 1600;
    return { token: String(it.id), width: w, height: h, superseded: false, histogram: null };
  },
  develop_region: (a) => ({ token: String(a?.assetId ?? 1), width: 1200, height: 800, superseded: false, histogram: null }),
  develop_release_full: () => null,
  develop_base_info: (a) => {
    const it = items.find((i) => i.id === Number(a?.assetId)) ?? items[0];
    return { width: it.aspect >= 1 ? 6048 : Math.round(6048 * it.aspect), height: it.aspect >= 1 ? Math.round(6048 / it.aspect) : 6048, raw: isRaw(it) };
  },
  strip_ids: (a) => stripItems(a as StripArgs).map((i) => i.id),
  strip_page: (a) => {
    const all = stripItems(a as StripArgs);
    const cursor = a?.cursor as { id: number } | null;
    const start = cursor ? all.findIndex((i) => i.id === cursor.id) + 1 : 0;
    const limit = Number(a?.limit ?? 100);
    const page = all.slice(start, start + limit);
    const last = page.at(-1);
    return { items: page, nextCursor: start + limit < all.length && last ? { time: 0, rating: last.rating, filename: last.filename, id: last.id } : null };
  },
  list_presets: () => [...shimBuiltinPresets(), ...shimUserPresets],
  save_preset: (a) => {
    const p = { id: 100 + shimUserPresets.length, key: `db:${100 + shimUserPresets.length}`, source: "mine" as const, name: String(a?.name), develop: a?.develop, panels: a?.panels as string[], builtin: false };
    shimUserPresets.push(p);
    return p;
  },
  rename_preset: (a) => {
    const p = shimUserPresets.find((x) => x.id === Number(a?.id));
    if (p) p.name = String(a?.name);
    return null;
  },
  delete_preset: (a) => {
    const i = shimUserPresets.findIndex((x) => x.id === Number(a?.id));
    if (i >= 0) shimUserPresets.splice(i, 1);
    return null;
  },
  list_history: (a) => shimHistory.get(Number(a?.assetId)) ?? [],
  push_history: (a) => {
    const list = shimHistory.get(Number(a?.assetId)) ?? [];
    list.push({ id: shimStep++, label: String(a?.label), recipe: a?.recipe, snapshotName: null, createdAt: Date.now() });
    shimHistory.set(Number(a?.assetId), list);
    return shimStep - 1;
  },
  truncate_history: (a) => {
    const list = shimHistory.get(Number(a?.assetId)) ?? [];
    shimHistory.set(Number(a?.assetId), list.filter((s) => s.snapshotName !== null || s.id <= Number(a?.stepId)));
    return 0;
  },
  clear_history: (a) => {
    shimHistory.set(Number(a?.assetId), (shimHistory.get(Number(a?.assetId)) ?? []).filter((s) => s.snapshotName !== null));
    return 0;
  },
  save_snapshot: (a) => {
    const list = shimHistory.get(Number(a?.assetId)) ?? [];
    list.push({ id: shimStep++, label: String(a?.name), recipe: a?.recipe, snapshotName: String(a?.name), createdAt: Date.now() });
    shimHistory.set(Number(a?.assetId), list);
    return shimStep - 1;
  },
  rename_snapshot: (a) => {
    for (const list of shimHistory.values()) for (const s of list) if (s.id === Number(a?.id)) s.snapshotName = s.label = String(a?.name);
    return null;
  },
  delete_snapshot: (a) => {
    for (const [k, list] of shimHistory) shimHistory.set(k, list.filter((s) => s.id !== Number(a?.id)));
    return null;
  },
  paste_develop: (a) => (a?.ids as number[]).map((id) => ({ assetId: id, recipe: null })),
  restore_recipes: () => null,
  rotate_assets: (a) => ((a?.ids as number[]) ?? []).map((id) => ({ assetId: id, recipe: null })),
  // Two photos (one edited) and a trimmed video, so the sheet shows every control.
  export_summary: () => ({ photos: 2, videos: 1, trimmed: 1, edited: 1 }),
  list_memories: (a) => shimMemories.filter((m) => a?.includeHidden || !m.hidden),
  get_memory: (a) => shimMemories.find((m) => m.id === Number(a?.id)) ?? shimMemories[0],
  memory_curated_ids: (a) => items.slice(Number(a?.id) * 20, Number(a?.id) * 20 + 12).map((i) => i.id),
  set_memory_hidden: (a) => {
    const m = shimMemories.find((x) => x.id === Number(a?.id));
    if (m) m.hidden = Boolean(a?.hidden);
    return null;
  },
  regenerate_memories: () => null,
  list_slideshows: () => shimShows.map(summarize),
  get_slideshow: (a) => shimShows.find((s) => s.id === Number(a?.id)) ?? shimShows[0],
  create_slideshow: (a) => {
    const show = { id: nextShowId++, name: String(a?.name ?? "Slideshow"), doc: shimDoc((a?.assetIds as number[]) ?? []), updatedAt: NOW };
    shimShows.unshift(show);
    return show.id;
  },
  // Round 6: the part of a song, clips for what the editor adds, a video's best part.
  music_window: () => ({ start_ms: 32_000, fade_in_ms: 400, fade_out_ms: 1000 }),
  slideshow_clips: (a) => shimDoc((a?.assetIds as number[]) ?? []).clips,
  video_best_part: () => ({ in_ms: 2000, out_ms: 6500 }),
  create_slideshow_from_memory: (a) => {
    const memory = shimMemories.find((m) => m.id === Number(a?.memoryId));
    const show = { id: nextShowId++, name: memory?.title ?? "Memory", doc: shimDoc(items.slice(0, 10).map((i) => i.id)), updatedAt: NOW };
    shimShows.unshift(show);
    return show.id;
  },
  memory_film: async (a) => {
    // A pause long enough to see the film being made.
    await new Promise((r) => setTimeout(r, 1500));
    const memory = shimMemories.find((m) => m.id === Number(a?.memoryId));
    return { name: memory?.title ?? "Memory", doc: shimDoc(items.slice(0, 10).map((i) => i.id)), bpm: 92 };
  },
  save_memory_film: (a) => {
    const memory = shimMemories.find((m) => m.id === Number(a?.memoryId));
    const show = { id: nextShowId++, name: memory?.title ?? "Memory", doc: shimDoc(items.slice(0, 10).map((i) => i.id)), updatedAt: NOW };
    shimShows.unshift(show);
    return show.id;
  },
  save_slideshow: (a) => {
    const show = shimShows.find((s) => s.id === Number(a?.id));
    if (show) {
      show.name = String(a?.name ?? show.name);
      show.doc = a?.doc as typeof show.doc;
      show.updatedAt = Math.floor(Date.now() / 1000);
    }
    return null;
  },
  delete_slideshow: (a) => {
    const at = shimShows.findIndex((s) => s.id === Number(a?.id));
    return at === -1 ? null : shimShows.splice(at, 1)[0];
  },
  export_slideshow: () => null,
  set_face_radar_view: () => null,
  music_search: (a) => {
    const q = String((a?.query as { text?: string })?.text ?? "").toLowerCase();
    return shimMusic.filter((t) => !q || t.title.toLowerCase().includes(q) || t.artist.toLowerCase().includes(q));
  },
  music_download: (a) => {
    const track = a?.track as (typeof shimMusic)[number];
    const saved = { ...track, credit: `“${track.title}” by ${track.artist} — ${track.licenceName}`, path: `C:/Lumen Library/music/${track.id}.mp3` };
    shimSaved.push(saved);
    return saved;
  },
  list_library_music: () => shimSaved,
  delete_library_music: (a) => {
    const at = shimSaved.findIndex((t) => t.path === a?.path);
    if (at !== -1) shimSaved.splice(at, 1);
    return null;
  },
  // The shim has no FFmpeg, so it hands the show back unchanged with a plausible tempo.
  sync_slideshow_to_music: (a) => {
    const show = shimShows.find((x) => x.id === Number(a?.id));
    return { ...(show ?? shimShows[0]), bpm: 120 };
  },
  rebuild_slideshow: (a) => {
    const show = shimShows.find((x) => x.id === Number(a?.id));
    return show ?? shimShows[0];
  },
  render_edit_preview: (a) => String(a?.assetId ?? 1),
  list_mac_disks: () => [
    { id: "disk3-part2", name: "ADATA HD710 PRO", fs: "hfs", totalBytes: 2_000_154_525_696, freeBytes: 834_726_649_856, libraries: ["/Photo Library/Julian's Photo Library.photoslibrary"] },
  ],
  list_mac_folder: (a) => {
    const path = String(a?.path ?? "/");
    const tree: Record<string, { folders: string[]; mediaHere: number }> = {
      "/": { folders: ["Documents", "DSLR footage", "mami photos", "Photo Library", "transfer"], mediaHere: 0 },
      "/mami photos": { folders: [], mediaHere: 212 },
      "/DSLR footage": { folders: ["2023", "2024"], mediaHere: 18 },
    };
    return { path, ...(tree[path] ?? { folders: [], mediaHere: 40 }) };
  },
  start_mac_folder_import: () => 45,
  get_photos_library: (a) => ({
    summary: { name: String(a?.path ?? "").split("/").pop()?.replace(/\.photoslibrary$/, "") ?? "Photos Library", items: 34481, livePhotos: 728, notDownloaded: 0, referenced: 0, burstFrames: 4280, trashed: 0 },
    totalFiles: 35209,
    newItems: 35209,
    newest: items.slice(12, 24).map((i, n) => ({ token: String(i.id), fileName: `IMG_${String(9000 - n).padStart(4, "0")}.HEIC`, modifiedAt: null, video: n === 5 })),
  }),
  start_photos_library_import: () => 46,
  get_connected_device: (a) => {
    if (a?.kind === "mac") {
      return { kind: "mac", id: a.id, name: "ADATA HD710 PRO", storage: { totalBytes: 2_000_154_525_696, freeBytes: 834_726_649_856 }, totalItems: 0, newItems: 0, totalBytes: 0, newest: [], removable: true };
    }
    const phone = a?.kind === "iphone";
    return {
      kind: a?.kind,
      id: a?.id,
      name: phone ? "Julian's iPhone" : "EOS_DIGITAL",
      storage: phone ? { totalBytes: 128_000_000_000, freeBytes: 9_400_000_000 } : { totalBytes: 128_000_000_000, freeBytes: 20_100_000_000 },
      totalItems: 312,
      newItems: 48,
      totalBytes: 41_000_000_000,
      newest: items.slice(0, 12).map((i, n) => ({ token: String(i.id), fileName: `IMG_${String(312 - n).padStart(4, "0")}.${phone ? "HEIC" : "CR3"}`, modifiedAt: null, video: n === 3 })),
      removable: true,
    };
  },
  // The device browser (Round 4E): 6,000 items on a pretend card, 40 a day, the oldest
  // 500 already imported and every twelfth a Live Photo, enough to measure scrolling.
  open_device_browser: () => deviceSession(),
  get_device_browser: () => deviceSession(),
  browse_device: (a) => {
    const all = deviceMatches(a.filter as DeviceFilter);
    const start = (a.cursor as number | null) ?? 0;
    const end = Math.min(all.length, start + Number(a.limit ?? 200));
    return { items: all.slice(start, end), nextCursor: end < all.length ? end : null, matching: all.length, generation: 0 };
  },
  device_browser_ids: (a) => deviceMatches(a.filter as DeviceFilter).map((i) => i.index),
  device_browser_selection: (a) => {
    const picked = new Set(a.indexes as number[]);
    const chosen = deviceItems.filter((i) => picked.has(i.index));
    return { count: chosen.length, files: chosen.reduce((n, i) => n + i.files, 0), bytes: chosen.reduce((n, i) => n + i.size, 0), imported: chosen.filter((i) => i.imported).length };
  },
  close_device_browser: () => null,
  plan_device_browser_import: (a) => {
    const count = (a.indexes as number[]).length;
    return { count, bytes: count * 24_000_000, freeBytes: 400_000_000_000, fitsCount: count, fitsBytes: count * 24_000_000, buffer: 20_000_000_000 };
  },
  start_device_browser_import: () => 47,
  list_collections: () => {
    const first = (f: (i: Item) => boolean) => items.find(f)?.id ?? null;
    const col = (id: string, query: Record<string, unknown>, f: (i: Item) => boolean) => ({
      id,
      query: { text: null, dateFrom: null, dateTo: null, deviceIds: [], mediaType: null, favorites: false, minRating: null, hasGps: null, camera: null, lens: null, kind: null, ...query },
      count: items.filter(f).length,
      coverAssetId: first(f),
    });
    return [
      col("favorites", { favorites: true }, (i) => i.id % 7 === 0),
      col("videos", { mediaType: 1 }, (i) => i.video),
      ...(["live", "raw", "hdr", "screenshot", "panorama", "selfie"] as Kind[]).map((k) =>
        col(k === "screenshot" ? "screenshots" : k === "panorama" ? "panoramas" : k === "selfie" ? "selfies" : k, { kind: k }, (i) => hasKind(i, k)),
      ),
    ];
  },
  reveal_asset: () => null,
  list_unreadable: () => ({
    items: [
      { id: 6752, filename: "IMG_1855.JPG", relativePath: "2023/IMG_1855.JPG", extension: "jpg", byteSize: 2_300_000, contentHash: "ab12", cause: "incomplete" },
      { id: 6945, filename: "IMG_1768.HEIC", relativePath: "2023/IMG_1768.HEIC", extension: "heic", byteSize: 4_100_000, contentHash: "cd34", cause: "notAnImage" },
      { id: 7001, filename: "Panorama_big.TIF", relativePath: "2022/Panorama_big.TIF", extension: "tif", byteSize: 640_000_000, contentHash: "ef56", cause: "tooLarge" },
    ],
    nextCursor: null,
  }),
  // Round 22 lane B: Settings → Storage's temporary files. `?temp=big` is above the 2 GB warning.
  get_temp_usage: () => {
    const big = params.get("temp") === "big";
    const rows = [
      { kind: "keptAside", bytes: big ? 2_400_000_000 : 310_000_000, files: 12 },
      { kind: "unfinished", bytes: 48_000_000, files: 2 },
      { kind: "recovered", bytes: 6_000_000, files: 4 },
      { kind: "dngCache", bytes: big ? 1_200_000_000 : 220_000_000, files: 9 },
    ];
    const total = rows.reduce((s, r) => s + r.bytes, 0);
    return { rows, total, warn: total > 2 * 1024 ** 3 };
  },
  clear_temp: () => 584_000_000,
  // Round 22 lane B: the Repair page. `?repair=empty` shows a library with nothing wrong.
  get_library_health: () =>
    params.get("repair") === "empty"
      ? { total: 21_865, unreadable: 0, missing: 0, recovered: 0, checkedAt: NOW - 7200, causes: [] }
      : {
          total: 21_865,
          unreadable: 3,
          missing: 2,
          recovered: 1,
          checkedAt: NOW - 7200,
          causes: [
            { cause: "incomplete", n: 1 },
            { cause: "notAnImage", n: 1 },
            { cause: "tooLarge", n: 1 },
          ],
        },
  get_recovered: () => (params.get("repair") === "empty" ? [] : [{ assetId: 6752, name: "IMG_1855.JPG", kind: "partial", percent: 62, width: 4032, height: 3024 }]),
  get_repair_outcomes: () =>
    params.get("repair") === "empty"
      ? []
      : [
          { name: "IMG_0042.HEIC", dir: "100APPLE", outcome: "repaired", reason: null, assetId: 6945 },
          { name: "IMG_0043.HEIC", dir: "100APPLE", outcome: "sourceDamaged", reason: null, assetId: 6752 },
          { name: "IMG_0107.MOV", dir: "101APPLE/clips", outcome: "failed", reason: "the card was removed", assetId: null },
        ],
  start_recover: () => null,
  keep_recovered: (a: Record<string, unknown>) => ({ done: a.ids, failed: [] }),
  undo_keep_recovered: (a: Record<string, unknown>) => ({ done: a.ids, failed: [] }),
  discard_recovered: () => null,
  // Import · Repair → check a folder or card: a card with one file the library lacks.
  get_check_status: () => ({
    running: null,
    folder: { folder: "E:/DCIM-backup/Card 3", checkedAt: NOW - 600, safe: 412, notInLibrary: 2, other: 3, cancelled: false, safeToDelete: false },
    library: null,
    lastFullCheckAt: NOW - 86400 * 9,
  }),
  get_check_items: (a: Record<string, unknown>) => ({
    items:
      a.status === "notInLibrary"
        ? [
            { path: "100APPLE/IMG_0042.HEIC", status: "notInLibrary", reason: "not_imported", assetId: null, byteSize: 3_100_000 },
            { path: "100APPLE/IMG_0043.AAE", status: "notInLibrary", reason: "sidecar_not_kept", assetId: null, byteSize: 900 },
          ]
        : [{ path: "Thumbs.db", status: "other", reason: null, assetId: null, byteSize: 24_000 }],
    nextCursor: null,
  }),
  get_check_problems: () => ({ items: [], nextCursor: null }),
  // Round 21 lane B: Import → Repair. A card and an unpaired phone; a check that finds five things.
  list_repair_sources: () => [
    { ref: { kind: "volume", id: "E:/" }, name: "EOS_DIGITAL", detail: "E: · 58 GB · removable", ready: true, removable: true },
    { ref: { kind: "iphone", id: "00008110-001A" }, name: "Julian's iPhone", detail: "Unlock the phone and tap Trust", ready: false, removable: false },
  ],
  start_repair_check: () => null,
  cancel_repair: () => null,
  // `?repair=idle` shows the source step, `?repair=running` the progress, anything else the result.
  get_repair_status: () => ({ phase: ({ idle: "idle", running: "checking" } as Record<string, string>)[params.get("repair") ?? ""] ?? "done", done: 140, total: 417, counts: { safe: 412, new: 3, damaged: 1, sourceDamaged: 1, sidecar: 1, inTrash: 0 }, cancelled: false, current: "IMG_0042.HEIC", bytesDone: 0, bytesTotal: 0 }),
  get_repair_items: (a: Record<string, unknown>) => ({
    items: [
      { id: 1, name: "IMG_0042.HEIC", dir: "100APPLE", status: a.status, assetId: null, byteSize: 3_100_000 },
      { id: 2, name: "IMG_0043.HEIC", dir: "100APPLE", status: a.status, assetId: null, byteSize: 2_800_000 },
      { id: 3, name: "IMG_0107.MOV", dir: "101APPLE/clips", status: a.status, assetId: null, byteSize: 48_000_000 },
    ],
    nextCursor: null,
  }),
  start_repair_import: () => null,
  start_check_folder: () => null,
  start_check_library: () => null,
  cancel_check: () => null,
  save_check_report: () => "D:/Reports/Lumen folder check.txt",
  retry_unreadable: () => 2,
  count_unreadable: () => 2,
  trashed_unreadable_ids: () => [],
  unreadable_source_folders: () => [],
  list_devices: () => [
    { id: 1, displayName: "iPhone 17 Pro", kind: 1, icon: null, itemCount: 812, lastImportAt: NOW - 3600, hidden: false, group: "iphone" },
    { id: 4, displayName: "iPhone 13", kind: 1, icon: null, itemCount: 2210, lastImportAt: NOW - 86_400, hidden: false, group: "iphone" },
    { id: 2, displayName: "Sony ILCE-7M4", kind: 2, icon: null, itemCount: 436, lastImportAt: NOW - 86_400 * 3, hidden: false, group: "camera" },
    { id: 5, displayName: "NIKON Z 6II", kind: 2, icon: null, itemCount: 97, lastImportAt: NOW - 86_400 * 9, hidden: false, group: "camera" },
    { id: 6, displayName: "DJI Mini 4 Pro", kind: 2, icon: null, itemCount: 41, lastImportAt: NOW - 86_400 * 20, hidden: false, group: "drone" },
    { id: 3, displayName: "Old phone", kind: 1, icon: null, itemCount: 12, lastImportAt: NOW - 86_400 * 300, hidden: true, group: "phone" },
  ],
  list_iphones: () => ({ usbmuxdAvailable: true, devices: [{ udid: "00008120-000A1B2C3D4E", name: "Julian's iPhone", model: "iPhone16,1", paired: true }] }),
  start_iphone_import: () => 43,
  list_volumes: () => [{ mount: "E:\\", label: "EOS_DIGITAL", key: "vol:EOS_DIGITAL:64000000000", totalItems: 312, newItems: 48, removable: true }],
  rename_device: () => null,
  set_device_hidden: () => null,
  start_volume_import: () => 42,
  forget_volume: () => 312,
  get_exiftool_status: () => ({ available: true, version: "13.59", path: "shim", envVar: "LUMEN_EXIFTOOL_DIR" }),
  get_raw_formats_status: () => ({ converter: "found", converterPath: "C:\\Program Files\\Adobe\\Adobe DNG Converter\\Adobe DNG Converter.exe", cacheBytes: 1_830_000_000, cacheFiles: 37, envVar: "LUMEN_DNG_CONVERTER" }),
  clear_dng_cache: () => 1_830_000_000,
  get_ffmpeg_status: () => ({ available: true, version: "ffmpeg version 7.1 (browser shim)", path: "shim", envVar: "LUMEN_FFMPEG_DIR" }),
  get_playback: (a) => ({ mode: "native", proxyReady: false, videoCodec: "h264", audioCodec: "aac", container: "mov,mp4", durationMs: 12_000 + Number(a.id) * 300, width: 1920, height: 1080 }),
  prepare_proxy: () => null,
  get_proxy_cache_size: () => 48_000_000,
  clear_proxy_cache: () => 48_000_000,
  prefetch_previews: () => null,
  find_apple_photos_libraries: () => [],
  prefetch_playback: () => null,
  set_video_support: () => null,
  set_favorite: () => 1,
  trash_assets: () => 1,
  restore_assets: () => 1,
  delete_now: () => 1,
  empty_trash: () => 0,
  get_favorite_ids: () => items.slice(0, 8).map((i) => i.id),
  list_albums: () => albums,
  get_album: (a) => albums.find((x) => x.id === Number(a.id)),
  // The Hidden folder: the password is "lumen"; nothing is really hidden in the shim.
  vault_status: () => ({ hasPassword: shimVault.password !== null, count: shimVault.ids.size, unlocked: shimVault.unlocked }),
  vault_set_password: (a) => {
    shimVault.password = String(a?.password);
    shimVault.unlocked = true;
    return null;
  },
  vault_reset_password: (a) => {
    shimVault.password = String(a?.password);
    shimVault.unlocked = true;
    return null;
  },
  vault_unlock: (a) => (shimVault.unlocked = a?.password === shimVault.password),
  vault_lock: () => {
    shimVault.unlocked = false;
    return null;
  },
  move_to_vault: (a) => {
    for (const id of (a?.ids as number[]) ?? []) shimVault.ids.add(id);
    return a?.ids;
  },
  move_from_vault: (a) => {
    for (const id of (a?.ids as number[]) ?? []) shimVault.ids.delete(id);
    return a?.ids;
  },
  // Plenty of room, unless ?space=tight asks for the warning.
  plan_import: () =>
    new URLSearchParams(location.search).get("space") === "tight"
      ? { count: 2310, bytes: 52e9, freeBytes: 61.2e9, fitsCount: 1850, fitsBytes: 41.2e9, buffer: 20e9 }
      : { count: 48, bytes: 1.2e9, freeBytes: 400e9, fitsCount: 48, fitsBytes: 1.2e9, buffer: 20e9 },
  set_album_hides: (a) => {
    const al = albums.find((x) => x.id === Number(a?.id));
    if (al) al.hidesItems = Boolean(a?.hides);
    return null;
  },
  create_album: (a) => {
    const al: AlbumSummary = { id: albums.length + 1, name: String(a.name), kind: 0, parentId: null, hidesItems: false, coverAssetId: null, itemCount: 0, createdAt: NOW, updatedAt: NOW, query: null, collectionId: null, coverStyle: null, previewIds: [] };
    albums.unshift(al);
    return al;
  },
  create_smart_album: (a) => {
    const al: AlbumSummary = {
      id: albums.length + 1,
      name: String(a.name),
      kind: 1,
      parentId: null,
      hidesItems: false,
      coverAssetId: items[6].id,
      itemCount: 42,
      createdAt: NOW,
      updatedAt: NOW,
      query: (a.query as AlbumSummary["query"]) ?? null,
      collectionId: null,
      coverStyle: null,
      previewIds: [items[6].id, items[7].id, items[8].id],
    };
    albums.unshift(al);
    return al;
  },
  update_smart_album: () => null,
  rename_album: (a) => { const al = albums.find((x) => x.id === Number(a.id)); if (al) al.name = String(a.name); return null; },
  delete_album: (a) => { const i = albums.findIndex((x) => x.id === Number(a.id)); if (i >= 0) albums.splice(i, 1); return null; },
  add_to_album: (a) => (a.assetIds as number[]).length,
  remove_from_album: (a) => (a.assetIds as number[]).length,
  move_in_album: () => null,
  set_album_cover: (a) => {
    const al = albums.find((x) => x.id === Number(a.albumId));
    if (al && a.assetId != null) Object.assign(al, { coverAssetId: Number(a.assetId), coverStyle: null });
    return null;
  },
  // New automatic prints: the album's photos in another order.
  shuffle_album_cover: (a) => {
    const al = albums.find((x) => x.id === Number(a.id));
    if (al) al.previewIds = [...al.previewIds.slice(1), ...al.previewIds.slice(0, 1)];
    return null;
  },
  set_album_cover_style: (a) => {
    const al = albums.find((x) => x.id === Number(a.albumId));
    if (al) al.coverStyle = (a.style as AlbumSummary["coverStyle"]) ?? null;
    return null;
  },
  set_collection_cover_style: (a) => {
    const c = albumCollections.find((x) => x.id === Number(a.id));
    if (c) c.coverStyle = (a.style as AlbumCollection["coverStyle"]) ?? null;
    return null;
  },
  // Backup (Round 5A): a destination that is there, last run yesterday.
  get_backup: () => shimBackup,
  set_backup_destination: (a) => {
    shimBackup.config.destination = (a.destination as string | null) ?? null;
    shimBackup.target = shimBackup.config.destination ? `${shimBackup.config.destination}/Lumen Library` : null;
    return shimBackup;
  },
  set_backup_schedule: (a) => {
    shimBackup.config.schedule = a.schedule as BackupStatus["config"]["schedule"];
    return shimBackup;
  },
  start_backup: () => null,
  cancel_backup: () => null,
  list_album_collections: () => albumCollections,
  create_album_collection: (a) => {
    const id = Math.max(0, ...albumCollections.map((c) => c.id)) + 1;
    albumCollections.push({ id, name: String(a.name), parentId: (a.parentId as number | null) ?? null, createdAt: NOW, updatedAt: NOW, coverStyle: null });
    return id;
  },
  rename_album_collection: (a) => { const c = albumCollections.find((x) => x.id === Number(a.id)); if (c) c.name = String(a.name); return null; },
  move_album_to_collection: (a) => {
    const al = albums.find((x) => x.id === Number(a.albumId));
    const was = al?.collectionId ?? null;
    if (al) al.collectionId = (a.collectionId as number | null) ?? null;
    return was;
  },
  move_album_collection: (a) => {
    const c = albumCollections.find((x) => x.id === Number(a.id));
    const was = c?.parentId ?? null;
    if (c) c.parentId = (a.into as number | null) ?? null;
    return was;
  },
  delete_album_collection: (a): CollectionUndo => {
    const i = albumCollections.findIndex((x) => x.id === Number(a.id));
    const [collection] = albumCollections.splice(i, 1);
    const held = albums.filter((x) => x.collectionId === collection.id);
    const kids = albumCollections.filter((x) => x.parentId === collection.id);
    held.forEach((x) => (x.collectionId = collection.parentId));
    kids.forEach((x) => (x.parentId = collection.parentId));
    return { collection, albums: held.map((x) => x.id), collections: kids.map((x) => x.id) };
  },
  restore_album_collection: (a) => {
    const u = a.undo as CollectionUndo;
    albumCollections.push(u.collection);
    albums.filter((x) => u.albums.includes(x.id)).forEach((x) => (x.collectionId = u.collection.id));
    albumCollections.filter((x) => u.collections.includes(x.id)).forEach((x) => (x.parentId = u.collection.id));
    return null;
  },

  // Round 20 lane C: Home's summary (drafts, unreadable files, the four biggest places).
  home_summary: () => ({
    drafts: items.slice(2, 8).map((i) => i.id),
    draftCount: 8,
    unreadable: 2,
    peopleToReview: 0,
    places: [
      { place: { kind: "city", name: "Lisbon", country: "Portugal" }, label: "Lisbon, Portugal", photoCount: 286, lat: 38.72, lon: -9.14, coverAssetId: items[0]?.id ?? null },
      { place: { kind: "city", name: "Idrija", country: "Slovenia" }, label: "Idrija, Slovenia", photoCount: 144, lat: 46.0, lon: 14.03, coverAssetId: items[1]?.id ?? null },
      { place: { kind: "city", name: "Florence", country: "Italy" }, label: "Florence, Italy", photoCount: 97, lat: 43.77, lon: 11.25, coverAssetId: items[2]?.id ?? null },
      { place: { kind: "city", name: "Oslo", country: "Norway" }, label: "Oslo, Norway", photoCount: 41, lat: 59.91, lon: 10.75, coverAssetId: items[3]?.id ?? null },
    ],
  }),
};

// Phase 8f: Facet's masks on any photo, as `photos://ai-mask/` would serve them (u32 width and
// height, then weights): a sky over the top third, a subject as a soft ellipse, two people.
const SHIM_AI = { sky: "a".repeat(32), subject: "b".repeat(32), background: "c".repeat(32), p0: "d".repeat(32), p1: "e".repeat(32) };
let shimScanned = false;
const shimAiList = () => ({
  available: true,
  scanned: shimScanned,
  masks: shimScanned
    ? [
        { hash: SHIM_AI.sky, kind: "sky", instance: 0, model: "facet@1", score: 0.98, width: 96, height: 64 },
        { hash: SHIM_AI.subject, kind: "subject", instance: 0, model: "facet@1", score: 0.95, width: 96, height: 64 },
        { hash: SHIM_AI.background, kind: "background", instance: 0, model: "facet@1", score: 0.95, width: 96, height: 64 },
        { hash: SHIM_AI.p0, kind: "person", instance: 0, model: "facet@1", score: 0.93, width: 96, height: 64 },
        { hash: SHIM_AI.p1, kind: "person", instance: 1, model: "facet@1", score: 0.91, width: 96, height: 64 },
      ]
    : [],
});
function shimRaster(hash: string): string {
  const [w, h] = [96, 64];
  const out = new Uint8Array(8 + w * h);
  new DataView(out.buffer).setUint32(0, w, true);
  new DataView(out.buffer).setUint32(4, h, true);
  const smooth = (e0: number, e1: number, x: number) => {
    const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
    return t * t * (3 - 2 * t);
  };
  const ellipse = (x: number, y: number, cx: number, cy: number, rx: number, ry: number) => 1 - smooth(0.85, 1.05, Math.hypot((x - cx) / rx, (y - cy) / ry));
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const [u, v] = [(x + 0.5) / w, (y + 0.5) / h];
      const subject = Math.max(ellipse(u, v, 0.38, 0.62, 0.12, 0.3), ellipse(u, v, 0.62, 0.64, 0.11, 0.28));
      const weight =
        hash === SHIM_AI.sky ? (1 - smooth(0.3, 0.38, v)) * (1 - subject)
        : hash === SHIM_AI.subject ? subject
        : hash === SHIM_AI.background ? 1 - subject
        : hash === SHIM_AI.p0 ? ellipse(u, v, 0.38, 0.62, 0.12, 0.3)
        : hash === SHIM_AI.p1 ? ellipse(u, v, 0.62, 0.64, 0.11, 0.28)
        : 0;
      out[8 + y * w + x] = Math.round(weight * 255);
    }
  let bin = "";
  out.forEach((b) => (bin += String.fromCharCode(b)));
  return `data:application/octet-stream;base64,${btoa(bin)}`;
}

(window as unknown as { __LUMEN_MEDIA_URL__: unknown }).__LUMEN_MEDIA_URL__ = (
  kind: string,
  id: number | string,
  p?: Record<string, string | number>,
) => {
  if (kind === "ai-mask") return shimRaster(String(id).split("/")[1] ?? "");
  const it = items.find((i) => i.id === Number(id)) ?? items[0];
  // Like `protocol.rs`: thumbnails come at 256 and 1024 only; any other size is a 404.
  if (kind === "thumb" && ![256, 1024].includes(Number(p?.s ?? 256))) return "data:,not-found";
  const s = kind === "thumb" ? Number(p?.s ?? 256) : kind === "device-thumb" ? 320 : kind === "preview" ? 1600 : 2400; // "full"/"original" → 2400
  const w = it.aspect >= 1 ? s : Math.round(s * it.aspect);
  const h = it.aspect >= 1 ? Math.round(s / it.aspect) : s;
  return `https://picsum.photos/seed/lumen${id}/${w}/${h}`;
};

(window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
  transformCallback: (cb?: (r: unknown) => void) => {
    const id = Math.floor(Math.random() * 1e9);
    (window as unknown as Record<string, unknown>)[`_${id}`] = cb;
    return id;
  },
  convertFileSrc: (p: string) => p,
  invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
    const h = commands[cmd];
    if (h) return h(args);
    if (cmd.startsWith("plugin:")) return null; // window/dialog/event plugin calls are no-ops here
    throw { code: "unmocked", message: `No browser mock for ${cmd}` };
  },
};

console.info("[lumen] browser shim: ?library=1 skips setup, ?theme=glass&scheme=light forces a theme, ?nosplash=1, ?items=200");

// Round 24 lane A: the system clipboard in memory, so Copy / Paste settings need no permission prompt.
{
  let clipText = "";
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: {
      writeText: async (t: string) => {
        clipText = t;
      },
      readText: async () => clipText,
    },
  });
}
