# Round 21 — design (Julian's list of 2026-11-12)

Eighteen items in fourteen sections, three lanes (roadmap: `plans/2026-11-12-round-21-roadmap.md`).
Each section is self-contained: a chat reads only the sections its plan block names. **J** marks what
Julian decided on 2026-11-12; **Call** a choice made while writing this, open to his veto;
**Builder** what the implementing session decides; **Verify** a cause read from the code but not yet
reproduced: write the failing test first, and if it passes before the fix, stop and hand off instead
of guessing. Line numbers are from `main` at `c022b4d` and drift; Grep the symbol when a line is off.

| § | Item (Julian's list) | Lane |
|---|---|---|
| 1 | Crop shows a spinner and never the photo | A |
| 2 | Mask bar over the sliders; hover highlights over the mask edge | A |
| 3 | Right-click menu on masks; reorder masks (layering) | A |
| 4 | Motion blur (and glow, halation) pick up pixels outside their mask | A |
| 5 | Brush: Alt + right-drag for size and feather; size down to 1 px | A |
| 6 | Import → Repair: check a source, list, re-import | B |
| 7 | *Files Lumen can't open*: Restore and Remove | B |
| 8 | One iPhone listed twice | B |
| 9 | Import history: ten rows, then *Show all* | B |
| 10 | *Hide RAW files*: the name, and only on the unfiltered Library | C |
| 11 | Window buttons on the loading screen | C |
| 12 | Waking the PC reloads Lumen | C |
| 13 | The Develop filmstrip survives reloads; Home's *In Develop* is that filmstrip | C |
| 14 | Memory and slideshow music ready before the click | C |

Not in scope: reading new disk formats (HFS+ and APFS are already read by `photos-import/src/apple/`),
a repairs table or any migration, mask-aware sharpen / texture / clarity / grain, changing how masks
are stored, a Mac-only code path. `plans/2026-11-12-import-repair.md` (a stub) is replaced by §6.

Rules that hold everywhere (a chat does not read CLAUDE.md): commands are `#[tauri::command(async)]`
returning `Result<T, AppError>`; TS types come from ts-rs (`cargo test -p photos-core` writes
`src/types/generated/`), never hand-edited; no colour, radius, shadow, duration or z-index outside
tokens in `src/styles/globals.scss`; a component is a folder with `gui.tsx`, `styles.scss`,
`index.ts`; `@/` imports; never style a row's descendants by tag; every menu comes from a factory in
`src/lib/menus/`; reversible actions never confirm (6 s undo toast); lists are keyset-paged
(`{ items, next_cursor }`, about 200); bytes never go through `invoke`. **Data safety:** nothing
under Julian's library folder or on a device is ever written by a test or a replay; tests use temp
dirs; devices and disks are read-only sources.

---

## §1 Crop never shows the photo (lane A)

**Cause (Verify).** `src/hooks/use-develop-session.ts:323-334`. Entering crop sets
`uncropped = tool === "crop"` and renders `withoutGeometry(draft)`. The request key is
`JSON.stringify([assetId, shown, before, clipping, renders])`: it does not carry `uncropped`. For a
photo with no crop, turn, flip or straighten, `withoutGeometry(draft)` serialises the same as
`draft`, so no request is sent, the last preview keeps `uncropped: false`, and
`src/routes/develop/crop-stage.tsx:22` (`p.uncropped ? p.url : null`) waits for ever.

**Fix.** A preview of a recipe without geometry *is* the uncropped photo. In `send` (`:308`) set
`uncropped: req.uncropped || !hasGeometry(req.recipe)`, with
`hasGeometry = (r) => !sameRecipe(r, withoutGeometry(r))` in `src/lib/develop.ts` beside
`withoutGeometry` (unit-tested). The key stays as it is: a photo with geometry already re-renders
(its `shown` changes), one without needs no second render and crop opens at once.

**Test first** (`src/routes/develop/crop.test.tsx`): a photo whose saved recipe is the identity,
one `develop_preview` answered, press **R** → the crop image is in the document, no spinner, and
`develop_preview` was called once. A second case with a cropped recipe still sends a second request.

## §2 Mask mode overlaps (lane A)

Two defects, both CSS, no markup change.

1. **Slider thumbs paint over the pinned mask bar.** `.develop-masks__bar` is `position: sticky`
   with `z-index: var(--z-mask-bar)` (= 2, `globals.scss:190`); Mantine's slider thumb carries its
   own higher z-index in the same stacking context. Fix in
   `src/components/develop-slider/styles.scss`: `.develop-slider { isolation: isolate; }`, so a
   row's inner z-indexes stay inside the row and every row sits under the bar. Do not raise the
   token.
2. **Hover highlights cover the 3 px mask edge.** The edge is
   `box-shadow: inset 3px 0 0 var(--panel-accent)` on `.develop-panels[data-mask-mode]`
   (`src/components/develop-panels/styles.scss:8-10`); an inset shadow lies under its children, and
   slider rows, mask rows and buttons start at x = 0. Fix: in that rule add
   `padding-inline-start: 3px` (keep the shadow). Nothing can then reach the edge: this covers every
   hover in the column (Julian's "all hover effects"), including the pinned bar.

**Look** (browser shim, Glass and Gallery, column at 260 px and 360 px): scroll sliders under the
bar; hover *Opacity*, *Amount*, a mask row, *Add*, *Done*.

## §3 Mask menu and mask order (lane A)

**Today.** `src/components/develop-masks/gui.tsx`: rows at `:359-389` (click selects, an eye
button), no context menu; the bar's ⋯ menu at `:204-227` is hand-written (*Reset adjustments,
Rename, Duplicate, Delete*). Masks apply in array order, later ones over the result of earlier ones
(`photos-media/src/develop/masks.rs:219`).

**One factory** `maskMenu(mask, index, count, act)` in `src/lib/menus/masks.ts` returning
`MenuItem[]` (`@/components/context-menu`), used by the row's right-click **and** the bar's ⋯
(rendered with `Items`, replacing the hand-written entries):

`Rename · Duplicate · Invert (checked when on) · Hide / Show` — divider —
`Bring to front · Bring forward · Send backward · Send to back` (disabled at the ends) — divider —
`Reset adjustments · Delete` (red).

Right-click selects nothing; it acts on the row under the pointer. Open it the way other rows do
(**Builder:** Grep `onContextMenu` in `src/components` for the existing helper).

**Order. Call:** the list reads like layers: the mask applied **last** is the **top** row (the list
is drawn from the array reversed; storage and rendering do not change). *Bring forward* moves a mask
later in the array.
- Pure `moveMask(masks, id, toIndex): Mask[]` in `src/lib/develop-masks.ts`, unit-tested first
  (ends, same index, unknown id returns the input).
- Drag: Framer Motion's `Reorder.Group` / `Reorder.Item` (already a dependency; no new one) on the
  list, vertical only. The order is local while dragging; on drop one
  `applyMasks(..., "Reorder masks")`, so undo is one step. No drop, no history entry.
- Keys on a focused row: `Alt+↑` / `Alt+↓` move it one place (declare both in
  `src/lib/keybinds.ts`'s `develop` group as `develop.mask-forward` / `develop.mask-backward`).
- Every menu action is one `applyMasks` with its own label (`"Invert mask"`, `"Bring mask to front"`, …).

Tests: `moveMask`; one render test (right-click a row → the menu lists the entries; *Send to back*
reorders). Update the existing order assertions in `gui.test.tsx` for the reversed list.

## §4 Area tools read only their own mask (lane A)

**Cause.** `photos-media/src/develop/masks.rs:177-198` runs an area tool on a padded copy of the
mask's box and blends `out = src + m·(effect(src) − src)`. The effect itself reads **every** pixel
in the box, so a motion blur on "everything but the car" drags the car's pixels into the
background. Tools: `local_tools.rs:45-50` (`Sharpen, MotionBlur, Glow, Halation, Grain`).

**Fix: weight the samples.** For a tool that spreads pixels, the mask is also the *source* weight.
- **MotionBlur** (a linear filter `K`): `effect_w = K(src·w) / K(w)` per channel, where `w` is the
  mask weight `m` of each pixel; where `K(w) < 1e-4` keep `src`. Then the usual blend with `m`.
  Run `K` twice: once on the box premultiplied by `w`, once on a box whose channels are `w`.
- **Glow, Halation** (they add spread light): the light source is premultiplied
  (`effect(src·w)` gives the added light; add it to `src`, blend with `m`). No division.
  **Builder:** if either is not expressible as `src + spread(source)`, feed it `src·w` for its
  spread input only and leave the rest; say so in *As built*.
- Sharpen, Grain, texture, clarity, dehaze: unchanged (not in scope).
- Add `Tool::weighted(&self) -> bool` (or a small enum) in `local_tools.rs`; the unweighted path
  stays byte-identical. The global (unmasked) effects in `develop/mod.rs:267-279` are not touched.

**Tests first** (`photos-media`, beside the existing mask tests):
1. A 64×32 image, left half red, right half blue; a mask of exactly the right half (hard edge);
   horizontal motion blur longer than 16 px → every pixel with `m = 1` is still blue within 1e-3.
   Fails today.
2. A mask of 1 everywhere → the result equals today's path within 1e-4.
3. Glow with a white square outside the mask → pixels inside the mask do not brighten.

`mask-golden.json` is regenerated by its Rust test if it moves (never by hand); say so in the
commit. Preview speed: time one 1500 px preview with a motion-blur mask before and after; if it
more than doubles, note it in the hand-off (do not optimise in this round).

## §5 Brush: size and feather by dragging, sizes down to 1 px (lane A)

**Today.** `src/lib/develop-masks.ts:21-44`: `brush.radius` is a share of the photo's long edge,
`feather` and `flow` 0–1. Sliders in `develop-masks/gui.tsx:299-329` (*Size* 0.5–30 %). The ring is
drawn in `src/routes/develop/mask-overlay.tsx:158,353-356`; `Alt` while painting erases (`:197`).
Keys `[` `]` and `Shift+[` `Shift+]` exist (`keybinds.ts:205-208`).

**Gesture (Call: Photoshop's on Windows).** While painting is on, **Alt + right mouse button,
drag**: sideways changes size, up and down changes feather (up = softer). On macOS the same with
`Ctrl+Option` + drag. During the drag nothing is painted, the ring stays where the press began and
follows the new size and feather live, and a small label beside it reads `24 px · feather 60`.
The browser's context menu is suppressed on the canvas while painting.
- Pure `brushDrag(start: BrushSettings, dx, dy, longEdgeOnScreen, min): BrushSettings` in
  `develop-masks.ts`, unit-tested first: `radius = clamp(start.radius + dx / longEdgeOnScreen)` (the
  ring's edge follows the pointer one to one), `feather = clamp(start.feather − dy / 200, 0, 1)`.
- `mask-overlay.tsx` `paintDown`: `e.button === 2 && e.altKey` (or the Mac chord) starts the drag
  with pointer capture; release ends it. Left-button painting and `Alt` = erase are unchanged.
- `keybinds.ts`: `k("develop.brush-drag", ["Alt+RightDrag"], "Drag sideways for brush size, up and
  down for feather", true)` (fixed, like `develop.wheel`).

**Size in pixels, down to 1 px.** The stored value stays `radius` as a share of the long edge (no
recipe change). The *Size* slider shows the brush's **diameter in pixels of the full photo**
(`2 · radius · longEdgePx`), from **1 px** to 60 % of the long edge, on a logarithmic scale
(`d = dMax^t`, `t` the slider's 0–1 position) so small sizes are reachable; label `24 px`.
`[` and `]` multiply and divide by 1.1 with the same floor. **Builder:** find the photo's
full-resolution size already known to the panel (not the preview's `p.width`).

**Rust (Verify).** A stroke one pixel wide must still mark the mask at render size and fade, not
vanish, in a smaller preview. Test first in `photos-media` (brush rasteriser, `masks.rs:323` on):
a horizontal stroke with `radius = 0.5 / 6000` rendered 6000 px wide has coverage ≥ 0.5 on its
line; rendered 1500 px wide the line's coverage is > 0. Lower any radius clamp that prevents it.
Check `quantise` in `develop-masks.ts` keeps stroke points to at least 1e-5 of the frame.

---

## §6 Import → Repair (lane B)

**J:** pick a source, Lumen checks it against the library and lists what is wrong, every row has a
checkbox with a *Select all*, and *Re-import selected* copies them in. The source selector comes
first because the Windows folder dialog cannot open an iPhone or an Apple-formatted disk.

**What exists.** `photos-import/src/check.rs` (`check_folder`: hashes each file, looks it up by
`assets.content_hash`; writes nothing), its job and commands in `src-tauri/src/library_check.rs` and
`commands/library_check.rs` (status, cancel, paged items: copy this shape). Sources:
`ImportSource` trait objects in `src-tauri/src/connected.rs:16-52` (`VolumeSource`, `IphoneSource`),
`list_volumes` (`commands/devices.rs:22`), iPhones (`src-tauri/src/iphones.rs` `current()`), Apple
disks (`commands/apple.rs:97` `list_mac_disks`, `photos-import/src/apple/`). Import already
verifies copies and keeps sidecars (`asset_sidecars`, `import_skips`, `assets.source_dir`).

### Screen
The Import page gets a segmented switch under its title: **Import · Repair** (`?mode=repair`).
Repair has three steps on one page:

1. **Source.** One list in three groups, each row `icon · name · detail · button`:
   - *This computer*: every volume from `list_volumes`. Button *Choose folder…* (the OS dialog,
     opened at that volume's root); removable volumes also get *Whole disk*. (**Call:** no
     whole-disk check of a fixed disk.)
   - *Devices*: each iPhone; an unpaired one reads *Unlock the phone and tap Trust*, button off.
   - *Apple-formatted disks*: each disk from `list_mac_disks`.
   An empty group shows one muted line (*No device plugged in*). The list refreshes on the
   existing device events.
2. **Check.** Progress inline and in the activity ring, *Cancel*. Then the result:
   `Not in the library (n) · Library copy damaged (n) · Sidecar missing (n) · In Trash (n)` as
   filter chips, and a line *n files are safely in the library*.
3. **List.** Keyset-paged rows: checkbox, name, folder on the source, size, status. *Select all*
   ticks every row of the current filter, loaded or not (selection is
   `{ all: true, status, except: [ids] }` or `{ ids: [...] }`). *In Trash* rows start unticked and
   cannot be ticked (hint: *Put it back from Trash*). Button **Re-import selected (n)**.
   Empty result: *Everything on <source> is safely in the library.*

### Check (Rust, new `photos-import/src/repair.rs`, pure and unit-tested on temp libraries)
For each media file the source lists, in this order:
1. **Identity match** without reading the file: same file name, same byte size, and (when the asset
   has one) same `source_dir`. An intact match → `safe`, not listed. (**Builder:** confirm the
   column names for the stored original name and size.)
2. An identity match whose library original is **damaged** → `damaged`. Damaged means
   `assets.unreadable = 1`, or the original is missing on disk, or its size on disk differs from
   the catalogue. Also `damaged`: same name and `source_dir` but a different size, when the
   library's copy is flagged unreadable.
3. No identity match → hash the source file (as `check_folder` does): hash in the library →
   `safe`; in Trash → `in_trash`; else `new`.
4. For a `safe` or `damaged` asset, each sidecar on the source (the same pairing rules import
   uses) that `asset_sidecars` lacks → one `sidecar` item.
Types (ts-rs): `RepairSourceRef { kind: "folder"|"volume"|"iphone"|"macDisk", id }`,
`RepairSource { ref, name, detail, ready, removable }`,
`RepairItem { id, name, dir, status, assetId?, byteSize }`,
`RepairStatus { phase, done, total, counts, cancelled }`.
Commands (`commands/repair.rs`, job in `src-tauri/src/repair.rs`): `list_repair_sources`,
`start_repair_check(source, only_assets?: Vec<i64>)`, `get_repair_status`,
`get_repair_items(status, after, limit)`, `cancel_repair`, `start_repair_import(selection)`.
The check writes nothing. A Mac disk that is not yet an `ImportSource` gets a thin adapter over its
existing listing and read functions (**Builder**).

### Re-import (the data-safety block)
- `new` → the normal import pipeline (copy, verify, sidecars); one `import_batches` row named
  *Repair · <source>*, so it shows in History.
- `sidecar` → the existing sidecar copy; one `asset_sidecars` row.
- `damaged` → **swap**, in this order, each step checked before the next:
  1. Copy the source file to a temp name in the original's folder; hash it while copying; re-read
     and compare (import's own verify).
  2. If that hash already belongs to **another** asset: delete the temp file, report *already in
     the library as another photo*, change nothing.
  3. Move the damaged original (if it exists) to
     `<library>/kept-aside/<YYYY-MM-DD>/<its path under originals>`. **Never delete it.**
  4. Rename the temp file to the original's path. Update that asset's hash and size, clear
     `unreadable`, re-read its metadata, schedule its derivatives (what `retry_unreadable` does,
     `src-tauri/src/derivatives.rs:1224-1243`).
  A failure at any step leaves the library as it was (remove the temp file; step 3 undone if 4
  fails). Albums, ratings, edits and the asset id are kept: only the file is replaced.
  (**Call:** `kept-aside/` lives in the library folder, is never scanned, and Lumen never empties it.)
- Result line: *n imported · n repaired · n sidecars · n skipped*, and a toast.

**Tests first** (temp library + temp source folder; never a real path): classification of each
status; a truncated original swapped for the good file keeps its asset id, album membership and
edit, clears `unreadable`, and leaves the damaged bytes under `kept-aside/`; a failed copy changes
nothing; the duplicate-hash case; idempotence (a second run lists nothing).

## §7 *Files Lumen can't open*: Restore and Remove (lane B)

`src/routes/settings/unreadable.tsx` (commands `list_unreadable`, `retry_unreadable`,
`commands/library.rs:113-137`). Add per row **Restore…** and **Remove**, and in the header
**Restore all…** and **Remove all** beside *Try all again*.

- **Restore…** goes to `Import?mode=repair&assets=<ids|unreadable>`: the Repair source step with a
  heading *Where is a good copy of these n files?* If the files' `import_batches.source_path` (via
  the asset's batch) still exists as a folder, it is offered first as a one-click row
  (*Look in <path>*). The check runs with `only_assets`, lists only `damaged` rows, all ticked.
- **Remove (J: Trash, 30 days).** The grid's own trash command (**Builder:** Grep the command behind
  *Delete* in `src/lib/menus/library.ts`), with the 6 s undo toast, no confirm. Trashed unreadable
  files leave this list; a line under the header reads *n in Trash · Put back* (restores them) while
  any exist. They are purged by the normal 30-day rule.
- `list_unreadable` gains the trash filter it needs (`state`); one Rust test.

## §8 One iPhone listed twice (lane B)

**Cause (Verify).** usbmuxd reports a phone once per connection (USB and Wi-Fi sync) with the same
UDID. `photos-import/src/iphone.rs:102-121` (`list_iphones_blocking`) and `:142` on
(`list_iphones_named`) push every entry. **Fix:** one helper `unique_phones(devices)` used by both:
one entry per UDID, the USB connection preferred (**Builder:** the field on `idevice`'s
`UsbmuxdDevice`). Unit test with two entries of one UDID. Also check the sidebar keys its rows by
`udid`.

## §9 Import history: ten, then all (lane B)

`src/routes/import/gui.tsx:486` renders every batch. Show the **ten newest** (a running import is
always shown and counts as one); under the table one quiet button *Show all 34* ↔ *Show fewer*.
Component state only, not remembered. One render test (12 batches → 10 rows and the button).

---

## §10 *Hide RAW files* (lane C)

- **Name.** `src/routes/settings/sections.tsx:177`: **Hide RAW files** — *When a camera saved RAW
  and JPEG, the Library shows only the JPEG. Searches, filters and albums show everything.* The key
  `hide_raw_with_jpeg` stays. It already covers every RAW format (`photos-core/src/formats.rs:5-16`).
  Grep `NEF` in `src/` for other labels and mocks of this switch and rename them.
- **Rule (J): only the unfiltered Library.** `photos-core/src/scope.rs:114-116`
  `hides_raw_pairs`: today it hides under any search unless the search asks for RAW, so an album
  tag in the Library (*To edit in Trips › …*) whose RAWs have their JPEGs elsewhere gives
  *0 match*. New rule: `on && self.is_plain_library()`, where `is_plain_library` is true only when
  the scope is the Library timeline with nothing narrowing it: no trash, no album, no query, no
  tag sequence, no person, place, date or other filter the struct carries (**Builder:** read the
  struct; every optional narrowing field must be empty). Remove `asks_for_raw` if nothing else
  uses it.
- **Tests first** (`scope.rs`): plain Library hides a RAW with a JPEG sibling; the same scope with
  any tag or text does not; an album does not; counts agree with the rows.

## §11 Window buttons on the loading screen (lane C)

Two splashes exist: the static one in `index.html:18-47` (before the bundle runs) and React's in
`src/routes/boot.tsx`. Neither has window buttons; a stuck start cannot be closed.
- **`index.html`:** three buttons in the top corner of `.boot-screen` (minimise, maximise, close;
  46 × 38 px, the title bar's glyphs as inline SVG, colours from `--boot-fg`), outside the drag
  region. A small inline script calls
  `window.__TAURI_INTERNALS__.invoke("plugin:window|minimize" | "plugin:window|toggle_maximize" | "plugin:window|close", { label: "main" })`,
  only when `__TAURI_INTERNALS__` exists (hidden in the browser). **Builder:** if the title bar's
  Close calls an app command instead (Round 15: Close never blocks), call that one. On macOS
  (`navigator.platform` starts with `Mac`) the group sits top-left in the order close, minimise,
  maximise.
- **`boot.tsx`:** render the existing window-controls component (Grep `WindowControls`).
- One test for `boot.tsx` (buttons present while not ready). **Look:** `yarn tauri dev`, a cold
  start: the buttons work before the wordmark fades.

## §12 Waking the PC reloads Lumen (lane C)

**Cause (Verify).** `src-tauri/src/watchdog.rs`: the page beats every 2 s; `ui_frozen` (`:145`)
calls the page frozen when the last beat is older than `UI_FROZEN` = 10 s, and `check_page` (`:200`)
then ends the WebView2 renderer and reloads. Time asleep counts on Windows, and the page's timers
resume later than the watchdog thread, so the first tick after waking sees a "frozen" page.

**Fix.**
- The watchdog loop keeps its previous tick. `woke(prev, now) = now − prev > 5 × TICK`: the process
  itself was suspended. On a wake: set the stored beat to `now` and skip page and backend checks
  for `WAKE_GRACE = 30 s`. Pure functions, unit-tested first (a 1 s gap is not a wake; a 2 h gap
  is; no reload inside the grace; a real freeze after the grace still reloads).
- `src/lib/health.ts` (the page's check of a stuck backend): the same gap rule on its own interval,
  so waking does not flash the *stuck* banner.
- **The 30 s start after a reload** is not explained by the code read. Add one log line per boot
  gate in `boot.tsx` (`settings`, `library`, `resume`, each with its milliseconds) through the
  existing log command, so the next slow start names its cause. Do not guess a fix.

## §13 The Develop filmstrip survives; Home's *In Develop* is that filmstrip (lane C)

**Cause.** `src/lib/develop-source.ts:30,58`: `developSourceStore` and `lastDevelopPhoto` live in
memory. A reload resets the source to empty and only the last photo comes back. Home counts every
`develop_drafts` row (`photos-core/src/home.rs:38-42`, 58) but opens Develop with the six ids it
drew (`src/routes/home/continue.tsx:33`).

**J:** Lumen remembers the last filmstrip, across reloads, restarts and relaunches, and Home's card
shows and opens that set.

- **Store** the source in app data, per library: `develop-source.json` beside the library's other
  app-data state (`config_dir/JL-Studios/Lumen/…`; **Builder:** follow how `library-checks.json`
  is placed). Content: the `DevelopSource` as the store holds it (a scope, or `only` ids) and the
  last photo id. Commands `get_develop_source`, `set_develop_source(source, last)`. Not the DB.
- **Page:** hydrate both stores from `get_develop_source` before the Develop route first renders
  (in `Boot`, with the settings); a store subscriber writes back, debounced 500 ms and on
  `FLUSH_REQUESTED`. Ids no longer in the library are dropped when read.
- **Home** (`home.rs`, `continue.tsx`): the card's count is the remembered filmstrip's size (for
  `only`: the ids still shown; for a scope: the scope's count), its six thumbnails are the last
  photo first, then the strip's order, and a click calls `openDevelop(navigate, remembered, id)`.
  The card's small line is the source's label (*Selection*, an album's name). With nothing
  remembered, the card falls back to photos with Develop work, and then opens **all** of them
  (new command `list_draft_ids`, newest first, capped at 5,000), never six of 58.
- **Tests first:** Rust round trip of the file and dropped ids; `home.rs` count for an `only`
  source; a store test (hydrate → `only` restored; a change is written once after the debounce).

## §14 Music ready before the click (lane C)

**Goal (J).** After launch, a click on a memory or a slideshow starts at once: the music is already
chosen and the cut already made.

**Known.** Play on a memory calls `memory_film`, cached in `memory_films`, built by
`build_for_memory` and cut by `beat_sync` (STATE.md line 20; `src-tauri/src/track_profiles.rs`,
`music.rs`, `commands/slideshows.rs`, `photos-core/src/music_profile.rs`, migration 0028).

**Design.** A background step on the maintenance lane, below normal priority, started 20 s after the
library opens and paused while an import or a video preparation runs:
1. Profile every track that has no profile yet.
2. For the twelve newest memories and every saved slideshow that has no valid cached film or plan,
   run exactly what Play runs (track choice, beat sync) and store it where Play looks.
3. Run again when a memory is created or a slideshow is saved.
It shows in the activity ring as *Preparing memories*. Missing FFmpeg or music is a state: skip.
Nothing here changes which track is chosen.

**Builder first:** read those files, add a log line with the milliseconds Play waits for each part,
and prepare the slow parts only. **Acceptance** (temp library with one memory and one slideshow):
after the step has run, Play logs a cache hit and starts in under a second; a unit test proves the
step is idempotent (a second run builds nothing).
