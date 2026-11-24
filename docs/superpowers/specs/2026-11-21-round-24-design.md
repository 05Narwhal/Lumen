# Round 24 — design

Date: 2026-11-21. Owner: Julian. Status: approved in conversation (answers in *Julian's answers* are
decided; do not reopen). Plan: `docs/superpowers/plans/2026-11-21-round-24-plan.md`.

## Intent

A round of Develop fixes Julian hit in a long editing session, a crash that recovery made worse, a
built-in plugin store, and two first-party plugins that push the plugin platform forward: **Disk
Atlas** (Julian's file explorer as Lumen's file dialog and *Show in folder*) and **Wayfinder**
(finds where photos without GPS were taken, a fast Rust rewrite of the idea behind
[FindMyPhoto](https://github.com/M-Enzinger/findmyphoto)). Both plugins are built **only** with the
plugin platform: anything they need that the platform lacks is added to the platform as a general
feature any plugin can use. No line of Lumen names Disk Atlas or Wayfinder.

## Goals

1. §1 Dragging a mask to reorder it never opens that mask's settings.
2. §2 Masks rename inline (double-click a row or the bar name, or *Rename*), no popup.
3. §3 The mask menu is compact: order items in an *Arrange ▸* submenu, tighter rows.
4. §4 The Develop before/after view never shows a broken *before* image (seen after a full reset).
5. §5 *Copy settings* can also copy readable JSON to the system clipboard; *Paste settings* applies
   JSON from the clipboard.
6. §6 The Develop crash (halation on an ellipse mask, zoomed in, long session) is found and fixed;
   after any crash or forced restart Lumen opens **Home with an alert**, never a stuck loading
   screen, and Close always closes.
7. §7 A plugin store page: browse, install, update, remove; upload shown but disabled.
8. §8 Plugin platform 1.1: provider contributions (file picker, reveal), app detection, a settings
   page per plugin, host-run ONNX models, library read, estimated places, original GPS writes.
9. §9 The Disk Atlas plugin, plus a `--select` flag in Disk Atlas if it lacks one.
10. §10 The Wayfinder plugin and its review screen.
11. §11 The originals rule changes to allow confirmed metadata writes and lossless edits.

## Non-goals

- **Mask layering (occlusion) is dropped.** Julian changed his mind: masks keep adding up in order
  as today; *Arrange* still changes the order. No recipe version change.
- Uploading plugins from Lumen (disabled button only). Paid plugins, ratings, comments.
- Lossless trim/crop **to originals** (allowed by §11, built in a later round).
- Inline rename anywhere but masks.
- Changing an existing GPS position, in any file, ever (Wayfinder only fills missing GPS).

## Anti-goals (a failure even if it works)

- Any write to an original that is not verified before it replaces the file, or that touches more
  than the GPS tags.
- Wayfinder writing anything without the person pressing *Write N locations*.
- Lumen code that special-cases a plugin id, name or executable.
- A store that can install an unsigned package, or an update signed by a different key.
- A recovery loop: relaunching into the work that just crashed.

## Constraints

- CLAUDE.md non-negotiables stand, with #11 changed by §11. Next migration **0040** (taken by §8.7).
- Crate rule: `photos-plugins` depends on `photos-core` only. Anything that needs `ort`, ExifTool
  or Tauri is a trait in `photos-plugins` implemented in the binary (`src-tauri/src/plugins/`).
- Plugin sandbox stays: no filesystem, network or process access from wasm. New powers are host
  functions behind manifest permissions that the install sheet lists.
- Licences: Julian's ruling: Lumen is free, never monetised; GeoCLIP-derived weights are listed
  under *Non-commercial weights* (CC-BY-NC-4.0) in `docs/THIRD-PARTY-MODELS.md`, which the rule
  allows. FindMyPhoto is AGPL: **no code is ported**; Wayfinder is written from the GeoCLIP paper
  and the GeoCLIP repository (MIT). FindMyPhoto is credited for the idea in the plugin's README.
- Session budget: opus chats ≤ 70k tokens, sonnet chats ≤ 100k.

## Approaches (for the plugins)

1. **General platform features (chosen).** Manifest-declared ONNX models run natively by the host
   (GPU via the existing `ort` stack); new permissions for library read, estimated places and
   original GPS writes; a generic place-review screen fed by any plugin; declarative providers and
   app detection (Disk Atlas needs no wasm at all). Fast, reusable, sandbox intact.
2. Plugins render their own UI as sandboxed web pages: flexible, but a large new security surface.
3. Plugins ship native helper processes: fastest to write, breaks the no-process rule, one build
   per OS.

Julian picked 1.

---

## §1 Drag never opens a mask

`src/components/develop-masks/gui.tsx`: rows are `Reorder.Item`s with `onClick={() => select(id)}`
(:403) and `onDragEnd` → `drop()` (:224–229, :402). A drag that ends on the row fires the click.
Fix: a `draggedRef` set on drag start (or when the pointer moved more than the drag threshold),
cleared on the next tick after `onDragEnd`; `onClick` returns early while it is set. A plain click
still selects. One render test: drag-and-drop a row → selection unchanged; click → selected.

## §2 Inline rename

Today `rename()` (gui.tsx:196–199) calls `promptName({ title: "Rename mask" })`. Replace it with an
inline text field in place of the name, in the mask row and in the pinned mask bar (:239):

- Opens on double-click of the row name or bar name, on menu *Rename*, and on `F2` with a mask
  selected (add the keybind to `src/lib/keybinds.ts` so it is rebindable).
- Field takes the name's font and width; text is selected on open.
- `Enter` or blur saves (trimmed; empty → unchanged); `Esc` cancels. One undo step *Rename mask*.
- While editing, row drag and the row's click/select are off; keys don't reach Develop shortcuts.
- Double-click on a row no longer does anything else.

A small `InlineName` component in `src/components/inline-name/` (folder convention), used only by
masks this round. One render test (double-click → type → Enter saves; Esc cancels).

## §3 Compact mask menu

`maskMenu()` in `src/lib/menus/masks.ts`: *Bring to front / Bring forward / Send backward / Send to
back* move into one *Arrange ▸* submenu (same items, same disabled rules, same shortcuts shown).
Menu becomes: Rename, Duplicate, Invert ✓, Hide, ─, Arrange ▸, ─, Reset adjustments, Delete. Rows
get tighter: the menu's item padding and icon size come down one step **for this menu only** via a
`dense` option on the menu factory output (a token in `globals.scss` if no size token fits). The
pinned bar's *Mask options* overflow uses the same items.

## §4 Before image after a full reset

The before pane (`src/routes/develop/canvas.tsx:473`, `:479`) shows `bv.url` from
`session.beforeView`; `use-develop-session.ts:337–350` keys it on `[assetId, beforeRecipe(draft)]`
and fetches `develop_before` → `mediaUrl("edit-preview", token)`. Suspected: after *Reset all* the
key still matches but the backend dropped the token's preview, so the `<img>` 404s (broken-image
icon and alt text, as in Julian's screenshot). **Verify first**: reproduce in the browser shim or
app (edit, *Reset all*, toggle before/after split), confirm the 404. Fix at the narrowest layer:
the before view is refetched when its token is no longer served (an `onError` on the `<img>`
triggers one refetch per key), and/or the backend keeps a before token alive while any session
references it. Never show alt text: while loading or after a failed refetch, show the after image's
placeholder treatment. If the 404 is not the cause, fix the real cause in the same block when it
is small (≤ ~50 lines); otherwise record what was seen and hand off.

## §5 Settings through the system clipboard

Copy/paste lives in `src/lib/develop.ts`, `src/lib/menus/develop.ts`, `src/components/panel-choice/`.

- *Copy settings* keeps copying to Lumen's clipboard and **also** writes readable JSON to the
  system clipboard (`@tauri-apps/plugin-clipboard-manager` if present, else
  `navigator.clipboard.writeText`). One action, both clipboards.
- Format (pretty-printed, 2-space):
  ```json
  {
    "lumen": "develop-settings",
    "version": 1,
    "app": "26.x.y-alpha",
    "settings": { ...the same object Lumen's clipboard holds, recipe field names as in the recipe... }
  }
  ```
- *Paste settings* (button, menu, shortcut): if the system clipboard holds valid
  `"lumen": "develop-settings"` JSON, that wins over Lumen's clipboard; the usual
  *choose what to paste* list opens with it. Unknown fields are ignored, out-of-range values
  clamped, `version` newer than known → a toast *These settings come from a newer Lumen*, applied
  where understood.
- Masks travel as: gradients and ellipses (normalised coordinates) unchanged; AI masks as their kind
  (*Sky*, *Subject*, *Background*, *Person n*) and re-detected on the target photo (skipped with a
  note when the AI masks switch is off or *Person n* doesn't exist); brush masks left out with a
  toast *Brush masks stay with their photo*.
- Parse/validate in a pure `src/lib/develop-clipboard.ts` with unit tests (round-trip, clamp,
  brush dropped, garbage rejected).

## §6 The Develop crash and recovery

Julian's report: halation on an ellipse mask, zoomed in, long session; the preview stopped updating,
then Lumen crashed and relaunched into a loading screen without the theme; Close did nothing. It
happened again after a restart.

What exists (STATE.md bullets *Stability (Round 15 S1)* and *Window and recovery (Round 15 S3)*):
`src-tauri/src/watchdog.rs` (stalls, frozen-page reload), `restart.rs` (`restart_lumen` saves
`window.resume` and relaunches), `src/routes/boot.tsx:35–45` (`take_resume` → straight back to the
saved route), `:90` (splash waits on settings, library, min time, resume, strip hydrate),
`window.rs:323` close handling (`EXIT_GRACE` 4 s), `commands/develop.rs` (`develop_preview` and
`develop_region` take a request id, "latest wins").

**6.1 Diagnose (verify, then fix).** Read Lumen's log files under `config_dir/JL-Studios/Lumen/`
(read-only) for both crashes. Reproduce on a **temp library** with a large photo: ellipse mask,
halation, zoom 100 %+, drag sliders for minutes. Watch memory and the render queue. Prime suspects,
in order: renders not cancelled (each slider tick starts a full area-tool render, `request` only
drops the result), the area tools' buffers at zoomed region size allocating per call, a panic in a
render thread taking the process down. Fix what the evidence shows; a running render whose request
is stale must stop at its next tile/row check, and at most one render per kind runs.

**6.2 Recovery opens Home.** Any relaunch after a crash (6.4 marker), a watchdog restart or
*Restart Lumen* from the health banner opens **Home**, never the saved route (`take_resume`'s route
is kept only for the alert's button), with an alert: *Lumen restarted after a problem. Your edits
are kept.* and a *Back to Develop* button (when the saved route was Develop) that goes to the saved
route. Develop drafts are already saved; the filmstrip is still remembered. A normal launch is
unchanged.

**6.3 Never stuck.** The splash has a ceiling (10 s): after it, the app shell renders with whatever
is ready and the theme from local settings (the theme must apply before or independently of the
library load). A failing step shows its error in the shell instead of holding the splash. Close
must end the process while the splash shows and while a render is running (check that the close
handler isn't blocked behind the same lock or the stuck render; the 4 s force-exit must always arm).

**6.4 Crash flag.** Lumen writes a `running` marker at start and removes it on clean exit; a marker
found at start means the last run crashed → 6.2's alert path. The marker holds the last route.

## §7 Plugin store

**Hosting (free).** A public GitHub repo `05Narwhal/lumen-plugins`:
- `index.json` (schema below) at the repo root, served from
  `https://raw.githubusercontent.com/05Narwhal/lumen-plugins/main/index.json`.
- Packages (`.lumenplugin`, signed as today) as GitHub Release assets of that repo.
- Submissions: pull requests adding an entry, reviewed and merged by Julian (`CONTRIBUTING.md` in
  that repo). A CI workflow validates `index.json` against the schema, downloads each new package,
  checks its blake3 and signature. Built locally in `plugins/store/` of this repo as the seed
  content (index, schema, validator script, workflow, README); Julian creates and pushes the repo.

**index.json** (version 1):
```json
{ "version": 1, "plugins": [ {
  "id": "jl.wayfinder", "name": "Wayfinder", "author": "JL Studios", "jl": true,
  "summary": "Find where photos without GPS were taken.", "kinds": ["tools"],
  "permissions": ["library.read", "places.write", "originals.gps"],
  "icon": "https://…/icon.png", "homepage": "https://…",
  "latest": { "version": "1.0.0", "api": "1.1.0", "url": "https://github.com/…/wayfinder-1.0.0.lumenplugin",
              "blake3": "…", "size": 123, "released": "2026-11-21" }
} ] }
```
Lumen still verifies the package signature on install; `blake3` is checked on download. `jl: true`
is shown as *By JL Studios* only if the package is signed by the JL key.

**Network.** The store is built in and may use the network whenever it needs to: fetch the index
when the page opens and in the background (at launch, then every 6 h while Lumen runs) to find
updates; download packages. No switch turns it off. Requests go through Rust (`reqwest`, already a
dependency), never the webview; the index is cached on disk so the page shows the last list
offline. Offline (or GitHub unreachable): the page shows a dedicated offline screen (one sentence,
*Try again*), with installed plugins still manageable.

**Updates.** Settings → Plugins gets *Update plugins automatically* (default **on**). When on, a
newer version found in the index is downloaded, verified and installed in the background (themes,
presets, prompts apply at once; tool and importer plugins take effect on their next load); a note
*Updated <name> to <version>* appears in Settings → Plugins and the store. When off, the store and
Settings → Plugins show an *Update* button and a count. An update signed by a different key than
the first install is refused (existing rule). A plugin installed from a file whose id is in the
index is updated from the store too.

**Page.** A **Plugins** page in the sidebar's bottom group, under Develop (route `/plugins`, a
`1`–`8` key only if one is free, a palette entry *Open plugin store*). Tabs: *Discover* (search,
kind filter, cards with icon, name, author, summary, *By JL Studios* badge, *Get* / *Installed* /
*Update*), *Installed* (same list as Settings → Plugins with update state; links to each plugin's
settings page), *Upload* (disabled: *Uploading from Lumen is coming later. For now, open a pull
request.* with a link to the repo). A card opens a detail sheet: description, permissions in plain
words (from the existing install sheet), version, size, homepage. *Get* runs the existing install
flow (install sheet, permissions) on the downloaded file. Empty state per tab.

## §8 Plugin platform 1.1

WIT `lumen:plugin@1.1.0`; 1.0.0 plugins keep working unchanged (the host serves both). The SDK
(`plugins/sdk/lumen-plugin`), `docs/PLUGINS.md` and `pack.mjs` validation learn every addition.
Each new permission gets a plain-words line on the install sheet.

**8.1 Providers (declarative, no wasm).**
```toml
[[providers]]
kind = "file-picker"          # or "reveal"
app = "disk-atlas"            # refers to an [[detect]] entry
# file-picker: how to call it; placeholders are filled by Lumen, never by a shell
args = ["--pick", "{mode}", "--title", "{title}", "--start", "{start}", "--name", "{name}", "{filters...}", "--parent", "{parent}"]
modes = { open-file = "open", open-multiple = "open-multiple", open-folder = "folder", save = "save" }
output = "json-stdout"        # one line {"paths":[…],"filter":n}; exit 0 picked, 1 cancelled, 2 bad arguments
# reveal:
# args = ["--select", "{path}"]   (or ["{folder}"] when the app has no select flag)
```
- Lumen's dialog kinds are `open-file`, `open-multiple`, `open-folder`, `save`; `modes` maps each
  to the app's word (a kind missing from `modes` is not provided: the system dialog opens).
  `{filters...}` expands to one `--filter "Label|*.a;*.b"` per filter; an argument pair whose
  placeholder has no value (`--name` outside save, `--start` unset) is dropped as a pair; `{parent}`
  is the main window's HWND on Windows (dropped elsewhere). Unknown placeholders are refused at
  install. Output: one JSON line `{"paths":["…"],"filter":n}`, exactly Disk Atlas's documented
  answer (`docs/file-picker.md` in its repo); exit 1 = cancelled (no fallback), exit 2 or anything
  else = failure (fallback). The program runs with stdout piped (GUI-subsystem exe on Windows) and
  Lumen waits without blocking the UI thread; the system dialog is not modal-blocking either.
- Lumen gets **one** dialog module, `src/lib/dialogs.ts` (`pickFile`, `pickFiles`, `pickFolder`,
  `pickSave`), backed by a Rust command `dialog_pick` that asks the active provider, else the
  system dialog. Every current `open(`/`save(` from `@tauri-apps/plugin-dialog` in `src/` (about 23
  sites, 17 files) moves to it. Reveal: `reveal_asset` and every `revealItemInDir` / `openPath`
  reveal site go through one Rust `reveal_path` that asks the active reveal provider, else the
  system. Any provider failure (missing exe, non-zero exit other than *cancelled*, bad JSON,
  timeout 10 min for pickers / spawn failure for reveal) falls back to the system dialog/reveal and
  logs why. The program is launched by the host with an argument vector (no shell); only an
  executable resolved by `[[detect]]` can be launched.
- At most one active provider per kind; if two plugins provide one, Settings → Plugins shows a
  choice. A plugin's providers are on only while its own settings toggles (8.3) allow it.

**8.2 App detection (declarative).**
```toml
[[detect]]
id = "disk-atlas"
name = "Disk Atlas"
windows = { exe = "disk-atlas.exe", paths = ["%LOCALAPPDATA%/Programs/Disk Atlas", "%ProgramFiles%/Disk Atlas"],
            path_env = true, registry = ["HKCU/Software/Microsoft/Windows/CurrentVersion/App Paths/disk-atlas.exe"] }
linux   = { exe = "disk-atlas", path_env = true }
macos   = { app = "Disk Atlas.app" }
version = ["--version"]       # optional; first line of stdout shown
```
The host resolves it at load and on demand; the result (found path, version, or not found) is
available to the plugin's settings page and to providers. A *Locate…* button stores a person-chosen
executable for that detect id (kept in the plugin's settings store). The chosen exe must exist and
match the declared `exe` file name.

**8.3 A settings page per plugin.** Any plugin with `[[settings]]` or `[[detect]]` gets its own page
at Settings → Plugins → *<plugin name>* (a row in the list opens it; back returns). Lumen draws:
header (icon, name, version, author, enable switch), one *status* row per detect entry (*Disk Atlas
found · 0.4.2 · C:\…* / *Not found* + *Locate…* + the plugin's `homepage` link), then the declared
settings controls (existing types bool/number/text/choice) with optional `group` headings and a
new optional `requires = "<detect id>"` that disables a control while that app is not found.
Providers read their on/off from a bool setting named in the provider (`enabled_by = "use_picker"`).

**8.4 Host-run models (`models` permission).**
```toml
[[models]]
id = "image"                       # plugin-local id
url = "https://…/wayfinder-image-fp16.onnx"
blake3 = "…"; size = 0; licence = "CC-BY-NC-4.0"
inputs = { pixels = { kind = "image", size = 224, mean = [0.481,0.458,0.408], std = [0.269,0.261,0.276], layout = "nchw" } }
```
The host downloads models after install consent (progress in the activity ring), checks blake3,
stores them under app data `plugins/<id>/models/`, and runs them through `photos-ml`'s ort setup
(same execution providers, DirectML/CPU fallback). WIT `models` interface: `run(model, inputs) ->
outputs` on f32 tensors, plus `run-on-asset(model, asset, input-name) -> outputs` where the host
decodes the photo (`decode_display_full`, so orientation is right), resizes and normalises per the
manifest, batching across calls where it can. Wasm never sees file bytes. Missing/failed model is a
state (the plugin's tools say *Model not downloaded*), never an error dialog. A manifest model may
instead be `kind = "vectors"` (a header + f16 matrix + f32 payload rows file): WIT
`models.nearest(source, query: list<f32>, k) -> list<(row, score, payload)>` runs the dot-product
top-k natively, where `source` is a manifest vectors model **or** a vectors file the plugin wrote to
its own storage (same format, e.g. embeddings of the person's own geotagged photos), so no plugin holds a big matrix in wasm. The plugin's 30 s limit is per call; a tool
run is many calls (batches of ≤ 32 assets), each reporting progress. Trait
`ModelHost` in `photos-plugins`, implemented in `src-tauri/src/plugins/` on top of `photos-ml`.

**8.5 Library read (`library.read`).** WIT `library`: keyset-paged `assets(filter, cursor) ->
page` with id, capture time (`captured_at` wall-clock and offset if known), file name, folder
(`assets.source_dir`), media kind, GPS (lat, lon, source: camera / estimated / none), place name,
burst/live group id, camera model. Filter: `has-gps`, time range, album id, *current selection*.
Read-only; 200 per page.

**8.6 Estimated places and the generic place review (`places.write`).** A plugin never writes a
place directly. WIT `places`: `suggest(batch)` sends suggestions: per asset up to 5 candidates
(lat, lon, confidence 0–1, label), a state (`anchor` / `inferred` / `guess`), a group id and group
label, and an optional `flag` reason (`far-from-group`, `impossible-speed`, `low-confidence`). The
host shows the **Place review** screen (route `/review/places?plugin=<id>`, opened by the plugin's
tool or a *Review* button on its settings page):

```
┌ Place review · Wayfinder ─────────────────────────────────────┐
│ [Check these 12]  [Moments 41]  [Done 870]        Write 870 ▸ │
│ ┌──── map (EOX / OSM as Settings) ────┐ ┌ Moment ────────────┐ │
│ │  ● anchors  ◐ inferred  ○ guesses    │ │ 12 Mar 14:02–14:40 │ │
│ │  drag a pin to move it               │ │ 23 photos · Ljubljana│
│ └──────────────────────────────────────┘ │ ▢▢▢ samples        │ │
│ timeline strip: moments in time order    │ [Confirm all ↵]    │ │
│                                          │ [Move pin] [Split] │ │
│                                          │ [Show all 23]      │ │
│                                          └────────────────────┘ │
└───────────────────────────────────────────────────────────────┘
```
- Keys: `Enter` confirm, `R` reject (photo or moment goes back to *no location*), `M` type a place
  (offline atlas search, Nominatim when *Exact place names* is on), `S` split, `←/→` previous/next.
- Every action is sent back to the plugin (`on-review(event)` export) and the plugin returns updated
  suggestions (8.9's propagation); the screen re-renders from them.
- **Write N locations** (and only that) commits: each confirmed/accepted asset gets GPS in the
  library DB with `place_source = 'estimated:<plugin id>'` (existing column, migration 0035), and —
  with `originals.gps` — in the original file (8.7). The write runs as a background job (activity
  ring, cancellable between files); one undo toast covers the whole write: DB rows always revert;
  files revert by removing the written GPS (verified as in 8.7) and a file that changed since the
  write (`file_hash` mismatch) or is unreachable is skipped and listed (*N files kept their
  location*). The map, info panel and search show estimated places with an *Estimated* marker.
- Generic: nothing in the screen knows Wayfinder; another plugin could feed it.

**8.7 GPS into originals (`originals.gps`, migration 0040).** Allowed by §11. Only for an asset
whose file has **no** GPS (re-checked from the file right before writing, not from the DB); never
replaces or edits existing GPS tags; no other tag changes. **Formats this round:** JPEG, HEIC/HEIF,
PNG, TIFF, DNG, NEF (incl. HE / HE\*). Videos, other RAWs (CR2/CR3, ARW, …) and anything else stay
DB-only, listed in the result (*Location saved in Lumen only*). Steps per file: copy the original
to a temp file in the same folder, write `GPSLatitude/Ref`, `GPSLongitude/Ref` (and `GPSAltitude`
only if given) with ExifTool (the bundled sidecar), plus the marker
`GPSProcessingMethod = "ESTIMATED:<plugin id>"` (EXIF; XMP `exif:GPSProcessingMethod` where EXIF
can't be written) so estimated places stay distinguishable from camera GPS; then
**verify**: ExifTool reads the temp file back, GPS equals what was written within 1e-6°, every other
tag equal to the original's, image still decodes (`decode_display_full` succeeds; NEF: the
embedded preview JPEG still extracts byte-identical and `photos-raw` still decodes it where it
could before); then atomically replace (rename over, `fsync`). Any failure → temp removed, original
untouched, the asset stays DB-only with a note in the result list. **No kept copy** (Julian's
answer); undo removes the GPS tags the same verified way.
Identity: `assets.content_hash` stays the import hash (derivatives, duplicates and import skip keep
working). Migration 0040 adds `assets.file_hash TEXT NULL` (blake3 of the current bytes when they
differ from `content_hash`) and `assets.metadata_written_at`. *Check the library* / Repair compare
the file to `file_hash` when set, so a GPS-written file is healthy, never "damaged". Re-importing
the camera's untouched file still matches `content_hash` and is skipped as a duplicate; *Check a
folder against the library* compares source files with `content_hash` (unchanged) and library
files with `file_hash` when set. Export, Check, duplicate and import-skip tests cover a written file.

**8.8 SDK and docs.** `lumen-plugin` gets typed wrappers for `models`, `library`, `places` and the
`on-review` export; `docs/PLUGINS.md` documents providers, detect, settings pages, models, library,
places, `originals.gps`; one tiny example per new declarative kind under `plugins/examples/`
(`provider-echo` launching a test exe in fixtures is enough for tests).

## §9 Disk Atlas plugin (`jl.disk-atlas`)

Declarative only (no `plugin.wasm`): `[[detect]]` for Disk Atlas, `[[providers]]` file-picker and
reveal, `[[settings]]`: *Use Disk Atlas for file dialogs* (bool, default on, `requires`),
*Use Disk Atlas for Show in folder* (bool, default on, `requires`). Lives in
`plugins/first-party/disk-atlas/`, signed with the JL key by `pack.mjs`, listed in the store seed
index. Disk Atlas repo (`X:/Documents/Coding/Projects/Desktop-Apps/Cargo/disk-atlas`, its own
conventions and CLAUDE.md): it already has `--pick <mode>` (JSON on stdout, exit 0/1/2,
`docs/file-picker.md`) and opens a path argument in the running window. If it cannot open a folder
with one file selected, add `--select <file>` there (own branch and commit in that repo, its
tests). The plugin's reveal args use `--select {path}` once that exists.

## §10 Wayfinder (`jl.wayfinder`)

A tools plugin (wasm) using `models`, `library.read`, `places.write`, `originals.gps`. Credits
FindMyPhoto (idea) and GeoCLIP (model) in its README.

**10.1 Model — Lumen Wayfinder 1.0.** Built on **GeoCLIP** (Vivanco et al., NeurIPS 2023, MIT code and
weights, trained on MP-16), not from scratch (Julian's answer). GeoCLIP is retrieval-shaped (image
embedding vs. a gallery of location embeddings), which is what makes Wayfinder's improvements
cheap. **Challenger:** PLONK (*Around the World in 80 Timesteps*, Dufour et al., CVPR 2025;
`nicolas-dufour/PLONK_YFCC` on Hugging Face, YFCC being closest to personal photos) is measured on
the same accuracy check; it is adopted instead only if it beats GeoCLIP + Wayfinder's improvements
on median error **and** stays ≥ 5 photos/s (its flow sampling takes many steps). Large
vision-language models are out (too heavy to run locally at speed). Exported in WSL by
`scripts/ml/export_wayfinder.py` into: `wayfinder-image-fp16.onnx` (CLIP ViT-L/14 + GeoCLIP image
head → 512-d normalised embedding), `wayfinder-location.onnx` (GeoCLIP's location encoder, small;
lets the plugin add gallery points) and `wayfinder-gallery.bin` (GeoCLIP's 100k-coordinate gallery
encoded once at export time: 100k × 512 f16 + f32 lat/lon, with a header), a `kind = "vectors"` model whose top-k runs in the host (`models.nearest`). If the
export fails, DirectML can't run it (CPU must still work), or the raw model's median error on the
accuracy check is worse than 750 km, the lane stops after the model block and Julian decides. Speed target: ≥ 10 photos/s on Julian's GPU
including decode (FindMyPhoto's PyTorch CPU path is the baseline to beat; measure both). Accuracy
check on a **copy** of Julian's library: photos with camera GPS, location hidden, median error and
share within 1 / 25 / 200 / 750 km for raw model vs Wayfinder with library signals. Listed under
*Non-commercial weights* in `docs/THIRD-PARTY-MODELS.md` (licence string `CC-BY-NC-4.0`), uploaded
by Julian to the store repo's release (or `models-v1`).

**10.2 Improvements over GeoCLIP (plugin logic, Rust in wasm).**
- **Personal retrieval**: the person's own camera-GPS photos are embedded with the same image
  encoder and stored as a vectors file in plugin storage; a photo without GPS whose nearest own
  photo scores above a similarity threshold (start 0.85, tuned in W2) takes that photo's place as
  its top candidate (people revisit places; far finer than any global gallery).
- **Denser gallery where you go**: extra gallery points (≈ 1 km grid within 25 km of each of the
  person's GPS clusters) encoded with `wayfinder-location.onnx` and added to the search, so guesses
  near home and favourite places resolve to streets, not cities.
- **Test-time crops**: the image embedding is the mean of the full frame and two centre crops
  (on only if W2 shows it helps).

**Signals.**
- **Anchors**: camera GPS and person-confirmed places. Never changed.
- **Time interpolation**: a photo between two anchors within 2 h is placed by time-weighted
  interpolation, `inferred`, if the implied speed between anchors ≤ 250 km/h (else no inference);
  one anchor within 15 min → that anchor's place, `inferred`. Bursts and Live pairs share a place.
- **Library prior**: GeoCLIP's top-k re-ranked by distance to the person's own GPS clusters (places
  they have been), weight tuned on the accuracy check; never overrides an anchor.
- **Folder and file names**: a folder like `Paris 2024` or `2024-07 Bled` looked up in the offline
  atlas (host place lookup exposed through `places.lookup(name)`), used as a prior for the moment.
- **Moments**: photos without GPS grouped by time gaps (≤ 30 min between consecutive photos, split
  where guesses jump > 50 km); each moment gets one place (median of member guesses, weighted by
  confidence) unless members are anchored/inferred differently.
- **Flags**: a member guess > 25 km from its moment's place (`far-from-group`), implied speed to a
  neighbour > 250 km/h (`impossible-speed`), confidence < 0.2 (`low-confidence`) → *Check these*.
- **Propagation on review**: confirming a photo or moment makes it an anchor; its time neighbours
  are re-inferred at once; rejecting clears; moving a pin re-anchors. Each `on-review` returns only
  changed suggestions.
- Thresholds above are starting values; the accuracy check may tune them (record final values).

**10.3 Flow.** Tool *Find locations…* (palette, Library selection menu, Wayfinder settings page):
scope = selection / album / whole library, only assets with no GPS. Progress in the activity ring;
embeddings cached per asset (plugin storage, keyed by `content_hash`) so a re-run is instant. When
done, the Place review opens. Nothing is written until *Write N locations*.

## §11 The originals rule

New wording (CLAUDE.md non-negotiable 11, architecture spec §10 table row and §12 first line):

> **Originals are kept intact.** Import copies; edits are recipes; deletion goes to a 30-day trash.
> Two exceptions, each only on the person's explicit action and each verified before it replaces
> the file: (a) **metadata writes** that add information the file lacks (e.g. GPS on a photo with
> none), never changing what the camera recorded; (b) **lossless edits** (video trim by stream copy,
> lossless JPEG crop). Nothing that re-encodes ever replaces an original.

The CLAUDE.md / CLAUDE.local.md rules about **Claude sessions** never writing to Julian's library
are unchanged: features that write originals are tested on temp libraries only.

---

## Julian's answers (decided)

- Layering: dropped. Rename inline: masks only. Menu: *Arrange* submenu + tighter rows.
- Clipboard: readable JSON; *Paste* applies JSON from the clipboard; masks: shapes yes, AI
  re-detected, brush left out.
- Crash: Home with an alert.
- Store: GitHub repo + Releases; anyone listed after Julian's PR review; upload disabled; its own
  sidebar page under Develop; network whenever needed, no off switch, offline screen; auto-update
  on by default with a switch to turn it off.
- Disk Atlas: all dialogs; reveal selects the file (add `--select` to Disk Atlas if needed);
  detection by known spots + *Locate…*.
- Geolocation plugin: approach 1; name **Wayfinder**; GeoCLIP as the base, improved (PLONK measured
  as a challenger); NEF included in original GPS writes; only adds GPS where none exists; writes to
  originals allowed (rule updated); verify only, no kept copy; lossless-only for trim/crop;
  the moments review design as in §8.6 / §10.2; licences: not a concern (NC listing).

## Calls made in writing (Julian may overrule)

- Store trust is today's rule: a package's key is pinned to its plugin id at first install;
  third-party signed packages install from the store like from a file; unsigned never.
- Recovery to Home covers crash, watchdog restart and the banner's *Restart Lumen*.
- Original GPS writes this round: JPEG, HEIC/HEIF, PNG, TIFF, DNG, NEF; others DB-only.
- Wayfinder's gallery top-k runs in the host (`models.nearest`); the lane stops after the model
  block if the model fails the bar in §10.1.
- Disk Atlas plugin and Wayfinder are signed with the JL key and seeded in the store index.
- Plugin API 1.1 is served alongside 1.0 (how: D1's call).

## Left to the builder

Exact component split inside each lane, token names, test file names, the provider output adapter
shape, the area-tool fix once diagnosed, Wayfinder threshold tuning (recorded in the plan's hand-off).
