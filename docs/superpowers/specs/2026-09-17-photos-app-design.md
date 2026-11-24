# Photos App — General Design

**Date:** 2026-09-17
**Status:** Approved for planning
**Stack:** Tauri v2 · Rust (Cargo workspace) · React 19 + TypeScript · Vite · Mantine · SCSS · Framer Motion
**Purpose:** A local-first photo and video library for macOS, Linux and Windows — map, device import, per-device sections, albums, non-destructive edits, memories and a slideshow maker.

This document is the architectural spine. It fixes the decisions that are expensive to change later and leaves feature-level detail to the per-phase implementation plans.

---

## 1. Goals

- **Fast at 50k–250k items.** 60fps scrolling; first grid paint under a second on a cold start.
- **Rust does the work.** Indexing, decoding, resizing, hashing, geo-clustering, grid layout, search and video export all run in native code. The frontend renders and animates; it does not compute.
- **Every format opens.** JPEG, PNG, WebP, GIF, TIFF, BMP, HEIC/HEIF, AVIF, JPEG XL, every common RAW, and video from phones, DSLRs and camcorders.
- **Local-first and private.** No account, no cloud, no telemetry. The library is a folder you can back up with any tool. (Since 2026-09-22 the app fetches its ML models automatically — a plain GET of public files that sends nothing about the user; see `2026-09-22-lumen-ml-design.md` §2.3. Music search stays opt-in. Amendment 2026-11-08: *Exact place names*, an opt-in switch, sends photo coordinates (never photos) to OpenStreetMap Nominatim; plan `2026-11-08-exact-place-names.md`.)
- **One codebase, three platforms.** No per-OS feature gaps in the UI.

## 2. Non-goals

Out of scope, revisited only after the roadmap in §17 completes:

- Cloud sync, sharing links, multi-user libraries
- Masks and local adjustments, lens profiles. (Full RAW development with curves, colour and ML Auto/denoise came into scope on 2026-09-22 — `2026-09-22-lumen-develop-design.md`; §12 remains the simple editor.)
- Mobile or web clients
- Importing Lightroom catalogs (Apple Photos libraries *are* in scope since 2026-09-19: §8.4)

## 3. Fixed decisions

| # | Decision | Choice |
|---|---|---|
| 1 | File ownership | **Managed library.** Imports are copied into an app-owned tree; the app is the source of truth. |
| 2 | Target scale | **50k–250k items.** Keyset pagination, covering indexes, no full-table scans on the hot path. |
| 3 | iPhone import | **Native USB on all three platforms** via the libimobiledevice protocol stack. |
| 4 | Codecs | **Bundled.** libheif and the RAW decoder linked in-process; FFmpeg shipped as a sidecar binary. |
| 5 | Map | **MapLibre GL JS + OpenFreeMap vector tiles** (no key), Esri imagery for satellite, and a bundled Natural Earth floor when offline or when online detail is switched off. |
| 6 | Video playback | **Native first, FFmpeg transcode fallback** streamed over the custom protocol. |
| 7 | Slideshow | **In-app editor with live preview, plus MP4 export.** |
| 8 | Memories | **Rule-based clustering in v1**; on-device ML (faces, scenes) as a later phase. |
| 9 | Device sections | **Derived from EXIF camera identity** (make / model / body serial), with an import-batch id stamped on every asset. |
| 10 | Editing | **Basic non-destructive adjustments.** Originals are kept intact (exceptions: confirmed metadata additions and lossless edits, §12). |
| 11 | Deletion | **In-app trash with 30-day recovery.** |
| 12 | First milestone | **Catalog + grid + viewer.** |
| 13 | Apple sources | **Read-only and in-process.** Mac-formatted disks (HFS+, APFS) are read with pure-Rust libraries (`hfsplus`, `apfs`) from the partition device, which Windows lets an unelevated process read; no driver, no helper, no admin. Apple Photos libraries are read from a temporary copy of `Photos.sqlite`. Encrypted and transparently compressed files are out of scope. |
| 14 | AI assistants | **Opt-in local MCP server** (`127.0.0.1`, bearer token, off by default), scoped to Develop's filmstrip, writing drafts only. Preview JPEGs travel base64 inside MCP results as the protocol requires — a third channel for external clients only, not app transport. |
| 15 | Plugins | **WebAssembly components (wasmtime) in Lumen's process, with host-function capabilities only** (no files, no network, no library); data kinds are files Lumen checks; app-wide under `config_dir/JL-Studios/Lumen/plugins/`; spec `2026-11-13-round-23-design.md`. |

---

## 4. System architecture

### 4.1 Process model

Three processes, by design:

1. **Tauri host (Rust)** — owns the catalog, the derivative cache, the job queue and all file I/O. Single writer to SQLite.
2. **Webview (React)** — renders. Holds UI state and a query cache. Never reads a file path, never decodes an image itself.
3. **FFmpeg sidecar** — spawned per job for video probing, thumbnail extraction, transcoding and slideshow export.

FFmpeg is a **subprocess, not linked bindings**. Three reasons, each sufficient on its own: a malformed file from a 2009 camcorder kills a subprocess instead of the app; the GPL/LGPL boundary stays clean because the binary is distributed alongside ours rather than linked into it; and build times stay tolerable. libheif and the RAW decoder link in-process, where they are well-behaved and subprocess overhead would dominate a 20ms call.

### 4.2 Transport — the performance-critical decision

Two channels, strictly separated.

**Metadata → `invoke` (JSON).** Queries return pages of ~200 rows — a few KB. Fast, ergonomic, type-safe.

**Bytes → custom protocol.** A registered `photos://` scheme serves every image and video:

| URL | Serves |
|---|---|
| `photos://thumb/{id}?s=256` | grid tile |
| `photos://thumb/{id}?s=1024` | large grid tile / quick look |
| `photos://preview/{id}?s=2560` | viewer preview |
| `photos://original/{id}` | full-resolution original |
| `photos://video/{id}` | native playback, `Range` supported |
| `photos://video/{id}?proxy=1` | transcoded stream for codecs the webview can't decode |

A grid tile is therefore literally `<img src="photos://thumb/abc123?s=256">`. This matters more than any other choice in the document:

- The webview fetches on its own network thread — zero main-thread blocking.
- Decoding happens off-thread in the browser's optimized image path.
- The webview's own image cache handles eviction, which we would otherwise reimplement badly.
- No base64 (33% overhead), no IPC serialization of megabytes, no leaked `createObjectURL` handles.

**Hard requirement:** the protocol handler must be registered asynchronously and must honour the `Range` request header, returning `206 Partial Content` with a correct `Content-Range`. Video seeking is broken without it. Verify this first thing in Phase 0 — if it proves unworkable, the fallback is an in-process `axum` server bound to `127.0.0.1:0` with a per-session bearer token, and the frontend's URL builder is the only code that changes.

**Events → `Emitter`.** Import progress, job completion and library mutations are pushed as Tauri events. The frontend never polls.

### 4.3 Crate layout

A Cargo workspace under `src-tauri/`. Boundaries are enforced by the compiler, and incremental builds stay fast once ONNX Runtime enters the tree.

```
src-tauri/
  Cargo.toml                  # workspace root
  crates/
    photos-core/              # domain types, SQLite, queries, migrations
    photos-media/             # decode, metadata, resize, thumbnails, ffmpeg sidecar
    photos-raw/               # rawler (LGPL-2.1) behind a C interface, built alone as a shared library
    photos-import/            # volume detection, DCIM scan, iPhone AFC
    photos-jobs/              # priority queue, worker pool, cancellation
    photos-ml/                # phase 7: ONNX faces + scene embeddings
    photos-models/            # model list, file check, resuming download (no workspace deps)
    photos-plugins/           # Round 23: plugin manifest, package, signature, store, themes, wasmtime host
  src/                        # the Tauri binary: commands, protocol, events
  binaries/                   # ffmpeg / ffprobe sidecars, per-triple
```

Dependency rule — one direction only:

```
src (tauri bin)
  ├──> photos-import ──> photos-media ──> photos-core
  ├──> photos-jobs   ──> photos-core
  ├──> photos-ml     ──> photos-core
  │         └──────────> photos-models
  ├──> photos-plugins ──> photos-core
  └──> photos-models
```

`photos-raw` is on no arrow: no crate depends on it, it depends on no workspace crate, and `photos-media::raw_lib` loads it at run time (`photos_raw.dll` / `libphotos_raw.so` / `libphotos_raw.dylib`) so people can replace it. It is the only crate that may use `rawler`.

`photos-models` depends on no workspace crate; the app, `photos-ml` and Lumen Setup (`installer/`) use it. Setup is the second binary that may depend on `tauri`.

`photos-core` depends on no other workspace crate. **No crate depends on `tauri` except the binary**, so every crate is testable with plain `cargo test` and no webview.

### 4.4 Command layer

`src/commands/` holds thin adapters only — deserialize, call a crate, serialize. No business logic. One module per domain: `library`, `albums`, `import`, `map`, `edits`, `memories`, `slideshow`, `settings`.

Every command returns `Result<T, AppError>`, where `AppError` serializes to `{ code, message, detail? }`. `code` is a stable machine-readable string the frontend switches on; `message` is user-facing.

TypeScript types are **generated, never hand-written**: `ts-rs` derives declarations from the Rust structs into `src/types/generated/`, committed to the repo, regenerated by `cargo test`, and verified clean in CI. The IPC boundary cannot silently drift.

---

## 5. Data model

### 5.1 Disk layout

```
<LibraryRoot>/
  library.json                                  identity: { id (UUID), name, createdAt } — how a library is recognised on any machine or mount path
  library.db                                    SQLite (WAL)
  originals/2026/09/17/3f8a2b1c_IMG_4821.HEIC   hash prefix + original filename
  derivatives/
    thumbs/3f/3f8a2b1c_256.jpg
    thumbs/3f/3f8a2b1c_1024.jpg
    previews/3f/3f8a2b1c_2560.jpg
    video/3f/3f8a2b1c_proxy.mp4
  trash/                                        originals awaiting purge
  exports/                                      rendered slideshows
  logs/
```

Every file is keyed by its **BLAKE3 content hash**. This costs two lines of code and buys:

- Re-importing the same SD card is a no-op — dedup is a primary-key lookup.
- Derivatives key off content, so the same photo imported from two devices shares one thumbnail.
- Integrity checking is a rehash.

Fanning out by the first two hex characters keeps directories under ~1000 entries, which matters on Windows.

### 5.2 Schema

`rusqlite` with the `bundled` feature — no system dependency on any platform — plus FTS5 and R\*Tree. WAL mode, `synchronous=NORMAL`, one writer connection, an `r2d2` pool of readers.

```sql
CREATE TABLE assets (
  id                INTEGER PRIMARY KEY,
  content_hash      TEXT NOT NULL UNIQUE,       -- BLAKE3
  relative_path     TEXT NOT NULL,              -- within originals/
  filename          TEXT NOT NULL,              -- as it arrived
  extension         TEXT NOT NULL,
  media_type        INTEGER NOT NULL,           -- 0 image, 1 video, 2 motion photo
  byte_size         INTEGER NOT NULL,
  width             INTEGER,
  height            INTEGER,
  orientation       INTEGER NOT NULL DEFAULT 1,
  duration_ms       INTEGER,                    -- video only
  captured_at       INTEGER,                    -- unix seconds, local wall time
  captured_at_utc   INTEGER,                    -- null when tz unknown
  tz_offset_minutes INTEGER,
  imported_at       INTEGER NOT NULL,
  file_modified_at  INTEGER,
  device_id         INTEGER REFERENCES devices(id),
  import_batch_id   INTEGER REFERENCES import_batches(id),
  gps_lat           REAL,
  gps_lon           REAL,
  gps_altitude      REAL,
  thumbhash         BLOB,                       -- ~25 bytes, instant placeholder
  dominant_color    TEXT,                       -- oklch(), for tile backgrounds
  rating            INTEGER NOT NULL DEFAULT 0,
  is_favorite       INTEGER NOT NULL DEFAULT 0,
  state             INTEGER NOT NULL DEFAULT 0, -- 0 active, 1 trashed, 2 pending purge
  trashed_at        INTEGER,
  live_photo_group  TEXT,                       -- pairs HEIC + MOV
  metadata_json     TEXT                        -- full EXIF/XMP, rarely queried
);

CREATE INDEX idx_assets_timeline ON assets(state, captured_at DESC, id DESC);
CREATE INDEX idx_assets_device   ON assets(device_id, state, captured_at DESC, id DESC);
CREATE INDEX idx_assets_batch    ON assets(import_batch_id);
CREATE INDEX idx_assets_favorite ON assets(is_favorite, captured_at DESC) WHERE is_favorite = 1;
CREATE INDEX idx_assets_trash    ON assets(trashed_at) WHERE state = 1;

CREATE VIRTUAL TABLE assets_geo USING rtree(id, min_lat, max_lat, min_lon, max_lon);

CREATE TABLE devices (
  id            INTEGER PRIMARY KEY,
  make          TEXT,
  model         TEXT,
  body_serial   TEXT,
  display_name  TEXT NOT NULL,                  -- user-editable
  kind          INTEGER NOT NULL,               -- phone, camera, drone, scanner, unknown
  icon          TEXT,
  first_seen_at INTEGER NOT NULL,
  UNIQUE(make, model, body_serial)
);

CREATE TABLE import_batches (
  id            INTEGER PRIMARY KEY,
  source_kind   INTEGER NOT NULL,               -- folder, volume, iphone
  source_label  TEXT NOT NULL,
  started_at    INTEGER NOT NULL,
  finished_at   INTEGER,
  item_count    INTEGER NOT NULL DEFAULT 0,
  skipped_count INTEGER NOT NULL DEFAULT 0,
  state         INTEGER NOT NULL                -- running, complete, failed, cancelled
);

CREATE TABLE albums (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  kind           INTEGER NOT NULL,              -- 0 manual, 1 smart
  query_json     TEXT,                          -- smart albums only
  cover_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
  parent_id      INTEGER REFERENCES albums(id) ON DELETE CASCADE,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL
);

CREATE TABLE album_items (
  album_id  INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  position  REAL NOT NULL,                      -- fractional: reorder is one UPDATE
  added_at  INTEGER NOT NULL,
  PRIMARY KEY (album_id, asset_id)
);
CREATE INDEX idx_album_items_order ON album_items(album_id, position);

CREATE TABLE edits (
  asset_id    INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  recipe_json TEXT NOT NULL,
  version     INTEGER NOT NULL DEFAULT 1,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE derivatives (
  content_hash  TEXT NOT NULL,
  kind          INTEGER NOT NULL,               -- thumb, preview, proxy
  size          INTEGER NOT NULL,
  edit_version  INTEGER NOT NULL DEFAULT 0,     -- bumps when the recipe changes
  relative_path TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  generated_at  INTEGER NOT NULL,
  PRIMARY KEY (content_hash, kind, size, edit_version)
);

CREATE TABLE memories (
  id             INTEGER PRIMARY KEY,
  kind           INTEGER NOT NULL,              -- trip, event, on_this_day, person
  title          TEXT NOT NULL,
  subtitle       TEXT,
  start_at       INTEGER NOT NULL,
  end_at         INTEGER NOT NULL,
  centroid_lat   REAL,
  centroid_lon   REAL,
  cover_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
  generated_at   INTEGER NOT NULL,
  is_hidden      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE memory_items (
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  score     REAL NOT NULL,                      -- curation rank
  PRIMARY KEY (memory_id, asset_id)
);

CREATE TABLE slideshows (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  document_json TEXT NOT NULL,                  -- see §14
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE VIRTUAL TABLE assets_fts USING fts5(
  filename, device_name, place_name, keywords,
  content='', tokenize='unicode61 remove_diacritics 2'
);
```

Migrations are numbered SQL files embedded with `include_str!`, applied inside a transaction on startup and tracked in `schema_version`. Forward-only; no down migrations.

### 5.3 Pagination

Never `OFFSET`. Every timeline query uses keyset pagination on the covering index:

```sql
SELECT ... FROM assets
WHERE state = 0 AND (captured_at, id) < (?1, ?2)
ORDER BY captured_at DESC, id DESC
LIMIT 200;
```

Constant time regardless of scroll depth. Every list command returns `{ items, next_cursor }`.

---

## 6. Media pipeline

### 6.1 Ingest

Each file passes through a fixed sequence. Every stage is a pure function over the previous stage's output, so each is independently testable.

1. **Discover** — walk the source, filter by extension allowlist, collect `(path, size, mtime)`.
2. **Hash** — streaming BLAKE3, 1MB chunks. Parallel across files via `rayon`.
3. **Dedup** — `SELECT id FROM assets WHERE content_hash = ?`. Hit → record as skipped, stop.
4. **Copy** — write into `originals/YYYY/MM/DD/`, `fsync`, verify size. Copy, never move: the source device is not ours to mutate.
5. **Extract metadata** — EXIF / XMP / QuickTime atoms → capture time, timezone, orientation, dimensions, GPS, camera identity.
6. **Resolve device** — upsert `devices` on `(make, model, body_serial)`; that row *is* the library section.
7. **Insert** — one transaction, batched 200 rows at a time.
8. **Enqueue derivatives** — thumbhash + 256px eagerly; 1024px and 2560px lazily on demand.
9. **Emit progress** — throttled to ~10Hz so the UI doesn't drown in events.

Crash safety: an interrupted import leaves the batch in state `running`. On next launch, any `running` batch is reconciled — orphaned copies without an `assets` row are deleted, and the batch is marked `failed` with a resumable cursor.

### 6.2 Decode strategy

| Family | Path |
|---|---|
| JPEG / PNG / WebP / GIF / TIFF / BMP | `image` crate, pure Rust |
| HEIC / HEIF / AVIF | **FFmpeg sidecar** (`ffmpeg -i in.heic -frames:v 1 out.jpg`), then the `image` crate |
| JPEG XL | `jxl-oxide`, pure Rust |
| RAW (CR2/CR3/NEF/ARW/DNG/RAF/ORF/RW2…) | `rawler` — **embedded preview first** |
| Video | FFmpeg sidecar (`ffprobe` for metadata, `ffmpeg` for frames) |

**The RAW fast path is the single biggest thumbnailing win.** Essentially every RAW file embeds a full-size JPEG preview that the camera already rendered. Extracting it is a seek and a copy — on the order of 5ms, versus 300–800ms for a full demosaic. So:

- Thumbnails and viewer previews use the embedded JPEG.
- Full demosaic runs only when you open a RAW at 1:1 or apply an edit, and the result is cached.

Video thumbnails are taken at 10% of duration, capped at 5 s in (not frame 0 — that is frequently black), via a single sidecar invocation writing a JPEG. A seek past the end falls back to the first frame.

**HEIC decodes through the sidecar, not `libheif-rs` (decided in round 2).** One dependency covers both HEIC and video, it needs no C toolchain on any of the three platforms, and the cost — a subprocess and a temporary JPEG per decode — is paid once per file because the 2560 preview is cut down to every smaller size afterwards. The consequence is that **HEIC and video are renderable only when FFmpeg is present**: `is_renderable_with(kind, ffmpeg_available)` decides, `Tile.renderable` carries it to the grid, and files imported before FFmpeg appeared are re-queued for derivatives by a maintenance job as soon as it does.

### 6.2a High dynamic range

Two unrelated things are called an HDR photo, and they need opposite handling.

| Kind | Where | Pixels | What we do |
|---|---|---|---|
| **Gain map** | Apple HEIC, Google Ultra HDR JPEG | SDR, plus a map of how much brighter each pixel may go | Derivatives are already correct; the viewer serves the original so the webview can apply the map |
| **PQ / HLG** | AVIF, HEIF, video frames | In an HDR transfer function | Tone mapped through linear light when rendering any SDR derivative, or they come out flat |

`photos-media::hdr` decides which, from a bounded scan for gain-map markers (Apple's
`urn:com:apple:photo:2020:aux:hdrgainmap`, ISO 21496-1, Google's container namespace) plus
ffprobe's `color_transfer` and PNG's `cICP` chunk. JPEG and PNG are therefore classified
with no FFmpeg present. `assets.hdr` stores the answer (migration 0006).

The viewer skips its 2560 preview for an HDR photo, because that preview is a re-encoded
SDR JPEG: `full/{id}` serves the original for every format the webview can decode. Chromium
cannot decode HEIC at all, so an HDR HEIC still shows its rendered SDR derivative — the
information is in the file, but nothing in the stack can put it on screen without
re-encoding to AVIF, which cannot carry Apple's gain map.

### 6.3 Resize and encode

`fast_image_resize` with Lanczos3 — SIMD-accelerated, several times faster than a naive resample. Thumbnails encode as **JPEG at q82**, not AVIF: these live on local disk where file size is irrelevant, and JPEG decodes several times faster in the webview. That tradeoff runs the other way for a web app; here, decode speed is the whole game.

Three sizes: `256` (grid), `1024` (large grid, quick look), `2560` (viewer preview). Long-edge fitted, EXIF orientation baked in so the frontend never rotates anything.

**ThumbHash** is computed once at ingest and stored in the `assets` row. It is ~25 bytes, arrives with the metadata page, and decodes client-side into a blurred placeholder — so tiles paint *instantly* on fast scroll and fill in as real thumbnails arrive. This is the difference between a grid that feels solid and one that flickers grey.

### 6.4 Video playback

1. `ffprobe` at ingest records codec, profile, pixel format and bit depth.
2. A capability table decides whether the webview can play it natively (H.264 baseline/main/high 8-bit, AAC — plus HEVC on macOS and Windows).
3. Native → `photos://video/{id}` streams the original with range support.
4. Not native → a proxy is generated on first play (H.264 High + AAC 160k, capped at 1080p, `+faststart`) and cached at `derivatives/video/{h2}/{hash}_proxy.mp4`. Subsequent plays are instant.

Proxy generation is a normal job with progress, so a 4K ProRes clip shows a spinner rather than appearing broken. `photos://video/{id}?proxy=1` answers `202 Accepted` with `Retry-After: 2` while the transcode runs; progress arrives as `proxy://progress` and completion as `proxy://finished`. The command `get_playback(id)` returns `native` / `proxy` / `unavailable` from the stored `ffprobe` block, probing (and storing the result) on first ask for videos imported before FFmpeg existed. Settings → Playback & codecs shows the proxy cache size with *Clear*.

**As built, the capability table is:** container must be MP4/MOV family, audio must be AAC, MP3 or absent, pixel format must be 8-bit; then H.264 is native everywhere and HEVC is native on macOS and Windows, proxy on Linux. Everything else is a proxy — or `unavailable` when FFmpeg is missing and the container is not MP4/MOV.

---

## 7. Job scheduler

`photos-jobs` is small and carries most of the app's perceived speed.

- **Worker pool** of `num_cpus - 1` threads, so the UI thread always has a core.
- **Four priority lanes,** drained strictly in order:
  1. `Interactive` — thumbnails for tiles currently on screen
  2. `Prefetch` — the next screenful in the scroll direction
  3. `Import` — the ingest backlog
  4. `Maintenance` — integrity checks, trash purge, memory regeneration
- **Cancellation.** The frontend reports its visible range on scroll; jobs for rows scrolled past are cancelled before they start. Without this, fast scrolling through 250k items queues tens of thousands of doomed jobs and the app stops responding.
- **Coalescing.** Duplicate requests for the same `(hash, kind, size)` join one in-flight job.
- **Backpressure.** A bounded queue; enqueuing blocks the producer rather than growing without limit.

Every job reports `{ id, kind, progress, state }`, surfaced in a single activity indicator in the UI shell.

---

## 8. Import subsystem

### 8.1 Volume detection

A 2-second poll of mounted volumes via `sysinfo`, diffed against the previous snapshot. Deliberately *not* three platform-native APIs (`WM_DEVICECHANGE` / DiskArbitration / udev): the poll costs nothing measurable, behaves identically everywhere, and removes three chunks of unsafe platform code. New volume containing `DCIM/` → an import prompt appears in the shell.

Incremental import remembers `(volume_uuid, relative_path, size, mtime)` per device, so re-inserting a card offers only what's new.

The same poll serves **portable libraries**: when a volume appears, every registered library whose path is currently unavailable is looked for on it (same path below the old root, then `<mount>/Lumen Library`), matched by the `library.json` id, and offered to the user. Everything inside a library is relative to its root, so a library on an exFAT drive opens unchanged on macOS, Linux and Windows.

### 8.2 iPhone over USB

The riskiest component in the project, and the one to spike first.

- The `idevice` crate (pure-Rust libimobiledevice) speaks usbmuxd → lockdownd → AFC.
- Flow: detect → pair (device shows the Trust prompt) → mount AFC → enumerate `/DCIM/**` → stream files over AFC into the ingest pipeline.
- **Live Photos:** `IMG_1234.HEIC` + `IMG_1234.MOV` are imported as one asset with `media_type = 2` and a shared `live_photo_group`.
- **Platform caveat:** usbmuxd is a system service. Present by default on macOS; `usbmuxd` package on Linux; on Windows it ships with Apple Mobile Device Support. If it's absent, the app must detect that and say so plainly with a link, rather than failing with a socket error.

**Spike outcome (Windows 11, round 2, 2026-09-18):** with the Apple Devices app installed, usbmuxd answers on `127.0.0.1:27015` and `UsbmuxdConnection::default()` connects, so the transport works on Windows and the MTP fallback is not needed. No phone was attached during the spike, so device enumeration and the AFC pull are covered by unit tests only; `cargo test -p photos-import --test iphone_spike -- --ignored --nocapture` re-runs the probe with hardware and prints what it finds. macOS and Linux are still unverified.

**As built:** `IphoneSource` holds one AFC session behind a mutex and a current-thread tokio runtime, because AFC handles cannot cross the rayon staging threads; each file is streamed in 1 MB chunks into a temporary file that the ingest pipeline then reads. Errors collapse to four states the UI can act on — `usbmuxd_missing`, `iphone_not_paired`, `iphone_trust_required`, `iphone`. Polling runs only while a device-facing screen is open (`set_iphone_polling`), and publishes `device://iphones`.

Live Photo pairing happens once per batch, after the last file: a still and a `.mov` sharing a filename stem get the same `live_photo_group`, the still becomes `media_type = 2`, and `LayoutScope` hides the MOV half from every grid.

### 8.3 The `ImportSource` trait

All three sources — folder, volume, iPhone — implement one trait:

```rust
trait ImportSource {
    fn label(&self) -> String;
    fn enumerate(&self) -> Result<Vec<SourceItem>>;
    fn open(&self, item: &SourceItem) -> Result<Box<dyn Read + Send>>;
}
```

The ingest pipeline is written once against `Read`. Adding Android/MTP later is one new implementation and zero changes to ingest.

A source may also return per-item **extras** (`fn extras(&self, item) -> Option<ItemExtras>`, default `None`): favourite, album names, title, caption, keywords, and a capture time or location that overrides what the file says. Ingest applies them to the inserted asset, or to the existing one when the file is a duplicate, so importing an Apple library over photos already imported from a folder still brings their favourites and albums.

### 8.4 Apple disks and Photos libraries

**Measured 2026-09-19 (Windows 11, unelevated, USB HFS+ disk with 1.3 M files):** `\\.\PhysicalDriveN` is refused (error 5), but the partition device `\\?\GLOBALROOT\Device\HarddiskN\PartitionM` opens for reading, seeks to any offset, and `hfsplus::HfsVolume` opens it in 365 ms. Raw devices only accept sector-aligned reads, so a buffered `AlignedReader` sits between the device and the parsers. Partition types are recognised by content, never by partition table (which needs the whole-disk device): `H+`/`HX` at byte 1024 is HFS+, `NXSB` at byte 32 is an APFS container.

- **Where:** Windows only probes partition devices (every 5 s, only partitions not already known); on macOS these disks mount natively, and on Linux HFS+ mounts natively, so both are ordinary volumes there. APFS on Linux is not supported (the block device needs root).
- **What the device page offers for a Mac disk:** its Photos libraries (found in the top three folder levels), and a folder browser to import any folder with or without subfolders.
- **Libraries:** the same `PhotosLibrarySource` reads a library from a Mac disk or from any mounted or copied folder (on macOS, `~/Pictures/Photos Library.photoslibrary` needs Full Disk Access). It imports each visible original (`originals/<X>/<UUID>.<ext>`), its Live Photo video (`<UUID>_3.mov`) and RAW alternate (`<UUID>_4.<raw>`) under the photo's original filename, so Live Photo pairing by stem works unchanged. Skipped and counted: trashed items, unpicked burst frames, originals not on disk (iCloud *Optimize Storage*), and *referenced* files kept outside the library. Apple's edits are not imported; the original is. Entity numbers in the Core Data schema differ between macOS versions and are read from `Z_PRIMARYKEY`.

---

## 9. Frontend architecture

### 9.1 Folder structure

```
src/
  main.tsx
  app.tsx                        providers: Mantine, QueryClient, Router, MotionConfig
  styles/
    globals.scss                 all root-level styles: tokens, resets, scrollbars, focus rings
    mixins.scss                  every mixin, grouped by concern
    _typography.scss
    _animations.scss             shared keyframes + easing curves
  components/
    <component-name>/
      gui.tsx
      styles.scss
      index.ts                   re-export, enabling `@/components/photo-grid`
  routes/
    router.tsx                   the React Router tree
    <route-name>/
      gui.tsx
      styles.scss
      index.ts                   enabling `@/routes/library`
  utils/
    color.ts                     oklch helpers, hex↔rgb, contrast
    date.ts                      formatting, grouping, relative time
    format.ts                    bytes, durations, counts
    geo.ts                       bounds, distance, tile math
    layout.ts                    justified-row helpers
    media.ts                     protocol URL builders
  hooks/
    use-library-query.ts
    use-visible-range.ts
    use-keyboard-nav.ts
    use-color-scheme.ts
  lib/
    ipc.ts                       typed invoke wrapper, error mapping
    query-keys.ts                single source of truth for cache keys
    events.ts                    Tauri event subscriptions → query invalidation
  types/
    generated/                   ts-rs output — do not edit
```

The `@` alias maps to `src/` in both `vite.config.ts` and `tsconfig.app.json`, so imports read `@/routes/library`, `@/components/photo-grid`, `@/utils/date`. Each folder's `index.ts` re-exports from `gui.tsx`.

**Discipline:** a component folder owns its `styles.scss` and imports nothing from another component's stylesheet. Anything shared moves to `mixins.scss` or becomes a token in `globals.scss`.

### 9.2 Routes

| Path | Purpose |
|---|---|
| `/` | Home — memories, recent imports, highlights |
| `/library` | All photos, timeline-grouped |
| `/library/:deviceId` | A single device section |
| `/albums`, `/albums/:id` | Albums |
| `/map` | World map |
| `/memories`, `/memories/:id` | Generated memories |
| `/slideshows`, `/slideshows/:id` | Slideshow editor |
| `/import` | Import sources and active batches |
| `/trash` | Recently deleted |
| `/settings` | Library, codecs, appearance |

The viewer is **not** a route of its own. It is an overlay driven by a `?asset=<id>` search param, so it opens over any grid, closes back to exact scroll position, and is linkable.

### 9.3 Theming

`globals.scss` defines the entire visual system as CSS custom properties in **oklch**, and nothing else in the app hardcodes a colour:

```scss
:root {
  --hue-brand: 265;
  --hue-accent: 190;
  --chroma-ui: 0.03;

  --bg-base:    oklch(98% 0.004 var(--hue-brand));
  --bg-raised:  oklch(100% 0 0);
  --bg-sunken:  oklch(95% 0.006 var(--hue-brand));
  --fg-primary: oklch(20% 0.02 var(--hue-brand));
  --fg-muted:   oklch(52% 0.015 var(--hue-brand));
  --brand:      oklch(62% 0.19 var(--hue-brand));
  --border:     oklch(88% 0.008 var(--hue-brand));
  /* radii, spacing scale, shadows, z-index layers, motion durations */
}

:root[data-mantine-color-scheme="dark"] {
  --bg-base:    oklch(16% 0.012 var(--hue-brand));
  --bg-raised:  oklch(21% 0.014 var(--hue-brand));
  /* …only the tokens change; no component stylesheet knows dark mode exists */
}
```

Because lightness is perceptually uniform in oklch, the dark palette is largely the light one with lightness inverted — and changing `--hue-brand` alone re-themes the whole app. That is the "easily change the scheme" requirement, satisfied structurally rather than by find-and-replace.

Mantine is configured with `defaultColorScheme="auto"` so it follows the OS, and its theme object reads the same custom properties, so Mantine components and hand-written SCSS can never disagree. `postcss-preset-mantine` provides the `light-dark()` helper and breakpoint mixins.

### 9.4 Responsiveness

A single set of breakpoints lives in `mixins.scss` and is shared with the Mantine theme so JS and CSS agree:

```scss
$bp: (xs: 36em, sm: 48em, md: 62em, lg: 75em, xl: 88em);
@mixin above($name) { @media (min-width: map.get($bp, $name)) { @content; } }
```

- Navigation is a persistent sidebar above `md`, a collapsible drawer below, and a bottom bar below `sm`.
- The grid's target tile size scales with viewport width; layout is recomputed on a debounced `ResizeObserver`.
- The viewer chrome auto-hides in narrow windows.

### 9.5 Animation

Framer Motion, applied where it reads as quality and nowhere else.

- **Route transitions** — `AnimatePresence` keyed on pathname; a 180ms fade plus 8px rise. Shared layout animation for the sidebar indicator.
- **Viewer open/close** — `layoutId` shared-element transition from the grid tile to the full-screen image. This is the single most valuable animation in the app.
- **Staggered entrance** — shell chrome and album cards only.
- **Micro-interactions** — hover lift on cards, spring-based selection checkmarks, drawer slides.

**Explicitly forbidden:** per-tile animation in the photo grid. Mounting motion components on thousands of virtualized tiles destroys scroll performance. Grid tiles animate with plain CSS `opacity` on decode, nothing more.

`MotionConfig` respects `prefers-reduced-motion` globally; every duration is a token from `globals.scss` so the whole app's feel is tunable from one file.

### 9.6 The photo grid

The hardest piece of frontend engineering in the app.

- **Layout is computed in Rust.** Given container width, target row height and a page of aspect ratios, `photos-core` returns row descriptors (`[{ height, items: [{ id, width }] }]`) for a justified, Google-Photos-style layout. Results are cached by `(query_hash, container_width)` and invalidated on resize. The frontend positions absolutely and never measures.
- **Virtualization** by row via `@tanstack/react-virtual`, with an overscan of two screenfuls.
- **Windowed data.** TanStack Query holds ~5 pages; pages outside the window are dropped. At 250k items the frontend never holds more than a few thousand rows in memory.
- **Visible-range reporting.** The scroll handler (throttled to one animation frame) reports the visible id range to Rust, which re-prioritizes the thumbnail queue and cancels work for rows now off-screen.
- **Paint order per tile:** ThumbHash placeholder → `<img>` with `decoding="async"` and `loading="lazy"` → fade in on load.
- **Sticky date headers** derived from the same Rust layout pass.

### 9.7 The viewer

- Opens via shared-element transition from the tile.
- Loads the 1024px thumbnail first (usually already cached), then swaps to the 2560px preview, then to the original only when you zoom past 100%.
- Zoom and pan on a GPU-composited transform, pinch and trackpad gestures, double-click to fit/fill.
- Adjacent assets prefetched in both directions so arrow-key paging is instant.
- Keyboard: arrows, `space`, `f` favorite, `i` info, `e` edit, `delete` trash, `esc` close.
- Info panel: EXIF, camera, lens, a mini-map when GPS is present, histogram.

---

## 10. Map

MapLibre GL JS with Protomaps vector tiles, styled from the same oklch tokens so the map matches light and dark mode rather than sitting in the UI as a bright rectangle.

**Clustering happens in Rust.** Shipping 250k points to `supercluster` in the browser is exactly the mistake this architecture exists to avoid. Instead:

```rust
fn map_clusters(bounds: Bounds, zoom: u8, filter: Query) -> Vec<Cluster>
```

The R\*Tree index selects the viewport, points are snapped to a zoom-dependent grid, and each cell returns a centroid, a count and a cover asset id. A few hundred features cross the boundary per pan, whatever the library size. Clusters render as themed markers with the cover thumbnail; clicking one zooms in, and clicking a leaf opens the viewer.

Reverse geocoding for place names uses a bundled offline dataset of cities and countries (a few MB), looked up via the same R\*Tree. No network, no API key, no per-request privacy leak. Place names feed the FTS index so "Lisbon" is searchable.

**Amendment 2026-11-08 — exact place names.** The atlas has no borders (Idrija, Slovenia was named after Gorizia, Italy). With Settings → Connections → *Exact place names* on (off by default), a background worker sends each distinct position (rounded to 5 decimals, ≈1 m, so no point moves across a border) to OpenStreetMap Nominatim at most once per second and stores the town and village it names (`assets.place_source = 1`, cache `place_lookups`, migration 0035). The atlas still names everything at once and whenever Lumen is offline; those rows are named again online later. Plan: `plans/2026-11-08-exact-place-names.md`.

---


**As built (Phase 4).** The roadmap called for MapLibre GL with Protomaps vector tiles and
an API key in settings, which without a key draws blank land under the markers. Instead the
base map is a Natural Earth 110 m coastline, simplified and bundled at 25 KB, drawn to a
canvas in Web Mercator. No key, no tile server, no request per pan, and the world is still
recognisable — which is the same trade the rest of the app makes. Markers stay DOM elements
so each can carry a real thumbnail and behave like a button.

Clustering is `photos-core::geo`. The viewport query walks a covering index (migration 0007)
and the grouping happens in Rust rather than SQL: `GROUP BY` on computed cell keys made
SQLite build a temporary B-tree that cost more than the scan itself. Cell keys shift
coordinates into positive space so a truncating cast *is* a floor, which a sign-corrected
expression needed three divisions per row to achieve.

Measured at 250k synthetic items (criterion, dev machine):

| Query | Time |
|---|---|
| Clusters, whole world at zoom 3 | 14.4 ms |
| Clusters, city viewport at zoom 8 | 0.5 ms |
| `map_bounds` | 9.6 ms |

**As built (round 3).** The canvas outline was not enough: people expect streets, place
names and satellite imagery on a photo map. The Map screen is now MapLibre GL 6, which is
what §3 always specified, with OpenFreeMap as the vector source instead of Protomaps
because it needs no key and has no quota. *Map* uses OpenFreeMap's `liberty` style in light
schemes and `fiord` in dark ones (`dark` was measured too low in contrast to read streets);
*Satellite* is Esri World Imagery with liberty's roads, borders and labels on top. Online
detail is a setting (Settings → Map, on by default) because the tile servers see which areas
are viewed; off, or when the servers cannot be reached, the map draws the bundled Natural
Earth land, lakes and borders in theme colours and makes no request. The viewer's mini-map
stays the bundled canvas drawing.

Markers remain DOM elements, moved on every camera frame from `map.project`, with the hover
scale on an inner element so the hit box never moves. Clustering takes a fractional zoom
(MapLibre's zoom + 1, since it measures in 512px tiles) and merges any two groups whose
markers would be closer than 60px on screen, so markers never overlap.

Reverse geocoding is `photos-core::places`: GeoNames `cities15000` trimmed to name,
country, position and population, gzipped to 565 KB and compiled in. It refuses to name a
city more than 50 km away and says nothing at all past 300 km. A maintenance pass fills
`assets.place_name`, which feeds both the search index and the month headers — a month
where three fifths of the photos share a place is titled "September 2026 · Lisbon,
Portugal".

## 11. Albums and search

**Manual albums** hold an ordered list with fractional `position` values, so a drag-reorder is a single `UPDATE` rather than a renumbering. Nested albums via `parent_id`. An asset can live in many albums; removal from an album never touches the file.

**Smart albums** store a serialized query (`query_json`) evaluated against the same query builder that powers the library view — one code path, so filters behave identically everywhere.

**Search** combines FTS5 (filename, device, place, keywords) with structured filters (date range, device, media type, favorite, has-GPS, camera, lens). All of it compiles to one SQL statement in Rust. Text search over 250k rows returns in single-digit milliseconds.

**As built (Phase 3).** `AssetQuery` in `photos-core/src/query.rs` holds the text and every structured filter and compiles to `(joins, where clauses, parameters)`; `LayoutScope` carries an optional one, so search results, smart albums and filter chips render through the layout index, its cache and the virtualized grid that already existed. Camera and lens read the metadata blob through SQLite's `json_extract`.

`assets_fts` keeps its own copy of the indexed text. Migration 0001 created it contentless (`content=''`), which cannot delete a row without replaying the exact text it indexed — impossible once a device has been renamed, so stale names stayed searchable. Migration 0004 recreates it with content and `photos-core/src/fts.rs` owns every write: rows are indexed inside `insert_assets`' transaction, dropped in `delete_rows`, and re-indexed for a whole device by `rename_device`. `rebuild_search_index` is exposed in Settings.

Measured at 250k synthetic items (criterion, dev machine, release):

| Query | Time |
|---|---|
| Free-text count | 5.6 ms |
| Structured count (rating, kind, location) | 2.8 ms |
| Layout index load for a text query | 5.3 ms |

The structured count was 198 ms before migration 0005, which adds an index carrying `state`, `media_type`, `rating`, `gps_lat` and `live_photo_group` so the scan never leaves the index. `ANALYZE` runs on the maintenance lane, not in a migration: a migration runs against an empty table and teaches the planner nothing.

---

## 12. Non-destructive edits

Originals are kept intact. An edit is a JSON **recipe** stored against the asset. Two exceptions (Round 24, `2026-11-21-round-24-design.md` §11), each only on the person's explicit action and verified before it replaces the file: **metadata writes** that add what the file lacks (GPS on a photo with none; never changing camera-recorded values) and **lossless edits** (video trim by stream copy, lossless JPEG crop). Nothing that re-encodes replaces an original; `assets.content_hash` stays the import identity and `assets.file_hash` records the current bytes.

A recipe:

```json
{
  "version": 1,
  "crop": { "x": 0.1, "y": 0.0, "w": 0.8, "h": 1.0 },
  "rotate": 90,
  "straighten": -1.4,
  "flip_h": false,
  "exposure": 0.3,
  "contrast": 0.1,
  "saturation": -0.05,
  "temperature": 120,
  "tint": -4,
  "highlights": -0.2,
  "shadows": 0.15
}
```

Rendering happens in `photos-media`, in Rust, in linear light. Saving a recipe bumps `edits.version`, which bumps `edit_version` on derivatives — so thumbnails regenerate automatically and the cache never serves a stale edit. "Revert to original" deletes one row.

The editor UI previews at preview resolution with a debounced round-trip to Rust; full-resolution rendering happens only on export. Deliberately **not** in scope: curves, masks, local adjustments, lens corrections, noise reduction.

---

## 13. Memories

**Phase 6 — rule-based.** A maintenance job clusters the library:

- **Trips** — sessions separated by >12h gaps whose GPS centroid is >50km from home, home being the densest cluster of the last year.
- **Events** — dense same-day bursts at one location.
- **On this day** — same calendar day, previous years.
- **Curation** — within a cluster, rank by favorites, faces (when available), sharpness, exposure sanity, and near-duplicate suppression via perceptual hash; take the top N.

Runs incrementally on import, full regeneration on demand. Fast in Rust: the whole library is a few SQL queries plus a clustering pass.

**Phase 7 — ML.** `photos-ml` using `ort` (ONNX Runtime):

- Face detection → embedding → clustering into unnamed people you can name
- A small CLIP-class image encoder for scene tags *and* natural-language search ("photos of the beach at sunset")

Models run entirely on-device. They are **fetched automatically** on first launch after the update, SHA-256 verified, into `config_dir/JL-Studios/Lumen/models/` (decided 2026-09-22; the installer budget rules out bundling). The design is `2026-09-22-lumen-ml-design.md`; nothing before Phase 7 depends on it.

---

## 14. Slideshow maker

The risk here is a preview that looks different from the export. The design removes that risk structurally: the slideshow is a **declarative document**, and both the preview and the renderer are consumers of it.

```json
{
  "version": 1,
  "resolution": [1920, 1080],
  "fps": 30,
  "audio": { "asset": "…", "fade_in_ms": 800, "gain_db": -6 },
  "clips": [
    {
      "asset_id": 1234,
      "duration_ms": 4000,
      "transition": { "kind": "crossfade", "duration_ms": 600 },
      "motion": { "kind": "ken_burns", "from": [0,0,1.0], "to": [0.05,0.02,1.12] }
    }
  ]
}
```

- **Editor** — a timeline with drag-reorder, per-clip duration, transition and motion pickers, plus "auto-fill from memory" seeded by a generated memory.
- **Preview** — a canvas driven by a single clock, reading the same document. It is a preview, not a second implementation: same easing curves, same transition maths, shared as constants generated from Rust.
- **Export** — Rust walks the document and builds one FFmpeg `filter_complex` graph, rendering from full-resolution originals through the edit pipeline. Progress via job events; output to `exports/`.
- **Beat sync** — clip boundaries are snapped to a beat *list* from a dynamic-programming beat tracker (a learned front end when its model is present, spectral flux otherwise); see the ML design §6. The single-tempo detector of Phase 6d refused real music (confidence 0.16 on a real track).

---

## 15. Performance budget

Targets are checked, not hoped for. A synthetic library generator produces 250k fixture assets, and a benchmark suite runs in CI on every release build.

| Metric | Target |
|---|---|
| Cold start → first grid paint | < 1.0s |
| Scroll at 250k items | 60fps sustained, no blank tiles above 2000px/s |
| Thumbnail generation, 8 cores | ≥ 200 images/s (JPEG), ≥ 60/s (RAW via embedded preview) |
| Viewer open → preview visible | < 150ms |
| Timeline page query (200 rows) | < 5ms |
| Map cluster query, any zoom | < 30ms |
| Text search, 250k items | < 20ms |
| Memory, idle at 250k | < 500MB RSS |
| Installer size, per platform | < 180MB |

---

## 16. Error handling, testing, packaging

**Errors.** `thiserror` in the crates, one `AppError` at the boundary with a stable `code`. A partly-failed import completes and reports which files failed and why, rather than aborting. Missing originals mark the asset `unavailable` and show a placeholder — never a crash. A missing usbmuxd or FFmpeg sidecar produces a specific, actionable message.

**Testing.**

- `photos-core` — schema migrations, query correctness, keyset pagination boundaries, layout computation
- `photos-media` — decode golden tests against a committed fixture set (JPEG, HEIC, AVIF, JXL, CR3, NEF, ARW, DNG, MOV, MP4, ProRes), metadata extraction, resize correctness
- `photos-import` — ingest against fixture trees, dedup, crash-recovery, incremental behaviour
- `photos-jobs` — priority ordering, cancellation, coalescing, backpressure under load
- Frontend — Vitest + Testing Library for hooks and components; Playwright over `tauri driver` for viewer, import and album flows
- Benchmarks — `criterion` for the pipeline, the synthetic library for end-to-end timings

TDD applies throughout: tests precede implementation.

**Packaging.** GitHub Actions matrix over macOS (universal), Windows (x64) and Linux (x64). FFmpeg and ffprobe binaries are vendored per target triple as Tauri sidecars. macOS builds are signed and notarized; Windows signed if a certificate is available. Tauri's updater plugin ships updates from a static manifest.

---

## 17. Roadmap

Each phase ends with something usable. Only Phases 0–2 should be planned in detail now; later phases get their own plans as they approach.

| Phase | Deliverable |
|---|---|
| **0 — Foundation** ✅ | Workspace, SQLite + migrations, `photos://` protocol with range support, UI shell, routing, oklch theming, dark/light, Framer Motion transitions. Proves the transport. |
| **1 — Catalog** ✅ | Folder import, ingest pipeline, metadata extraction, thumbnails, job scheduler, justified grid, viewer with zoom. **The first genuinely usable build.** |
| **1b — HEIC and video** ✅ | FFmpeg sidecar: probe, poster frames, HEIC decode, playback proxies, video in the viewer, codec status card. |
| **2 — Devices** ✅ | Volume detection, DCIM import, device sections, incremental import, iPhone over USB with Live Photo pairing. |
| **3 — Organisation** ✅ | Albums, favorites, trash with 30-day recovery, batch operations, FTS + structured search, smart albums, ratings, rubber-band selection. |
| **4 — Map** ✅ | Rust clustering over a covering index, a bundled-coastline canvas map, offline reverse geocoding, place search, mini-map in the viewer. |
| **4b — Apple sources** ✅ | Mac-formatted disks (HFS+, APFS) under Connected on Windows without admin rights; Apple Photos library import with favourites, albums, titles, captions, keywords and Photos' own dates and places. |
| **5 — Edits** ✅ | Recipe model, Rust render pipeline, editor UI, derivative invalidation, export. |
| **6 — Memories & slideshow** | Rule-based clustering, memory UI, slideshow document, editor, preview, MP4 export. |
| **7 — ML** | `2026-09-22-lumen-ml-design.md`. **7a:** runtime, model download, image embeddings, beat tracking, music / memory / transition / grouping algorithms. **7b:** faces, people, scene tags, natural-language search. |
| **8 — Develop** | `2026-09-22-lumen-develop-design.md`: labels and flags, the simple editor with video trim, metadata-faithful export (ExifTool) to JPEG/PNG/WebP/AVIF/HEIC, the Develop screen, ML Auto and denoise. |

Cross-platform verification is not a phase. Every phase ships tested on all three OSes; deferring that produces three weeks of platform bugs at the end.

---

## 18. Open risks

| Risk | Severity | Mitigation |
|---|---|---|
| Custom protocol range support insufficient for video seeking | High | Verify in Phase 0. Fallback: loopback `axum` server with a session token. |
| iPhone USB via usbmuxd unreliable on Windows | High | Timeboxed spike before Phase 2. Fallback: MTP on Windows behind the same `ImportSource` trait. |
| libheif cross-platform vendoring | Medium | Prefer a vendored/static build. Fallback: route HEIC through the FFmpeg sidecar, which already ships. |
| Installer size with FFmpeg + ONNX | Medium | Build a reduced FFmpeg with only needed codecs; make ML models a first-run download. |
| Grid performance at 250k despite layout in Rust | Medium | Benchmarks in CI from Phase 1, not after. |
| Preview/export divergence in the slideshow | Medium | Single declarative document; easing and transition constants generated from Rust. |
| Scope creep from Phase 7 pulling earlier | Medium | Nothing in Phases 0–6 may depend on `photos-ml`. |
| `hfsplus` / `apfs` crates are young (0.3, one author) | Medium | Read-only, behind our own `MacFs` enum; a parse failure is a per-file import failure, never a crash. Replaceable without touching ingest. |
| Unelevated partition access may be refused for internal disks by policy | Low | Verified for USB disks, which is the case that matters; a refused partition is simply not listed. |

---

## 19. Handing this to Fable

Phases 0–2 are specified tightly enough to plan against. When expanding:

- The transport contract (§4.2) and the crate dependency rule (§4.3) are load-bearing. Everything else negotiates; these two do not.
- The frontend folder convention (§9.1) is fixed — `gui.tsx` + `styles.scss` + `index.ts` per folder, `@/` alias imports.
- No colour may be written outside `globals.scss`. Every value is a token.
- Any decision that contradicts §3 is a change to this document first, code second.
