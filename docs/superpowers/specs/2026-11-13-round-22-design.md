# Round 22 — design (Julian's list of 2026-11-13)

Three of the list's four items in ten sections, three lanes, then a backup build. The fourth item
(themes and plugins) is Round 23 with its own spec, written after this round's build exists
(**J**, 2026-11-13). Roadmap and lane plans: `plans/2026-11-13-round-22-*.md` (written after this
spec is approved).

Each section is self-contained: a chat reads only the sections its plan block names. **J** marks what
Julian decided on 2026-11-13; **Call** a choice made while writing this, open to his veto;
**Builder** what the implementing session decides; **Verify** a cause read from the code or the
evidence but not yet reproduced: write the failing test first, and if it passes before the fix, stop
and hand off instead of guessing. Line numbers are from `main` at `5da0f87` and drift; Grep the
symbol when a line is off.

| § | Item | Lane |
|---|---|---|
| 1 | A square over the photo while hovering a preset | A |
| 2 | Images too large to decode open reduced, then in full | A |
| 3 | Repair cycles: it swaps a file for the same file | B |
| 4 | Recover what's readable | B |
| 5 | The Repair page (layout C); *Files Lumen can't open* leaves Settings | B |
| 6 | Settings → Storage: *Clear temporary files* | B |
| 7 | External apps: the list, detection, icons | C |
| 8 | *Edit in* / *Open with*: what is sent and how it comes back | C |
| 9 | Menus, shortcut, Settings → *External apps* | C |
| 10 | Backup build `26.1.1-alpha` | Final |

Not in scope: themes, plugins and the extension store (Round 23; §7's `AppSource` is the only hook
left for them); Develop and export of an oversized image at full size; charts on the Repair page; a
repairs history table; writing PSD; a stacks UI; publishing a release; locking originals by changing
their permissions.

Rules that hold everywhere (a chat does not read CLAUDE.md): commands are `#[tauri::command(async)]`
returning `Result<T, AppError>`; TS types come from ts-rs (`cargo test -p photos-core` writes
`src/types/generated/`), never hand-edited; no colour, radius, shadow, duration or z-index outside
tokens in `src/styles/globals.scss`; a component is a folder with `gui.tsx`, `styles.scss`,
`index.ts`; `@/` imports; never style a row's descendants by tag; every menu comes from a factory in
`src/lib/menus/`; reversible actions never confirm (6 s undo toast); lists are keyset-paged
(`{ items, next_cursor }`, about 200); bytes never go through `invoke` (images travel over
`photos://`); a missing tool or app is a state, never an error. **Data safety:** nothing under
Julian's library folder (`C:\Users\Sample\Pictures\Lumen Library`) or on a device is ever written by
a test or a replay; tests build their own library and source in temp dirs; devices and disks are
read-only sources.

---

## §1 A square over the photo while hovering a preset (lane A)

**J:** hovering a preset must look exactly like applying it. His screenshot: a photo with a
background mask carrying motion blur; hovering *B&W* turns the photo grey except a rectangle around
the subject (the car), which keeps the colour and a different blur.

**Verify.** The rectangle is the mask's bounding box. Round 21 A3 made area tools (motion blur, glow,
halation) read only their own mask's pixels; the likely cause is that patch being composited from
the committed recipe (or at another scale or crop) while the rest of the frame comes from the hover
preview's recipe.

**Tests first.** (1) Rust: a recipe with one mask carrying motion blur, rendered with a B&W preset
merged in, has no coloured pixel inside the mask's bounding box, and equals the render of the same
recipe with the preset applied for real. (2) If (1) passes before any fix, the Rust render is right
and the fault is how the frontend builds the hover request (**Builder:** Grep the preset hover in
`src/routes/develop/workspace.tsx` and the preview command it calls); write the failing frontend
test there. If neither fails, stop and hand off with what was seen.

Applying the preset (a click) goes through the same check: one test covers both paths.

## §2 Images too large to decode open reduced, then in full (lane A)

**J:** a big image is not a damaged image. It opens at a smaller size; when the viewer opens, the
full resolution loads in the background with a loading indicator, so zooming in finds it ready.

**What exists.** `classify_ingest_failure("decode failed: Memory limit exceeded")` is `TooLarge`
(`src-tauri/src/derivatives.rs:646-666`), but `ingest_now` (`:707-719`) flags every decode failure
`unreadable`, so Julian's 846 MB `biome_map….png` sits among the damaged files and fails Repair
every time. The limit is the `image` crate's default: `photos-media/src/decode.rs` opens files
with `image::ImageReader` and sets no `Limits`. (**Verify:** that this file fails with the memory
limit; read its header for the pixel size, read-only.)

- **Ingest.** On `TooLarge`, decode again at reduced size with bounded memory whatever the pixel
  count (**Builder:** FFmpeg scaling while decoding, or a row-by-row PNG / JPEG reader; the bound is
  about 512 MB). Thumbnail, thumbhash, colour and the display derivative come from that. The asset
  is **not** flagged `unreadable`. Stored width and height stay the real ones (from the header).
- **Oversized is computed**, not stored: real pixel count above the decoder's limit (**Call:** no
  column, no migration).
- **Viewer.** Opens on the display derivative as always. For an oversized photo it then asks for
  the full resolution in the background: a small ring and *Loading full resolution…* by the zoom
  control until ready; zoom uses it from then on. Closing the viewer cancels the load.
  **Call:** full resolution is made only when it fits a memory budget (**Builder:** measure; about a
  quarter of the machine's RAM); otherwise the largest size that fits, with the line
  *Shown at n % — larger than this PC can open in full*.
- **Develop** on an oversized photo works on the reduced size and says so in one line; export of
  the untouched original copies the file (**Call**; full-size develop is out of scope).
- Files flagged before this change are read again by Repair's *Check the library* (§5); no start-up
  pass.

**Tests first:** a generated PNG above the limit (temp dir) ingests with a thumbnail and no
`unreadable` flag; memory stays under the bound (**Builder:** how to assert); the viewer's request
for full resolution is cancelled on close.

## §3 Repair cycles: it swaps a file for the same file (lane B)

**J:** Repair shows no progress, ends on 2 images, and on the next run all 54 are back. The photos
exist only in one folder on his PC; if they are damaged that is fine, as long as Lumen says so once
and stops offering to repair them.

**Evidence (read-only, 2026-11-13).** `<library>/kept-aside/2026-11-12/` holds 326 files, 3.6 GB:
56 distinct files plus 270 numbered repeats (`name (2)` …). All 56 are byte-identical to the
originals now in their place. FFmpeg fails on 55 of 55 tested (35 HEIC decode errors, 16 *Invalid
data*, 4 PNG); the 56th is §2's PNG, kept five times. So the source holds the same bytes as the
library: every run copied a file over itself, counted it repaired, and kept one more copy aside.

**Cause in the code (read, 2026-11-13).** `by_identity` (`photos-import/src/repair.rs:231-257`)
classes an item `damaged` by name, size and `source_dir` when the library's copy is flagged, never
by content. `swap` (`:613-687`) accepts a candidate whose hash belongs to the same asset
(`find_by_hash` returns it, step 2 only refuses *another* asset), moves the original to
`kept_aside_path` (`:594`, which numbers a taken name), and its `UPDATE` sets `unreadable = 0`
(`:676`) before anything has decoded the file. That is the whole cycle: after a run every swapped
photo is unflagged, so the list shows only the few that were skipped (the "2 images"); ingest then
fails on the same bytes and flags all of them again (the 54 that come back). The job
(`src-tauri/src/repair.rs:241-296`) counts each swap as repaired.

**Rules after this round**
1. **Check.** For each `damaged` candidate (only those: few files), hash the source file and, when
   the hash differs from the library's, test that it decodes (a **probe**: a function passed in,
   which the binary fills with `photos_media::decode_preview_with`; `photos-import` already depends
   on `photos-media`). Same hash as the library's copy, or it does not decode → new class
   **`sourceDamaged`** (*Damaged on <source> too*): not tickable for re-import; its actions are
   *Recover what's readable* (§4), *Look somewhere else* (back to the source chips) and *Remove*.
2. **Swap.** Before the damaged original is moved, the temp copy must differ in hash from it and
   must pass the probe. Otherwise the temp file is removed and nothing is kept aside.
3. **A second run changes nothing.** With rules 1 and 2 no file is kept aside twice; a test holds
   the file count and bytes of `kept-aside/` equal across two runs. A damaged original is still
   never deleted.
4. **Repaired means it reads.** `unreadable` is cleared only after rule 2's probe passed.
5. **Progress.** `repair://progress` carries `phase` (`listing`, `checking`, `probing`, `copying`,
   `verifying`, `done`), the current file's name, `done` / `total`, and bytes for a file over
   100 MB. The page shows phase and name; the activity ring shows the count.
6. **Outcome.** Every item of a run ends with one outcome (`repaired`, `imported`, `sidecar`,
   `sourceDamaged`, `sameFile`, `duplicate`, `failed` with its reason); the page lists them under
   *Last run* and the summary line counts each.

**Tests first** (temp library + temp source): a damaged original whose source copy is identical →
`sourceDamaged`, no swap, `kept-aside/` unchanged; a second run changes nothing (idempotence: file
count and bytes of `kept-aside/` equal); a source copy that differs but does not decode → not
swapped; a good copy → swapped once, asset id, albums and edit kept; progress events name a phase
and a file; a 0-byte and a truncated file each get a reason.

## §4 Recover what's readable (lane B)

**J:** try to recover as much as possible, show *xx % recovered* per photo, and let him keep it or
not. A kept recovery replaces the damaged photo. The decision is made in Repair, not in Storage.

- **Rust, new `photos-media/src/recover.rs`** (pure, unit-tested on files damaged by the test). It
  tries, in order, and keeps the best result:
  1. **Partial decode:** rows (JPEG, PNG) or tiles (HEIC) that decode before the damage. Missing
     rows at the end are cropped off, so the result is a clean, smaller photo. `percent` = decoded
     pixels / the header's pixels.
  2. **Embedded preview** (EXIF thumbnail, preview image, a HEIC's thumbnail item; ExifTool or
     FFmpeg, **Builder**): reported as *Preview only · W × H*, with `percent` = its pixels / the
     header's.
  3. **Video:** what FFmpeg can remux while ignoring errors; `percent` = recovered length /
     the header's length. No index (`moov`) → nothing.
  Nothing readable → `None`.
- **Output** goes to `<library>/recovered/<asset id>.<jpg|mp4>` (JPEG quality 95; **Call**). That
  folder is Lumen's own derived data: never scanned as originals, emptied of an asset's file on
  *Keep* or *Discard*.
- **Repair list** (§5) gets a *Recovered (n)* chip: thumbnail, name, *n % recovered* or *Preview
  only*, **Keep** / **Discard**; header **Keep all** / **Discard all**. The thumbnail opens large.
- **Keep** is §3's swap with the recovered file as the candidate: the damaged original moves to
  `kept-aside/`, the asset keeps its id, albums, rating, date and edits, its extension and hash
  change (**Builder:** what `swap` step 4 needs when the extension changes), `unreadable` clears,
  and `assets.recovered_pct` (migration **0039**, `INTEGER NULL`) records the figure. The same
  migration adds `replaced_originals(content_hash PRIMARY KEY, asset_id, kept_rel,
  old_relative_path, old_extension, old_byte_size, replaced_at)`: what undo needs, and the reason
  a later check of the source does not offer the damaged file as *Not in the library* (its hash is
  known; `by_bytes` answers `safe`). §3's swap writes the same row. The info panel shows
  *Recovered n % of a damaged file*. Reversible: 6 s undo toast (swap back), no dialog.
- A staged recovery is described by `<library>/recovered/<asset id>.json` beside its file
  (**Call:** no table) and shown through `photos://recovered/<asset id>`.
- **Discard** deletes the staged file; the row returns to *Needs attention* as *Recovery discarded*
  and can be run again.
- **Nothing readable:** the row says so; its only action is *Remove* (Trash, 30 days, exists).

**Tests first:** a JPEG cut at 60 % of its bytes recovers a cropped image with a percent between 40
and 70; a file with only an EXIF thumbnail reports *preview only*; random bytes → `None`; Keep keeps
the asset id and album, sets `recovered_pct`, leaves the damaged bytes in `kept-aside/`; undo
restores them; Discard leaves the library as it was.

## §5 The Repair page; *Files Lumen can't open* leaves Settings (lane B)

**J:** layout **C** of the mockups (`.superpowers/brainstorm/236-1794569162/content/repair-layouts.html`,
git-ignored; described here in full). Settings holds nothing about repair any more.

One column, at most about 820 px wide, three blocks:

1. **Health card.** *21,809 of 21,865 open fine*; one stacked bar (healthy, can't be opened, file
   missing or changed, recovered: the facts the library knows without a source; missing sidecars
   belong to a source's check); a legend with counts; *Checked <when>*; button **Check the library** (the existing
   `LibraryCheck` job, moved here from Settings → Library). While any check or run is going, the
   bar is its progress and the line under it reads phase and file (§3 rule 5), with *Cancel*.
   Never checked: *Not checked yet* and the button.
2. **Check against** — one chip per source (`list_repair_sources`: volumes, iPhones, Apple disks)
   plus *A folder…*. An unpaired iPhone's chip is off with *Unlock and tap Trust* as its hint. More
   chips than fit wrap to two lines, then *More ▾* (**Builder**).
3. **List card.** Chips: *Needs attention n · Recovered n · Last run · History*; to the right one
   muted line of causes (*35 incomplete · 16 not an image · 4 damaged PNG*). Rows: checkbox,
   thumbnail or placeholder, name with date and size, cause, what is readable or the outcome, one
   action. Footer: **Recover what's readable (n)**, **Remove selected**.
   After a source is checked, the same card shows that check: a crumb *← Library health*, chips
   *Not in the library · Can be repaired · Damaged on source too · Sidecar missing · In Trash*, and
   **Re-import selected (n)** (Round 21's selection model is kept).

- **Causes** come from the reader's own message, which the derivative store keeps per hash
  (`DerivativeStore::failure_path`, `failed/<hh>/<hash>.txt`): a new pure
  `photos_media::failure::cause(text) -> UnreadableCause` (`incomplete`, `notAnImage`, `tooLarge`,
  `unknown`) sorts it (lane A owns `derivatives.rs` and its private classifier; lane B does not
  edit it). The last *Check the library* result is kept in a small JSON file beside `library.db`
  so the page opens with it (**Call:** a file, no table).
- **Settings:** remove `src/routes/settings/unreadable.tsx` and `check.tsx` with their rows in
  `sections.tsx` (*Files Lumen can't open*, *Check a folder*, *Check the library*). Every former
  entry (palette entries, `src/routes/home/attention.tsx`, `?assets=`) points at
  `/import?mode=repair`. Commands stay.
- **Empty state:** *Every photo in this library opens fine.* and **Check the library**.
- No charts (J chose C over the dashboard).

## §6 Settings → Storage: *Clear temporary files* (lane B)

**J:** one clean-up button in Storage, and a warning there when temporary folders pass a threshold.

`src/routes/settings/storage.tsx` gains a *Temporary files* group: one row per kind with its size —
*Replaced by Repair* (`kept-aside/`), *Unfinished copies* (`*.repair.part` and import leftovers),
*Recoveries not kept* (`recovered/`), *Conversion cache* (`cache/dng` in app data) — and
**Clear temporary files** (all ticked; each row has a checkbox). Above **2 GB** in total
(**Call**, one constant) the page shows a warning line: *Temporary files use 3.6 GB.*

- Commands `get_temp_usage` and `clear_temp(kinds)`.
- `kept-aside/` holds the only copies of replaced originals, so it goes to the **OS Recycle Bin**
  (**Builder:** what Lumen Setup's library removal uses); caches are deleted. No dialog; a toast
  says how much was freed (**Call**).
- **Data safety:** before removing anything, the resolved path must lie inside
  `<library>/kept-aside`, `<library>/recovered` or the app-data cache; a test proves a path outside
  is refused. Tests use temp libraries only.

## §7 External apps: the list, detection, icons (lane C)

**J:** show only apps that are installed, each with its logo; detect every major photo editor;
ArtCraft's apps (PhotoCraft and its siblings) by default; plugins must later be able to add apps.

- **Profile:** `AppProfile { id, name, kind: Editor | Developer | Video, accepts: [Photo | Raw |
  Video], reads_raw: bool, writes: InPlace | Sidecar | Never, win, mac, linux }` (the lane plan's
  *Names used across the blocks* is the exact form). `Editor` saves over the file it is given (Photoshop);
  `Developer` keeps its own catalogue or sidecars (Lightroom, Capture One); `Video` never writes
  its sources (Resolve).
- **Sources:** `trait AppSource { fn apps(&self) -> Vec<AppProfile> }` with two implementations
  now, the built-in table and the person's custom apps; plugins add a third in Round 23
  (**Call**). Module `src-tauri/src/external_apps/` (**Builder** may put the pure parts in
  `photos-core`).
- **Built-in table** (**Builder:** verify each app's identifiers per OS and drop what cannot be
  verified; nothing is guessed): Photoshop, Photoshop Elements, Lightroom Classic, Lightroom,
  Affinity Photo / Affinity, GIMP, Krita, Paint.NET, Capture One, DxO PhotoLab, darktable,
  RawTherapee, Luminar Neo, ON1 Photo RAW, Pixelmator Pro and Photomator (macOS), DaVinci Resolve,
  Premiere Pro, Final Cut Pro (macOS), and ArtCraft's apps that open photos or video (read the
  project's own site and repository for names, install paths and identifiers).
- **Detection:** Windows: `App Paths` and the Uninstall keys in HKLM and HKCU, then known install
  folders; macOS: bundle identifiers; Linux: `.desktop` files in the XDG data dirs, Flatpak and
  Snap exports. The result is cached and refreshed when a menu opens more than 60 s later.
- **Icons** are read from the installed app (exe resource, `.icns`, the `.desktop` icon), cached as
  64 px PNG under app data `cache/app-icons/`, and served at `photos://app-icon/<id>`. A generic
  icon stands in. Nothing is bundled, so no licence question arises.
- **Custom apps** (*Choose another app…*): path and name in the app's settings file; `Editor`,
  always given a copy.
- Commands: `list_external_apps(media)`, `add_custom_app(path)`, `remove_custom_app(id)`,
  `get_default_apps`, `set_default_app(media, id)`; type `ExternalApp { id, name, kind, iconUrl,
  custom }` (ts-rs).

**Tests first:** detection against a fake registry / folder tree per OS (pure functions take the
lookups as arguments); a missing app is absent, not an error; a custom app survives a restart.

## §8 *Edit in* / *Open with*: what is sent and how it comes back (lane C)

**J:** match what Lightroom users know from its Photoshop integration; an editing app gets a copy
beside the original, marked with the app's icon like the RAW badge; an app that does not modify its
files gets the original; an app that gains from a format gets that format; an existing copy is used
again when it is the same picture.

Lightroom's way, for reference: *Edit in Photoshop* (Ctrl/Cmd+E) makes `<name>-Edit.tif` beside the
original with Lightroom's adjustments, and Ctrl+S in Photoshop brings it back.

| App kind | Lumen sends |
|---|---|
| `Editor` | a **copy**, `<name>-Edit.<ext>`, as a new photo beside the original |
| `Developer`, `writes: Never` | the original itself |
| `Developer`, `writes: Sidecar` | a copy (so no stray sidecar lands in `originals/`; **Call**) |
| `Video` | the original itself |

- **What the copy is** (per app in Settings, default *Automatic*; **Call** made at Julian's "what
  do you suggest"): *Automatic* sends a 16-bit TIFF rendered through `edits::export_recipe` (draft
  first) when the photo has Lumen adjustments, and a copy of the original file (the RAW stays RAW)
  when it has none and the app reads that format. *With Lumen adjustments* and *Original file*
  force one. Lumen never hands an `Editor` the original itself (non-negotiable 11).
- **A 16-bit TIFF needs a new export format.** `ExportFormat` (`photos-core/src/export.rs`) has
  `jpeg | png | webp | avif | heic | original`; lane C adds `tiff` (the `image` crate's `tiff`
  feature is already on in `photos-media`). **Verify** that the render pipeline carries 16 bits to
  the encoder; if it is 8-bit, the TIFF is 8-bit this round and the as-built note says so. The
  Export dialog does not offer TIFF yet (**Call**).
- **The copy is a file beside the original** in `originals/`, in the source's folder, named
  `<stem>-Edit.<ext>` (`-Edit-2` … when taken). Migration **0038**: table `external_edits(id
  PRIMARY KEY, source_asset_id, asset_id UNIQUE NULL, app_id, relative_path, made_key, made_hash,
  seen_size, seen_mtime, created_at)` and column `assets.edited_in TEXT NULL` (read by the grid
  without a join).
  - A **rendered** copy has new bytes: it becomes an asset at once (catalogued in place, with the
    source's capture date and metadata, so it sorts beside it).
  - A copy of the **original file** has the source's bytes, and `assets.content_hash` is `UNIQUE`,
    so it cannot be an asset yet: its row waits with `asset_id NULL` and it is catalogued the
    moment its bytes change. *Check the library* skips paths that `external_edits` names.
  After writing a copy, Lumen itself never writes that file again; the person's editor does.
- **Coming back.** No watcher (no new dependency): when Lumen's window regains focus and at
  start-up it compares size and time of every `external_edits` file with `seen_size` /
  `seen_mtime`. A file whose time is under 2 s old is left for the next look (the app may still be
  writing). A changed file is re-hashed, its metadata
  re-read and its derivatives made again (what §3's swap does after a rename). A file saved beside
  it under the same stem (`<name>-Edit*.{tif,tiff,psd,psb,jpg,png}`, which is what *Save As* from a
  RAW produces) is brought in as a further copy. A PSD shows through FFmpeg's flattened composite
  (**Verify**); one that does not read is an ordinary *can't open* state.
- **Used again.** `made_key` = hash of the source's content hash, its recipe and the format. *Edit
  in* reopens an existing copy when its `made_key` matches and its file still has `made_hash` (the
  app has not saved over it). *Edit in* on a copy itself opens that file, with no new copy.
- **An original handed over** (`Never`, `Video`): remembered for the session; its size and time are
  checked when Lumen regains focus; a change is re-hashed and, if real, a toast says *<App> changed
  <name>. The original was modified outside Lumen.* and the next *Check the library* lists it as
  *file missing or changed*. **Call:** no lock. None of the three systems can bar one program from a file without changing the file's
  permissions for everything, which would be a write to an original.
- **Badge.** The tile carries the app's icon where the RAW badge sits (**Builder:** Grep the RAW
  badge under `src/components/photo-tile/`), tooltip *Edited in <app>*; the list payload gains
  `editedIn?: string`. No animation. An *Edited in* tag with the app as its value joins tag search
  (**Builder:** where the RAW tag is defined).
- **Several photos** are sent in one launch where the app takes several paths; copies are made on
  the job lane with the activity ring; a toast reads *Opening n photos in <app>*. No dialog.
- **Launch** through `tauri-plugin-opener` (already a dependency) or `std::process::Command`
  (**Builder**). An app that has gone: a toast (*<App> isn't installed any more*) and the list
  refreshes.

**Tests first:** the table above as a pure function (kind, media, has adjustments, app reads RAW →
what is sent); a copy keeps the source's date and gets its `external_edits` row; a changed copy is
re-hashed and its thumbnail remade; the reuse rule (same key and untouched → reused; saved over →
new copy); a *Save As* sibling is picked up; an `Editor` never receives a path equal to the source's.

## §9 Menus, shortcut, Settings → *External apps* (lane C)

**J:** two entries, a default app and an *Open with* submenu; *Choose another app…* at the bottom
of the submenu; Rotate, Tag people and Move to Hidden move under *More*.

- **Photo menu** (`src/lib/menus/photo.ts:42-66`), new order: Rate · Open · Favorite · Add to
  album… — **Edit in <default>** (app icon, `Mod+E`) · **Open with ▸** — **More ▸** — Move to Trash.
  - *Open with ▸*: the installed apps that fit the selection, each with its icon · divider · Show
    in Explorer · Copy files to… · divider · Choose another app….
  - *More ▸*: Rotate left · Rotate right · Tag people… · Move to Hidden.
- No default set: the first entry is absent and *Open with ▸* stands alone. A mixed selection
  (photos and videos) shows only apps that take every selected kind.
- The viewer's menu (`viewer.ts`) and Develop's get the same two entries from one shared factory
  in `src/lib/menus/parts.ts`. Palette entries *Edit in <default>* and *Open with…*. `Mod+E` joins
  `src/lib/keybinds.ts`, rebindable (**Builder:** confirm it is free).
- A menu item takes an image icon (`iconUrl`) beside the component icon (one change in the menu
  component).
- **Settings → Connections → External apps** (**Call** for the group; a section `external-apps`
  in `src/routes/settings/sections.tsx`, added to the `connections` group's `tabs`): three defaults (Photos,
  RAW, Video), each a select with icons and *None*; the detected apps, an `Editor` with its *Send*
  choice (Automatic · With Lumen adjustments · Original file); the copy's bit depth (16 or 8) and
  colour space (as Export offers); custom apps with *Remove*; **Choose another app…**.
- `src/lib/menus/menus.test.ts` and `library.test.ts` follow the new order.

## §10 Backup build `26.1.1-alpha` (final chat)

**J:** before the plugin work (which becomes `26.2.0`), a build with this round in it, as an
installer kept on disk. Not published; the previous version was not either.

After the three lanes are merged and the full checks pass: set the version to `26.1.1-alpha`
wherever `26.1.0-alpha` is recorded (`src-tauri/tauri.conf.json:4` and `:64`; **Builder:** follow
`docs/RELEASING.md` for the rest), add the round to `docs/CHANGELOG.md`, build Lumen and Lumen
Setup, copy the installer to `dist-release/26.1.1-alpha/`, and tag `v26.1.1-alpha` locally. No
signing key exists yet, so the build is unsigned and `scripts/release/publish.ps1` is not run.
