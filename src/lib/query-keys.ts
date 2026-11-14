import type { GroupBy } from "@/types/generated/GroupBy";
import type { LayoutScope } from "@/types/generated/LayoutScope";

/** JSON with keys sorted at every level, so two equal values always give one string. */
export const stable = (v: unknown): string =>
  Array.isArray(v)
    ? `[${v.map(stable).join(",")}]`
    : v && typeof v === "object"
      ? `{${Object.keys(v)
          .sort()
          .filter((k) => (v as Record<string, unknown>)[k] !== undefined)
          .map((k) => `${JSON.stringify(k)}:${stable((v as Record<string, unknown>)[k])}`)
          .join(",")}}`
      : JSON.stringify(v ?? null);

/**
 * Every field of the scope, so no two scopes share a cached layout. Listing fields by hand
 * left out `personId`: every person's page showed the first person's photos.
 */
export const scopeKey = (s: LayoutScope | null) => (s ? stable(s) : "");

export const queryKeys = {
  settings: ["settings"] as const,
  libraries: ["libraries"] as const,
  currentLibrary: ["library", "current"] as const,
  missingLibrary: ["library", "missing"] as const,
  libraryProblem: ["library", "problem"] as const,
  updateStatus: ["updates", "status"] as const,
  appInfo: ["app-info"] as const,
  mlStatus: ["ml-status"] as const,
  experimentalCatalog: ["experimental-catalog"] as const,
  // Round 22 lane C: the installed editors and the default per kind of media.
  externalApps: ["external-apps"] as const,
  defaultApps: ["external-apps", "defaults"] as const,
  layout: (width: number, rowHeight: number, gap: number, scope: LayoutScope | null, groupBy: GroupBy = "month") =>
    ["layout", width, rowHeight, gap, scopeKey(scope), groupBy] as const,
  layoutRows: (width: number, rowHeight: number, gap: number, scope: LayoutScope | null, chunk: number, groupBy: GroupBy = "month", fingerprint = "") =>
    ["layout-rows", width, rowHeight, gap, scopeKey(scope), groupBy, fingerprint, chunk] as const,
  asset: (id: number) => ["asset", id] as const,
  neighbors: (id: number) => ["neighbors", id] as const,
  // Under "neighbors", so a library change invalidates both.
  scopeNeighbors: (scope: unknown, id: number) => ["neighbors", "scope", scope, id] as const,
  sectionNeighbors: (scope: unknown, id: number, section: number | null) => ["neighbors", "section", scope, id, section] as const,
  stats: ["stats"] as const,
  /** Home's drafts, unreadable count and biggest places (Round 20 C); under "stats" so it refreshes with them. */
  homeSummary: ["stats", "home"] as const,
  /** What the tag search offers for a text (Round 7); refreshed with the library. */
  tagSuggestions: (input: string) => ["tag-suggestions", input] as const,
  /** A typed sentence read into tags (Round 20 §2); refreshed with the library. */
  searchSentence: (input: string) => ["search-sentence", input] as const,
  duplicates: ["duplicates"] as const,
  duplicateCount: ["duplicates", "count"] as const,
  batches: ["batches"] as const,
  failures: (batchId: number) => ["failures", batchId] as const,
  skips: (batchId: number) => ["skips", batchId] as const,
  recent: (batchId: number) => ["recent", batchId] as const,
  albums: ["albums"] as const,
  backup: ["backup"] as const,
  /** Under `albums`, so whatever refreshes albums refreshes collections too (Round 5A). */
  albumCollections: ["albums", "collections"] as const,
  vault: ["vault"] as const,
  album: (id: number) => ["album", id] as const,
  favorites: ["favorites"] as const,
  ffmpeg: ["ffmpeg"] as const,
  exiftool: ["exiftool"] as const,
  rawFormats: ["raw-formats"] as const,
  /** Import · Repair's files Lumen can't open (Round 18 C1; moved from Settings in Round 22 B6). */
  unreadable: ["unreadable"] as const,
  /** Flagged files in the Trash (under `unreadable`, so a library change refreshes it too). */
  unreadableTrashed: ["unreadable", "trashed"] as const,
  /** *Restore…*: the folders the files came from and the ids an `assets=` value names. */
  restoreFolders: (assets: string) => ["unreadable", "folders", assets] as const,
  restoreIds: (assets: string) => ["unreadable", "ids", assets] as const,
  /** Import · Repair's library check (Round 19 C4, moved from Settings in Round 22 B6); the lists hang below it. */
  check: ["check"] as const,
  checkItems: (verdict: string) => ["check", "items", verdict] as const,
  checkProblems: ["check", "problems"] as const,
  /** Import → Repair (Round 21 B4): the sources, the running check, and a filter's rows. */
  repairSources: ["repair", "sources"] as const,
  repairStatus: ["repair", "status"] as const,
  repairItems: (status: string) => ["repair", "items", status] as const,
  /** The last re-import's `RepairResult`, set by `repair://imported` (never fetched). */
  repairImported: ["repair-imported"] as const,
  /** Round 22 lane B: the Repair page's health card, flagged rows, recoveries and last run. */
  repairHealth: ["repair", "health"] as const,
  repairRecovered: ["repair", "recovered"] as const,
  repairOutcomes: ["repair", "outcomes"] as const,
  unreadableRows: ["unreadable", "rows"] as const,
  devices: ["devices"] as const,
  collections: ["collections"] as const,
  volumes: ["volumes"] as const,
  iphones: ["iphones"] as const,
  connected: (kind: string, id: string) => ["connected", kind, id] as const,
  macDisks: ["mac-disks"] as const,
  appleLibraries: ["apple-libraries"] as const,
  lightroomCatalogs: ["lightroom-catalogs"] as const,
  editVersions: ["edit-versions"] as const,
  memories: (includeHidden: boolean) => ["memories", includeHidden] as const,
  people: ["people", "listed"] as const,
  removedPeople: ["people", "removed"] as const,
  /** Round 4C. */
  pets: ["pets", "listed"] as const,
  removedPets: ["pets", "removed"] as const,
  slideshows: ["slideshows"] as const,
  libraryMusic: ["library-music"] as const,
  musicSearch: (query: unknown) => ["music-search", JSON.stringify(query)] as const,
  slideshow: (id: number) => ["slideshow", id] as const,
  memory: (id: number) => ["memory", id] as const,
  edit: (assetId: number) => ["edit", assetId] as const,
  /** Round 6: what Develop shows, which the library never does until export. */
  developDraft: (assetId: number) => ["develop-draft", assetId] as const,
  /** Phase 8f: what Lumen Facet found on a photo, and each raster's bytes by hash. */
  aiMasks: (assetId: number) => ["ai-masks", assetId] as const,
  aiRaster: (assetId: number, hash: string) => ["ai-raster", assetId, hash] as const,
  macFolder: (disk: string, path: string) => ["mac-folder", disk, path] as const,
  photosLibrary: (disk: string | null, path: string) => ["photos-library", disk ?? "local", path] as const,
  /** Everything under `session(id)` is refetched when the backend says the session changed. */
  deviceBrowser: {
    session: (id: number) => ["device-browser", id] as const,
    summary: (id: number) => ["device-browser", id, "summary"] as const,
    pages: (id: number, filter: string) => ["device-browser", id, "pages", filter] as const,
    ids: (id: number, filter: string) => ["device-browser", id, "ids", filter] as const,
    selection: (id: number, version: number) => ["device-browser", id, "selection", version] as const,
  },
  count: (scope: LayoutScope | null) => ["count", scopeKey(scope)] as const,
  searchOverview: (q: string) => ["search-overview", q] as const,
  playback: (id: number) => ["playback", id] as const,
  proxyCache: ["proxy-cache"] as const,
  mapBounds: (scope: LayoutScope | null) => ["map-bounds", scopeKey(scope)] as const,
  mapClusters: (bounds: unknown, zoom: number, scope: LayoutScope | null) =>
    ["map-clusters", JSON.stringify(bounds ?? null), zoom, scopeKey(scope)] as const,
  assetsInBounds: (bounds: unknown) => ["assets-in-bounds", JSON.stringify(bounds ?? null)] as const,
  places: (q: string, online: boolean) => ["places", q, online] as const,
};
