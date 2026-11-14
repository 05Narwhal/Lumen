# Lumen — Product & UX Design

**Date:** 2026-09-17
**Status:** Approved
**Parent:** [`2026-09-17-photos-app-design.md`](./2026-09-17-photos-app-design.md) — the architectural spine. This document sits under it and never contradicts it. Where the two disagree, the parent wins and this document is wrong.
**Scope:** identity, storage locations, shell, first-run, themes, every screen, the viewer, interaction rules, states, motion, accessibility.

Feature-level detail (which sliders the editor has, how the slideshow timeline snaps) belongs in the per-phase implementation plans. This document fixes how the app looks, feels and behaves so those plans have something to be consistent with.

---

## 1. Principles

1. **Photos are the content; the UI is a frame.** Chrome is neutral, quiet and small. Colour on screen comes from the photos, plus one accent for selection and primary actions.
2. **One primary action per screen.** Every screen, including every empty state, answers "what do I do here" with a single obvious button.
3. **Reversible by default.** Nothing that can be undone asks for confirmation. Everything that can be undone shows an undo toast.
4. **Never a spinner in the middle of the screen.** Background work lives in one activity ring. Grids paint placeholders instantly.
5. **Keyboard first, mouse equal.** Every action is reachable from the command palette and has a shortcut.
6. **Identical on three platforms.** Bundled fonts, custom title bar, no per-OS feature gaps.

---

## 2. Identity and storage

| Item | Value |
|---|---|
| Display name | **Lumen** |
| Company | JL Studios |
| Bundle identifier | `com.jlstudios.lumen` |
| Window title | `Lumen` (no document suffix) |
| App-data directory | `<config_dir>/JL-Studios/Lumen/` |
| Default library | `<pictures_dir>/Lumen Library/` |

**App-data directory** resolves via the `dirs` crate's `config_dir()`, then `JL-Studios/Lumen`. That is `%APPDATA%\JL-Studios\Lumen` on Windows, `~/Library/Application Support/JL-Studios/Lumen` on macOS, `~/.config/JL-Studios/Lumen` on Linux. This matches every other JL Studios app. **Tauri's identifier-based `app_config_dir` / `app_data_dir` are never used for persistent data.** The directory holds:

```
JL-Studios/Lumen/
  settings.json        appearance, import defaults, playback prefs — see §11
  libraries.json       registry: [{ id, name, path, last_opened_at }]
  logs/                rolling daily logs, 14 days retained
```

**Library** is the managed tree from the parent spec §5.1, identified by the UUID in its `library.json`. The default path is shown, not hidden, so a user can back it up with any tool or carry it on an external drive between machines. The registry (`libraries.json`) is keyed by that UUID and remembers each library's last path on this machine. *Settings → Library* lists every known library with its state (*Open*, *Available*, *Not connected*) and the actions *Open*, *Locate…*, *Forget*, plus *Open other…* and *Create new…*. The app opens the last-used library on launch. If its disk is not connected, the app shows a full-screen "Library not found" state with *Locate…*, *Choose another library* and *Create new library*, and never falls through to an empty grid. When a drive with a known library is plugged in, a toast offers to open it.

---

## 3. Shell

### 3.1 Window

- Minimum window size 900 × 600. Default 1280 × 800, remembered per display.
- **Custom title bar** on all platforms: `decorations: false` on Windows and Linux, `titleBarStyle: Overlay` with `hiddenTitle` on macOS. A 38px drag region spans the full width; the sidebar extends up into it. Since Round 15 any empty part of a top bar drags the window (`useWindowDrag`: everything except buttons, inputs, sliders, tabs and `[data-no-drag]`), including the gaps between a screen's toolbar buttons and Develop's top bar.
- **Window controls:** macOS traffic lights are native and sit at their standard position inside the sidebar's top-left. Windows and Linux get minimize / maximize / close rendered by us, 46 × 38px hit targets at the top-right of the content area, styled by the theme, close turning red on hover as users expect. Double-click on the drag region maximizes on all platforms. They are on every screen: the full-window overlays (viewer, film player, quick look) mount `<WindowChrome autoHide />`, whose controls stay hidden until the pointer reaches their top-right box or focus enters them, and those overlays keep their own buttons left of the controls (`--window-controls-w`).
- **Recovery (Round 15):** a page that stops answering is reloaded by Rust on the same screen; a backend that stops answering shows a banner above the content (*Lumen is not responding.* with **Restart Lumen** and **Wait**), never a modal. The palette offers *Reload window* (the page only) and *Restart Lumen* (recovered in place when it can be, else a new Lumen on the same screen with edits kept).

### 3.2 Layout

```
┌──────────┬──────────────────────────────────────────────┐
│ ● ● ●    │  drag region                        ─ ▢ ✕   │  38px
│ Lumen    ├──────────────────────────────────────────────┤
│          │  Page title            [search…]  [toolbar]  │  56px header
│ Home     │                                              │
│ Library  │                                              │
│ Albums   │                                              │
│ Map      │                content                       │
│ Memories │                                              │
│ Slidesh. │                                              │
│          │                                              │
│ DEVICES  │                                              │
│ iPhone   │                                              │
│ Sony A7  │                                              │
│          │                                              │
│ Import   │                                              │
│ Trash    │                                              │
│ Settings │                                              │
│ ◔ 3 jobs │                                              │
└──────────┴──────────────────────────────────────────────┘
   240px
```

**Sidebar** — 240px, collapsible to a 56px icon rail with `Ctrl/Cmd B` or the button under the wordmark. State persisted. Three groups:

1. `Home · Library · Albums · Map · Memories · Slideshows`
2. **Devices** — one entry per row in `devices`, ordered by most recent import, icon by `kind`. Right-click: Rename, Change icon, Hide. Hidden devices stay reachable from *Library → filter → device*.
3. Pinned to the bottom: `Import · Trash · Settings`, then the **activity ring**.

The active entry has a shared-layout indicator (Framer `layoutId`) that slides between items. Count badges appear only on *Import* (new items on a connected device) and *Trash* (items purging within 3 days).

**Header** — 56px. Page title left (large, theme heading font). Right side: a global **search field** and the page's context toolbar. Search opens the results page (§ *Search results page*, Round 4K); on `/library/:deviceId` it still narrows that device in place and offers "Search all of Lumen". `Esc` clears.

**Activity ring** — a 20px ring in the sidebar footer that fills with aggregate job progress and shows a count. Click opens a popover listing jobs by lane: imports, thumbnails, proxies, exports, maintenance, each with progress and Cancel. It is the **only** place background work is surfaced; screens never show their own spinners for background jobs.

### 3.3 Responsive behaviour

Breakpoints from the parent spec §9.4.

| Width | Sidebar | Header |
|---|---|---|
| ≥ `md` (62em) | Persistent, 240px or rail | Full |
| `sm`–`md` | Drawer, opened by a menu button in the header | Full |
| < `sm` (48em) | Bottom bar: Home, Library, Albums, Map, More | Title only; search behind an icon |

The map and the viewer collapse the sidebar to the rail automatically and restore it on exit.

---

## 4. First-run setup

Runs when `libraries.json` is absent or empty. Four full-screen steps, one decision each, a progress dots row at the bottom, *Back* on the left, the primary action on the right. The wizard renders inside the real theme engine, so step 2 re-themes the wizard itself live.

| Step | Content | Default / skip |
|---|---|---|
| 1 Welcome | Wordmark, one sentence ("Your photos, on your machine, fast."), *Get started* | — |
| 2 Theme | Four theme cards (§5) rendered as live miniatures of the shell with real sample tiles; `Light · Dark · Auto` segmented control under them. Clicking a card re-themes instantly. | Gallery, Auto |
| 3 Library | Path field showing `~/Pictures/Lumen Library`, *Change…*, free-space line. If the chosen folder already contains `library.db`, the primary action becomes *Open existing library*. | Default path |
| 4 First import | Three tiles: *Choose a folder*, any detected DCIM volume or iPhone (live, via §8 of the parent), *Skip for now*. Choosing a source starts the import and lands on Home with the batch in the activity ring. | Skip |

Rules: every step has a default, so the whole wizard is four clicks. Setup is re-runnable from *Settings → About → Run setup again*. Theme and scheme are persisted the moment they are clicked, not on finish.

---

## 5. Themes

Four themes ship. Each has a light and a dark variant so *Auto* (follow the OS) works with any theme. The user picks a theme in setup and can change it in *Settings → Appearance*.

| Theme | Character | Light surface | Dark surface | Accent | Radius | Tile gap | Headings | Extras |
|---|---|---|---|---|---|---|---|---|
| **Gallery** | Pure monochrome, hairlines, contact-sheet grid | white / #fafafa sidebar | #0f0f0f / #161616 | near-black (light) / white (dark) | 4px | 2px | Inter 600 | none |
| **Darkroom** | Dark-first, floating bordered panels | warm grey oklch(93%) | #0e0e10 / #141416 | amber oklch(80% 0.14 80) | 8px | 4px | Inter 600 | sidebar and panels are inset cards with 1px border |
| **Paper** | Warm editorial, whitespace, serif | cream oklch(96% 0.012 80) | oklch(18% 0.012 80) | violet oklch(55% 0.18 290) | 6px | 8px | Fraunces 500 italic | no sidebar border; hairline dividers only |
| **Glass** | Frosted translucent layers over a soft colour wash, pill nav | oklch(95% 0.01 260) + wash | oklch(20% 0.02 260) + wash | gradient blue→violet | 12px, nav 999px | 6px | Inter 600 | `backdrop-filter: blur(20px)` on sidebar, header, popovers; soft shadows |

**Implementation contract**

- `:root[data-theme="gallery" | "darkroom" | "paper" | "glass"]` selects the theme; Mantine's `data-mantine-color-scheme="light" | "dark"` selects the variant. Eight token blocks live in `src/styles/globals.scss`, nothing else in the app knows a theme exists.
- A theme may override **only** these token families: surface and foreground colours, accent, border colour and width, radius scale, tile gap, shadow scale, backdrop blur, heading font family and weight. Spacing, type sizes, motion durations and z-index layers are shared and not themeable. This keeps all four visually distinct and functionally identical.
- The Mantine theme object reads the same custom properties (`--lumen-*`), so Mantine inputs, menus and modals match hand-written SCSS in every theme.
- **Fonts are bundled**, self-hosted via `@font-face` from `src/assets/fonts/`: Inter Variable (all themes), Fraunces Variable (Paper headings). System fonts are the fallback stack only.
- The theme miniatures in setup and Settings are the real shell components rendered at 0.35 scale inside a container that sets `data-theme` locally, so previews can never drift from the real thing.

---

## 6. Screens

Every screen follows: header (title, search, toolbar) → optional filter row → content. Every screen has an **empty state**: an icon, one sentence, one primary button. Never a blank area.

### 6.1 Home — `/`

Sections in this order, each with a title row and a "See all" link where it applies:

1. **Memories** — horizontally scrolling carousel of large cards (3:2, cover image, title, date range), scroll-snap, arrow buttons on hover. Empty: "Memories appear as your library grows" with no button (nothing the user can do).
2. **Recently added** — one row per batch from the last 30 days: source icon, label, date, item count, a strip of up to 12 thumbnails, *Open in Library* (navigates with the batch filter applied).
3. **Devices** — tile per device: icon, name, item count, last import date. Connected devices show a green dot and *Import new*. At most 20, most recently imported first. *Show all N* expands the rest in place, and *Show fewer* folds them again (*Round 4H*). iPads and other tablets have a tablet icon here, in the sidebar and on the Connected and Import pages.
4. **Favorites** — a single justified row of favorites, *See all* opens Library with the favorites filter.
5. **Stats** — three small counters: items, videos, storage used. Not cards, just a line of figures under Favorites.

First-run empty Home (no items at all) replaces all sections with one centered card repeating the three import options from setup step 4.

### 6.2 Library — `/library` and device sections `/library/:deviceId`

- Justified grid (parent §9.6) with **sticky month headers** ("September 2026 · Ljubljana, Slovenia" when the month has a dominant place; count on the right).
- **Tile size**: a three-step control in the toolbar (S ≈ 120px, M ≈ 200px, L ≈ 320px target row height), plus `Ctrl/Cmd +` / `-` and trackpad pinch. Persisted.
- **Filter row** under the header: chips for `Photos · Videos · Favorites · Device · Date range · Has location`. Active chips are accent-filled. A `Clear` link appears when any is active.
- **Timeline scrubber** (*revised Round 4H*): it replaces the native scrollbar in every grid. A hairline track runs along the right edge, and the thumb in the accent colour follows the scroll position. On hover or drag, year labels appear beside the track (the year covering more of the library wins where two would touch) along with month ticks, and a bubble names the date under the pointer, or at the thumb while dragging. Dragging scrolls the grid directly, and clicking a year jumps to its start. A grid without dates (an album in manual order) gets a plain track. During a drag, new tiles show only their colour until it ends, so the drag holds 60 fps.
- Device sections are the same screen scoped by `device_id`. The header shows the device icon, an inline-editable name, the count, and *Import new* when connected.
- Toolbar: tile size, sort (newest / oldest), *Select* (enters selection mode, see §8).

**Search results page (Round 4K).** Enter, or a 300 ms pause in the header field, opens `/search?q=` from every screen (a device or device group in the Library still narrows its own grid). The page shows what the text was read as as chips (*Anna*, *Beach*, *2023*), each removable, and an ambiguous word with a ▾ menu of its other readings (*Florence — the person*); choosing one writes it into the text as a token (`person:"Florence"`). Words nothing recognised follow as "and “red umbrella”". Below, one strip of sections that scrolls sideways (People, Pets, Places with a small map, Albums, Memories, each only when non-empty, each with *See all*), then the photos: *Best match* (no month headers) or *Newest first*, with the count beside the title. A place card narrows the search to that place. With nothing found: "No photos match “…”." and *Clear search*. The palette's `Search: <text>` opens the same page.

**Search as built (Phase 3):** the header field writes the query into the URL (`?q=` plus a compact `?filters=`) after a 200 ms debounce, so a search survives reload, Back and a shared link. A match count sits beside the page title, `Esc` and the field's ✕ clear everything, and a *Filters* popover adds kind, device, location, rating and camera or lens. Every active filter becomes a removable chip under the header. Searching inside a device or album offers *Search all of Lumen*. Typing something the command palette does not recognise offers `Search: <text>`.

### 6.3 Albums — `/albums`, `/albums/:id`

- Index: card grid, 4:3 cover, name, count. Smart albums carry a small filter glyph. Nested albums render as folders with a stacked-cover treatment. Toolbar: *New album*, *New smart album*, sort.
- Album: the Library grid with manual order by default and *Sort by date* as a toggle. Drag to reorder in manual mode (fractional positions, parent §11). Header shows editable name and an *Add photos* button that opens a picker over the Library grid with multi-select.
- Smart album editor: a modal with rule rows (`field · operator · value`) and a live count.

**Smart albums as built (Phase 3):** a smart album is a saved `AssetQuery` in `albums.query_json` with `kind = 1`. It owns no membership rows: its count and its cover come from running the query, so it follows the library as imports arrive. The editor is a modal with the same filter controls the library header uses, plus a live count of what matches. Its card carries a filter glyph, its context menu offers *Edit rules…* instead of *Rename…*, and its detail view drops manual ordering and *Add photos* for a quiet "Updates itself".

**As built (round 2):** the index carries a virtual **Favorites** card first, then manual albums; each card's context menu offers Open, Rename… and Delete album (which confirms only when the album has items, and never touches files). The detail view is the Library grid in album scope with a Manual / By date toggle, an inline-editable title, *Add photos* (the Library grid in a full-screen picker in selection mode) and per-selection menu entries *Set as album cover* and *Remove from album*. Dropping tiles onto an album in the sidebar adds them. Smart albums, nesting and drag-to-reorder are Phase 3's remainder.

### 6.4 Map — `/map`

- Full-bleed MapLibre map; the sidebar collapses to the rail. Cluster markers are rounded squares showing the cover thumbnail with a count pill; leaves show just the thumbnail. Clicking a cluster zooms in; clicking a leaf opens the viewer.
- A **bottom sheet** (collapsed to a 56px handle, expandable to 40% height) lists the photos in the current viewport as a horizontal strip, updated on move-end. Hovering a strip item highlights its marker.
- Toolbar: search places (offline geocoder), *Photos · Videos* toggle, date range.

**As built (Phase 4):** the base map is a bundled coastline drawn on a canvas, so it works
offline with no API key. Drag to pan, wheel or double-click to zoom, arrow keys and `+`/`-`
for the keyboard. A cluster marker shows its cover thumbnail with a count and zooms to
exactly what it holds; a lone photo opens the viewer. *Find a place* searches the bundled
atlas, *Fit library* frames everything geotagged, and the strip along the bottom lists what
is in view. The viewer's info panel carries the same map in miniature with the place name.

### 6.5 Memories — `/memories`, `/memories/:id`

- Index: card grid identical to the Home carousel cards, grouped by kind (Trips, Events, On this day). Hidden memories are reachable via a *Show hidden* toggle.
- Detail: hero image with title and date range, then the curated set as a justified grid, then *All photos from this period* as a collapsed section. Toolbar: *Play* (viewer in auto-advance mode), *Make slideshow*, *Hide*.

### 6.6 Slideshows — `/slideshows`, `/slideshows/:id`

- Index: list rows with a thumbnail, name, clip count, duration, last edited. *New slideshow* and *From memory…*.
- Editor: preview canvas on top (16:9, transport controls under it), timeline strip below (clip thumbnails, widths proportional to duration, drag to reorder, drag edge to trim duration), inspector on the right for the selected clip (duration, transition, motion) or for the document when nothing is selected (resolution, fps, audio). *Export* opens a small sheet (preset, destination) and reports progress in the activity ring; on completion a toast offers *Reveal in folder*.

### 6.7 Import — `/import`

Two columns.

- **Sources** (left, 320px): *Folder…* button and a dropzone; then detected volumes with `DCIM/` and connected iPhones as cards showing name, new item count (incremental import), and *Import*. A missing usbmuxd or ffmpeg produces a persistent explanatory card with a link, in place of a broken source.
- **Batches** (right): running batches on top with progress, throughput and *Cancel*; then history rows with source, date, imported / skipped counts and an expandable list of failed files with reasons. *Open in Library* per batch.

Files and folders dropped from the OS onto **any grid** in the app also start an import and navigate here.

*Choose photos…* imports exactly the files picked; *Choose a folder…* has an *Include subfolders* switch; *Apple Photos library…* picks a `.photoslibrary` and opens its page.

### 6.7a Connected device — `/connected/:kind/:id`

Opened from the sidebar's *Connected* entries. Header card: what it is, how much is new, storage as one bar (photos and videos · other · free, counted in thousands like the label on the device), and the primary *Import N new* action, which turns into a progress bar with the current file. Below it the newest items as small squares; when the device holds more, the last square reads *+N more*.

- `kind = volume | iphone`: as above.
- `kind = mac` (a Mac-formatted disk): the header card, then one card per Photos library on it (name, item count, what will be skipped and why, newest squares, *Import library*), then a folder browser (breadcrumb, folders with their photo counts, *Import this folder* with *Include subfolders*).
- `kind = photos` (a library picked from Import, `id` = its path): the library card alone.

**Device browser (Round 4E).** The *+N more* square and a *Browse…* button on every source (the device card, each Photos library card, the Mac disk's folder browser) open a full-window sheet over the page. Title: the device's name. **Top:** *From* and *To* dates with quick picks (*New since last import*, *Today*, *This week*, *This month*, *This year*, *All*), kind chips (Photos, Videos, RAW, Live), *New only* (hidden where the source cannot know what it gave the library, a folder on a Mac disk) and *Include undated* (only meaningful with a range), then "1,204 of 10,312 · 41 GB on the device", with "Reading dates…" while a card's own dates are being read. **Grid:** 120 px squares under day headers (each with *Select day*), undated last under *No date*. A Live Photo or a RAW with its JPEG is one tile, badged LIVE or RAW; a video has a play badge; an already imported tile is dimmed with a check. A click toggles a tile, Shift extends from the last click, a drag draws a band, Space shows the focused tile larger, `Mod+A` selects everything under the filter. **Bottom:** *Select all (N)* for the filter, *Select none*, *Invert* (within the filter), the selection as "312 selected · 4.2 GB · 12 already in your library · … and 40 not shown", and **Import 312**, which runs the space check first, closes the sheet and leaves the page showing the import's progress. The selection survives every filter change. If the device is unplugged, the sheet says so and offers *Close*; Esc closes it.

### 6.8 Trash — `/trash`

The Library grid, sorted by `trashed_at` descending, each tile carrying a "Deletes in N days" label. Toolbar: *Restore*, *Delete now*, *Empty trash*. *Delete now* and *Empty trash* are the two irreversible actions in the app and confirm with a modal that states the count.

**As built (round 2):** *Restore* and *Delete now* act on the selection and are disabled with nothing selected; the same two actions appear in the tile context menu and in the floating bar, which swaps to Restore / Delete now inside `/trash`. Items are purged 30 days after `trashed_at` by a maintenance job that runs at startup and every six hours, deleting the original and every derivative.

### 6.9 Settings — `/settings`

A list of sections on the left and one section open at a time on the right (Round 5, 2026-10-04; it was one long scrolling page). The open section is in the address (`?section=music`; an old `#music` still opens it), ↑/↓ move through the list, and each entry carries one line of state under its name (*Glass, automatic*, *FFmpeg ready*, *Off*), so most of Settings is read without opening anything. Inside a section, settings that belong together share one surface; each row is its name and one sentence on the left, the control on the right, and wraps below in a narrow window. A section is one entry in `routes/settings/sections.tsx` (`SECTIONS`). Sections as built: Appearance, Library, Playback & codecs, Map, Music, Smart features, People, Keyboard, About. The list below is the original plan:

- **Appearance** — theme (four miniatures), scheme (Light / Dark / Auto), default tile size, reduce motion.
- **Library** — the library list from §2 (every known library with *Open* / *Locate…* / *Forget*, plus *Open other…* and *Create new…*), storage used, *Check integrity* (rehash, reports in the activity ring), *Rebuild thumbnails*.
- **Import** — copy behaviour note (always copy, never move), duplicate handling (skip / import anyway), auto-prompt on DCIM volume (on/off).
- **Playback & codecs** — ffmpeg sidecar status and version, usbmuxd status, proxy cache size and *Clear*.
- **About** — version, *Check for updates*, *Run setup again*, *Show logs*.

---

## 7. Viewer

An overlay driven by `?asset=<id>`, never a route (parent §9.2).

- **Open**: shared-element transition from the tile (`layoutId`), 220ms. Loads 1024 → 2560 → original as the parent §9.7 specifies.
- **Chrome**: top bar (close, filename, date and place, then favorite, info, edit, share, trash) and a bottom filmstrip of neighbours. Chrome hides after 2s without pointer movement and on any keyboard navigation; it returns on movement. `Tab` toggles it explicitly.
- **HDR (Phase 4):** a photo that carries extra dynamic range is served to the viewer as its
  own file rather than as our SDR preview, so a gain map actually shows. Tiles carry a small
  `HDR` badge and the info panel names the kind. HEIC is the exception: the webview cannot
  decode it, so an HDR HEIC shows its rendered SDR copy.
- **Ratings (Phase 3):** `1`–`5` set the rating and `0` clears it, so the zoom shortcuts moved to `Ctrl/Cmd 0` (fit) and `Ctrl/Cmd 1` (actual size). A five-star row sits in the toolbar before the zoom button and again, read-only, at the top of the info panel. Tiles at least 160 px tall show their stars in the bottom-left corner.
- **Toolbar order as built (round 2):** close · filename and subtitle · *favorite* · *info* · *show file* · *star rating* · *fit / actual size* · *trash*. Every button carries a tooltip with its shortcut; edit and share arrive with Phase 5. A loading pill sits above the filmstrip while the preview or full-resolution image is still arriving.
- **Info panel** slides in from the right (320px): EXIF summary, camera and lens, mini-map when GPS exists, histogram, file details, albums containing the asset. **Edit** replaces the panel's content with the adjustment controls (Phase 5) and adds *Revert*, *Cancel*, *Done* to the top bar.
- **Edit mode as built (Phase 5):** `?asset=<id>&edit=1`, from the toolbar's *Edit* (after *Info*), `e`, or the palette's *Edit photo*; stills and Live Photo stills only. The stage shows a 1024 px live preview rendered by Rust on each pause in slider movement; the info column holds three sections — *Light* (exposure, contrast, highlights, shadows), *Colour* (temperature, tint, saturation), *Crop & rotate* (crop tool with Free / Original / 1:1 / 4:3 / 3:2 / 16:9, rotate left and right, flip, straighten). Sliders centre on neutral, show their value in tabular numerals, and a double-click resets one. The top bar reads "Editing IMG_…" with *Revert to original* (when there is an edit), *Cancel* and *Done*. Nothing is stored until *Done*, which toasts "Edit saved" with Undo; *Revert* toasts "Reverted to original" with Undo; *Cancel* and `Esc` discard without asking (`Esc` leaves the crop tool first). Paging, zoom and chrome hiding are off while editing. *Export…* sits in the toolbar and the selection bar: JPEG (quality) / PNG / Original, full size or 4096 / 2048 px, into the library's Exports folder or a chosen one; progress in the activity ring, then a toast with *Reveal*.
- **Video**: native controls hidden; our own scrubber, play/pause, mute, and a loop toggle in the bottom bar. Proxy generation shows progress in place of the scrubber with the text "Preparing playback…".
- **Zoom**: scroll and pinch zoom around the cursor, double-click toggles fit / 1:1, drag pans. `0` fit, `1` 1:1.
- **Keys**: `←` `→` navigate, `Space` play/pause video, `f` favorite, `i` info, `e` edit, `Delete` trash (undo toast), `Esc` close, `Tab` chrome, `Ctrl/Cmd C` copy image.
- **Close** returns to the exact scroll position with the tile in view and focused.

---

## 8. Interaction rules

### 8.1 Selection

- Click opens the viewer. `Ctrl/Cmd` click toggles selection. `Shift` click selects a range. Hovering a tile reveals a checkmark in its top-left; clicking the checkmark toggles selection without opening.
- **Rubber band and arrow keys (Phase 3):** dragging from empty grid space draws a selection rectangle; every tile it brushes is selected, `Shift` adds to the selection and `Ctrl/Cmd` toggles. Dragging within 40 px of an edge scrolls. The arrow keys move a roving focus — left and right walk the flattened order across rows, up and down pick the nearest tile by horizontal centre — `Enter` opens, and `Shift` with an arrow extends the selection.
- **Double-click favorites** (round 2). The single click that opens the viewer is therefore held for 220 ms; a second click inside that window cancels the open and toggles the favorite instead, playing a yellow star burst at the pointer. `f` does the same for the selection and in the viewer.
- Dragging on empty grid space rubber-bands; `Ctrl/Cmd A` selects the visible scope.
- With one or more selected, a **floating action bar** rises from the bottom centre: `N selected · Favorite · Add to album · Share · Trash · ×`. `Esc` deselects.
- Selection state lives in the frontend only, keyed by asset id; it survives scrolling but not navigation.

### 8.2 Actions

- **Right-click** on any tile opens a context menu with the same actions as the floating bar, plus *Open*, *Show in Finder / Explorer*, *Copy*.
- **Context menus everywhere (Round 4G).** Right-click, `Shift+F10` or the context-menu key opens the object's menu (from the keyboard, at the focused element's corner; closing returns focus to it). One host renders every menu; the destructive item comes last after a divider; every action is also in the palette. The surfaces:
  - **Photo tile** (one, or the whole selection when a selected tile is right-clicked): Open · Favorite · Add to album… · Tag people… · Rotate left / right · Show in Explorer / Reveal in Finder · Copy files to… · Move to Hidden · — · Move to Trash.
  - **Viewer's photo:** Favorite · Rate ▸ · Add to album… · Rotate left / right · — · Copy image (the photo as shown, edits applied) · Export… · Develop · Show original in folder · Set as album cover (opened from an album) · Use as main photo of ▸ (the people in it) · Tag people… · Show info · Move to Hidden · — · Move to Trash.
  - **Face box in the viewer:** Tag… (*Tag as someone else…*) · New person… · Use as main photo of {name} · — · Not {name} · Not a face.
  - **Album** (the sidebar and `/albums`): Open · Rename… (Edit rules… for a smart album) · Set cover from… · Hide its photos from / Show them in the Library · Duplicate album · Export album… · — · Delete album (confirms when it holds photos).
  - **Collection card:** Open · Start slideshow · Export…. **Sidebar entries:** Open, and for Library *Open in Develop*.
  - **Memory card:** Open · Export photos… · — · Hide memory (Undo). *Play* and *Make slideshow* join with Phase F's memory films.
  - **Person:** Open · Select · Rename · Merge with… · Groups ▸ · — · Not a person · Remove from People.
  - **Slideshow row** (also its ⋮ button): Open · Duplicate · Rename… · — · Delete (Undo). *Play* and *Export…* join with Phase F.
  - **Import row** (finished): Show in folder · Retry failed · Copy error text · — · Dismiss (Undo; the row leaves the list on this computer, the history stays).
  - **Connected source:** Open · Browse… · Import new · and for a card or drive Import everything again · — · Eject · Forget this device. Read-only: nothing writes to a device; Eject is refused for the library's own drive and during an import.
  - **Library device** (a camera or phone in *Devices*): Open · Rename… · — · Hide device.
  - **Map pin:** Open photos here (Open photo) · Start slideshow of these · Add to album… · Zoom to fit.
  - **Develop's canvas:** Before / after ▸ (Toggle, Split, Side by side; the mode on is ticked) · Show / Hide clipping · Zoom ▸ (Fit, Fill, 100 %, 200 %) · — · Copy settings… · Paste settings · Reset develop · — · Flag ▸ · Label ▸ · Export…. **Develop's filmstrip** keeps its menu.
  - **Any text field:** Cut · Copy · Paste · — · Select all (Undo · Redo above them in a text area), instead of the menu of whatever the field sits in. Cut and Copy need a selection; a read-only field offers Copy and Select all; a password is never copied out.
- **Drag and drop**: tiles onto a sidebar album add them; tiles onto a sidebar device do nothing; OS files onto any grid import.
- **Undo**: trash, remove from album, favorite toggle, edits, reorders and renames all show a toast, bottom-left, for 6 seconds with *Undo*. Toasts stack to three; older ones collapse.
- **Confirm** only *Delete now*, *Empty trash*, *Delete library*, *Delete album with N items* (albums with zero items delete silently).

### 8.3 Keyboard

| Key | Action |
|---|---|
| `Ctrl/Cmd K` | Command palette: every action and navigation target, fuzzy search |
| `1`–`6` | Home, Library, Albums, Map, Memories, Slideshows |
| `/` | Focus search |
| `Ctrl/Cmd B` | Toggle sidebar |
| `Ctrl/Cmd I` | Import folder |
| `Ctrl/Cmd +` `-` `0` | Tile size larger, smaller, reset |
| `Ctrl/Cmd A` | Select all in scope |
| `Enter` | Open selected or focused tile |
| `Space` | Quick look: opens the 1024px thumbnail in a lightweight overlay without the full viewer chrome; `Space` again closes |
| `Delete` | Trash selection |
| `Ctrl/Cmd ,` | Settings |

The grid supports roving focus with arrow keys; the focused tile has a visible ring.

---

## 9. States

| State | Treatment |
|---|---|
| Empty | Centered icon, one sentence, one primary button. Never a blank grid. |
| Loading grid | ThumbHash placeholders paint immediately with `dominant_color` behind them; tiles fade in on decode. No skeletons in grids. |
| Loading cards/lists | Skeleton rows matching final layout, max 800ms before content or an error. |
| Background work | Activity ring only. A screen may show a slim progress line under its header while its own job runs (import on the Import page, export in the editor). |
| Error (recoverable) | Inline banner at the top of the content area: `AppError.message`, *Retry* when the action is idempotent, *Details* toggling `detail`. |
| Error (blocking) | Full-screen state: library not found, database locked, unsupported schema version. Each offers the specific fix. |
| Missing sidecar / usbmuxd | Persistent card in Settings and Import with the exact package name and a link. Never a raw socket error. |
| Unavailable original | Tile keeps its thumbnail with a small broken-link badge; viewer shows the preview plus a banner *Original file is missing*. |
| Offline map tiles | Map shows themed blank land with markers still placed; a banner says tiles are unavailable. |

---

## 10. Motion and accessibility

**Motion** follows the parent §9.5 exactly. Durations are tokens: `--motion-fast: 120ms`, `--motion-base: 180ms`, `--motion-slow: 260ms`, viewer open `220ms`; easing `cubic-bezier(.2,.8,.2,1)`. Framer Motion is used for: route transitions, sidebar indicator, viewer open/close, floating action bar, toasts, setup wizard steps, card hover lift. Grid tiles use CSS opacity on load and nothing else. `MotionConfig reducedMotion="user"` plus the Settings toggle, which sets `data-reduced-motion` on the root and zeroes every motion token.

**Accessibility**

- Every interactive element is reachable by keyboard with a visible 2px focus ring in the accent colour, offset 2px, in all eight theme variants.
- Text contrast meets WCAG AA in all eight variants; this is checked by a Vitest test that reads the token blocks and computes contrast for every `fg`/`bg` pair.
- Tiles are `button` elements with `aria-label` of the filename and date. The grid is `role="grid"` with roving `tabindex`.
- The viewer traps focus while open and restores it to the tile on close.
- All icons carry labels; icon-only buttons have tooltips after 600ms hover.
- Hit targets are at least 32 × 32px; window controls 46 × 38px.

---

## 11. Settings model

`settings.json` in the app-data directory, written atomically (temp file + rename), read once at startup and mirrored into TanStack Query. Unknown keys are preserved on write so downgrades are safe.

```json
{
  "version": 1,
  "appearance": { "theme": "gallery", "scheme": "auto", "tile_size": "m", "reduce_motion": false, "sidebar_collapsed": false },
  "import": { "duplicates": "skip", "prompt_on_volume": true },
  "playback": { "loop_videos": false, "mute_by_default": false },
  "window": { "width": 1280, "height": 800, "x": null, "y": null, "maximized": false }
}
```

---

## 12. Frontend structure additions

Beyond the parent §9.1, this design implies these folders. Names are fixed so implementation plans can reference them.

```
src/
  routes/
    setup/               first-run wizard (steps as child components)
    home/  library/  albums/  map/  memories/  slideshows/  import/  trash/  settings/
  components/
    app-shell/           title bar, drag region, window controls, sidebar, header slot
    sidebar/
    activity-ring/
    search-field/
    photo-grid/          justified grid, tiles, sticky headers, year scrubber
    photo-tile/
    selection-bar/       floating action bar
    viewer/              overlay, chrome, filmstrip, info panel
    theme-preview/       scaled shell miniature used by setup and settings
    empty-state/
    command-palette/
    toast/
  styles/
    globals.scss         eight theme token blocks, shared tokens, resets
    themes/              one partial per theme, imported by globals.scss
      _gallery.scss  _darkroom.scss  _paper.scss  _glass.scss
  assets/fonts/          Inter Variable, Fraunces Variable
```

---

## 13. Phase mapping

What this document adds to each roadmap phase of the parent §17.

| Phase | UX deliverables |
|---|---|
| 0 Foundation | App shell, custom title bar and window controls, sidebar with rail, header, activity ring, all four themes × light/dark, bundled fonts, theme preview component, first-run wizard (steps 1–3), Settings → Appearance and Library, empty states, toast system, command palette skeleton, JL-Studios app-data directory and `settings.json`. |
| 1 Catalog | Library grid with headers, tile size control, year scrubber, filter chips, selection model and floating bar, context menu, viewer with chrome, info panel, quick look, Import page (folder source, batches), wizard step 4, Home with Recently added and Stats. |
| 2 Devices | Sidebar Devices group, device sections, Home Devices tiles, volume and iPhone source cards, incremental "new items" badges, missing-usbmuxd card. |
| 3 Organisation | Albums index and detail, smart album editor, drag to sidebar, search results, favorites on Home, Trash page, batch actions. |
| 4 Map | Map screen, cluster markers, bottom sheet, place search, mini-map in the viewer info panel. |
| 5 Edits | Edit mode in the viewer, adjustment controls, revert, export sheet. |
| 6 Memories & slideshow | Home Memories carousel, Memories screens, slideshow list and editor, export progress. |
| 7 ML | People section in the sidebar, natural-language search; designed in its own document. |

---

## 14. Open questions resolved

| Question | Decision |
|---|---|
| One theme or several? | Four ship; chosen in setup, changeable in Settings. |
| Home screen or land in Library? | Separate Home. |
| Native or custom title bar? | Custom on all platforms. |
| Library default location | `~/Pictures/Lumen Library`, visible and changeable. |
| App-data location | `config_dir/JL-Studios/Lumen`, the JL Studios convention. |
| Fonts | Bundled Inter and Fraunces; never system-dependent rendering. |
| Confirmation dialogs | Only for the four irreversible actions in §8.2. |
