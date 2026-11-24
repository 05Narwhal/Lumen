# Writing Lumen plugins

Plugin API 1 (Lumen 26.2.0 and later). A plugin is a `.lumenplugin` file: a zip with a
`plugin.toml` manifest and the files the manifest names. Most plugins are data (presets,
an app list, a prompt) and run no code. An importer or an AI tool is a WebAssembly component that
runs in Lumen's sandbox. Everything on this page is checked by Lumen when a plugin is installed;
the first-party plugins in [`plugins/first-party/`](../plugins/first-party) (Disk Atlas, Wayfinder) are real ones to read.

The authority for every rule is the code: `photos-plugins` (`manifest.rs`, `package.rs`,
`theme.rs`, `licences.rs`) and the design in
`docs/superpowers/specs/2026-11-13-round-23-design.md`. If this page and Lumen disagree, Lumen is right.

## Kinds

| Kind | What it adds | Runs code | Declared by |
| --- | --- | --- | --- |
| Presets | Develop presets, as Lumen JSON or Lightroom `.xmp`; a preset may also describe a *look* for AI assistants | no | `[presets]` |
| Apps | Apps in *Edit in* and *Open with*, found by detection | no | `[apps]` |
| Prompts | Prompts AI assistants can use | no | `[[prompts]]` |
| Importer | An import source: a card on the Import page | yes (`plugin.wasm`) | `[importer]` |
| Tools | Tools AI assistants can call | yes (`plugin.wasm`) | `[[tools]]` |

One plugin can hold several kinds. A package with an importer or tools needs `plugin.wasm`, and a
package with `plugin.wasm` needs one of those two kinds.

## The package

```
plugin.toml            the manifest (required)
icon.png               at most 256 x 256 (optional)
plugin.wasm            only for an importer or tools
presets/*.json and *.xmp          apps.toml
prompts/*.md           assets/**                         LICENSE, NOTICES (optional)
SIGNATURE              made by pack.mjs
```

Limits: 50 MB unpacked, 2,000 files, no archives inside the package, no links. Every path is
relative and `/`-separated, with no `..`, no drive letters, no `\` or `:`, no names ending in `.` or
a space, and no Windows device names. Names that differ only in case are refused.

## plugin.toml

Unknown keys anywhere in the manifest are an error. Every key:

```toml
id = "studio.example.takeout"   # 3-64 characters of a-z, 0-9, "." and "-"; unique
name = "Google Takeout"
version = "1.0.0"               # semver
author = "Example Studio"
description = "One sentence."
license = "MIT"                 # an SPDX expression, see Licences
homepage = "https://example.com" # optional; shown as text, never fetched
lumen = ">=26.2.0"              # a semver range of Lumen versions
api = 1                         # the plugin API; a higher one needs a newer Lumen
permissions = ["import.read-chosen"]   # see Permissions

[presets]
dir = "presets"                 # a folder of .json and .xmp files

[apps]
file = "apps.toml"

[[prompts]]
name = "film-look"              # 1-40 of a-z, 0-9, "_" and "-"
description = "Retouch toward a warm film look"
file = "prompts/film-look.md"   # UTF-8 text, at most 32 KB

[importer]
label = "Google Takeout"        # the card's name: From <label>
pick = "folder"                 # folder | file
extensions = ["zip"]            # for pick = "file"
hint = "Choose the folder Google Takeout made"

[[tools]]                       # one table per tool
name = "level_horizon"          # 1-40 of a-z, 0-9 and "_"
description = "What it does, for the assistant"

[[assets]]                      # every bundled file that carries its own licence
path = "assets/fonts/Lora.woff2"
license = "OFL-1.1"

[[settings]]                    # Lumen draws the control; the plugin reads it with host.setting
key = "albums_from_folders"     # (API 1.2 has richer pages: see "Settings pages")
label = "Use folders as albums"
type = "bool"                   # bool | number | text | choice
default = true                  # must match the type
# choices = ["a", "b"]          # for type = "choice"; the default must be one of them
```

Required: `id`, `name`, `version`, `author`, `description`, `license`, `lumen`, `api`. Everything
else is optional. Every file the manifest names must be in the package.

AI assistants see a tool as `<short>__<name>` and a prompt the same way, where `<short>` is the last
segment of the id (`studio.example.takeout` gives `takeout`). Prompts and tools are offered only
while *AI assistants* are on in Settings.

## Permissions

API 1 has four. The person grants **all or none** when installing; Lumen enforces them. A plugin
that asks for none runs with only `host.log` and `host.setting` (its own settings).

| Permission | The install sheet says it can | What the plugin gets |
| --- | --- | --- |
| `import.read-chosen` | Read the folder or file you choose when you import with it | `chosen`: list, stat and read below the chosen folder; progress |
| `photos.read` | See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place | `photos`: the filmstrip's ids, metadata, a preview, the recipe |
| `develop.write-drafts` | Change Develop drafts of those photos (every change can be undone) | `drafts`: a merge-patch, one undo step labelled `AI: <plugin>: <label>` |
| `storage` | Keep its own files (up to 50 MB) | `storage`: read, write, remove and list named files |

`photos.read` and `develop.write-drafts` work only while AI assistants are on, only on the photos
in Develop's filmstrip, and only write drafts; the library keeps showing the saved edit until the
person applies the draft.

**The install sheet** shows the plugin's icon, name, version and trust line, then:

- **Adds**: one line per thing it adds (*3 Develop presets*, *An app: "GIMP"*,
  *A prompt for AI assistants: "film-look"*, *An importer: "Folder with notes"*, *1 tool for AI assistants*);
- **It can**: the sentences above for each permission, or *It runs no code.* for a data plugin;
- **It can never**: touch your originals or your library, use the internet, or start other programs;
- **Licence**, and for an update *Update 1.0.0 -> 1.1.0* with new permissions marked *New*.

A *problem* replaces **Install** with the reason: an unsupported API, a missing or malformed licence, a `plugin.wasm` import the permissions don't cover, a preset that
doesn't read. Notes (for example Lightroom settings that weren't converted) are shown and don't block.

## Licences

`license` and each `[[assets]]` licence are SPDX expressions (`AND`, `OR`, parentheses and `WITH`).
Lumen only checks that you declared one and that it is well-formed; it doesn't decide which licence
you may use. A `LicenseRef-<name>` needs its text in a `LICENSE` file in the package. The install
sheet shows the licence as written.

A plugin list or registry may set a stricter rule (the planned community registry lists only
OSI-approved licences and no no-derivatives ones). That rule belongs to the registry, not to the app,
so a plugin you install from a file always installs.

Put the licence text of anything you bundle next to it (for a font, its OFL
text, for instance in `assets/fonts/OFL.txt`).

## Themes

Plugins can't carry themes (API 1.4): a `[[themes]]` table is refused. A theme is its own, lighter
package, a `.lumentheme`, with no code, no permissions and no signature; it may also move things
around with a checked `theme.css`. See [THEMES.md](THEMES.md).

## Presets, apps and prompts

- **Presets.** `presets/*.json` is Lumen's preset shape: `{ "recipeVersion": 3, "name", "develop", "panels" }`,
  where `develop` is a partial Develop recipe (`basic`, `curve`, `hsl`, `grading`, `detail`, `effects`,
  and masks) and `panels` lists which panels the preset applies (`basic`, `curve`, `hsl`, `grading`,
  `detail`, `effects`, `masks`). `recipeVersion` must be one this Lumen reads (up to 3). Unknown keys are refused.
  Lightroom `.xmp` presets are converted when read, never written into the package; the sheet lists what
  wasn't converted. A JSON preset may add
  `"look": { "intent", "hues", "mono", "key", "skinSafe", "likesSky" }` to join the looks AI assistants
  choose from (`intent` is one line of up to 300 characters, up to four `hues` from 0 to 359, `key` is
  `low`, `mid` or `high`). Plugin presets show read-only in their own group; *Duplicate to my presets* makes a copy.
- **Apps.** `apps.toml` has one `[[app]]` per app with `id`, `name`, `kind` (`editor`, `developer`,
  `video`), `accepts` (`photo`, `raw`, `video`), `reads_raw`, `writes` (`inPlace`, `sidecar`, `never`), and
  detection only: `[app.windows]` (`app_paths`, `uninstall`, `globs`), `[app.mac]` (`bundle_ids`),
  `[app.linux]` (`desktop`). There are no argument templates: Lumen passes the file as the only
  argument, and always gives a plugin app a **copy**. An app that isn't installed isn't shown. Shells and script hosts
  (`cmd`, `powershell`, `sh`, `python*`, `node`, `osascript`, Terminal and the like) are refused, and Windows globs must start in
  `%ProgramFiles%`, `%ProgramFiles(x86)%` or `%LocalAppData%/Programs`.
- **Prompts.** Plain UTF-8 Markdown, at most 32 KB, with a name and a one-line description in the manifest.

## Code plugins: importer and tools

The WIT package is `lumen:plugin@1.4.0` (`src-tauri/crates/photos-plugins/wit/lumen-plugin.wit`; a 1.0
component still runs, 1.x only ever adds), and its first worlds are `importer-plugin`, `tools-plugin` and
`full-plugin` (both); 1.1 to 1.3 add `places-plugin`, `ui-plugin` and `proposals-plugin`. A component imports only the
interfaces its permissions allow, and an interface the code never calls isn't imported at all. The
easiest way to write one in Rust is [`lumen-plugin`](../plugins/sdk/lumen-plugin) (MIT):

```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
lumen-plugin = { path = "../sdk/lumen-plugin", features = ["importer"] }   # and/or "tools"
```

Implement `Importer` or `Tools` and export it with `export_importer!`, `export_tools!`, or
`export_full!(I, T)` when one component does both. Build with
`cargo build --release --target wasm32-wasip2` (`rustup target add wasm32-wasip2`) and copy the result next
to `plugin.toml` as `plugin.wasm`. Build one plugin per `cargo` invocation (`-p <name>`): in a
workspace build, cargo unifies the SDK's features and selects the `full-plugin` world.

The sandbox: no network, no sockets, no files except through `chosen` (below the folder the person
chose) and `storage` (its own 50 MB). Calls have time limits (an importer is stopped when `progress` reports
that Lumen wants it to stop, a tool call ends after 30 s), and memory is capped.

## Models

A plugin with the `models` permission declares the files it needs and Lumen runs them for it. Wasm never
holds the weights or a photo's bytes.

```toml
[[models]]
id = "image"
url = "https://…/image.onnx"            # https only
blake3 = "<64 hex digits>"              # size and blake3 are checked; a wrong file is discarded
size = 610526312
licence = "CC-BY-NC-4.0"                # shown on the install sheet
inputs = { pixel_values = { kind = "image", size = 224, mean = [0.48, 0.46, 0.41], std = [0.27, 0.26, 0.28], layout = "nchw" } }
```

`kind = "onnx"` (the default) is a model with f32 inputs and outputs; the key of an `inputs` entry is the
ONNX input's name. `kind = "image"` inputs are filled by Lumen (`run-on-asset`: the photo decoded upright
with its edits, short side to `size`, centre crop, `(v/255 − mean)/std`); `kind = "tensor"` inputs are
passed by the plugin (`run`). `kind = "vectors"` is a file of vectors (below). Lumen downloads the files
after the person agrees on the install sheet, into the plugin's own models folder, with progress in the
activity ring; a model that isn't there yet answers `model_missing: …` and the plugin should say so.
Each call counts once against the 30 s limit and takes one photo: ask for at most 32 per step.

`nearest(source, query, k)` ranks a vectors file by dot product and returns the best `k` rows with their
payload, so a plugin never holds a big matrix. `source` is `model(id)` (a `[[models]]` vectors file) or
`storage(name)` (a file the plugin wrote to its own storage). The file is little-endian: `LWVG`, u32
version 1, u32 rows, u32 dim, u32 dtype 1 (f16), `rows × dim` f16, then a payload of f32 per row (the
width is what is left of the file divided by `rows × 4`).

## Library

With `library.read` a plugin reads the person's library, never their bytes: `library.assets(filter,
cursor)` returns 200 assets per page (id, wall-clock capture time, folder it was imported from, file name,
kind, GPS with its source `camera` or `estimated`, place name, burst or Live group, camera). `filter` narrows
by `has-gps`, `since` / `until`, `album` and `selection` (the person's current selection, which is Develop's
filmstrip). Trashed assets and the movie half of a Live Photo are left out. There is no content hash:
asset ids are stable inside a library, so key a cache on them.

**API 1.4** adds `library.details(ids)` (up to 200 ids, from a page of `assets`): per asset the
`capture-source` of its date (`original` is the shutter's own clock; `file-time` means the file's date,
so no camera clock at all), `title`, `caption`, `keywords`, the `scene` words Lumen saw (*beach*,
*group of people*), the `people` named and listed on it, its `albums`, and its `text` (the lines Lumen
read, at most 12 of 120 characters, only those it was fairly sure of). Everything is read-only and from
Lumen's database; an asset that is gone or trashed is left out. It needs the same `library.read`, which
the install sheet now words as *titles, captions, albums, the names of people in them and the text
Lumen read in them*. It is an addition to the `library` interface: older components that never call
it keep running (the host's `lumen:plugin@1.4.0` answers their `@1.x` imports by semver).

## Places and review

A plugin never writes a place. With `places.write` it sends *suggestions* with `places.suggest` (per
asset up to five candidates with a confidence, a state `anchor` / `inferred` / `guess`, a group id and
label, and a flag `far-from-group` / `impossible-speed` / `low-confidence`). `places.lookup(name)` finds
a place name in Lumen's offline atlas. Lumen's **Place review** screen (`/review/places?plugin=<id>`)
shows the suggestions on a map; every thing the person does is sent back through the plugin's
`on-review(event)` export (`confirm`, `reject`, `move`, `split`), which answers with only the suggestions
that changed. A plugin instance may be dropped between calls, so keep the review's state in storage.
Only *Write N locations* stores anything: the place goes into the library with the *Estimated* marker, one
undo takes the whole write back.

In the Rust SDK this is the `places` feature (world `places-plugin`, used alone): implement `Tools` and
`Review` and call `export_places!(Tools, Review)`. `plugins/first-party/wayfinder/` is a complete example.
A plugin has 50 MB of storage in all; the Wayfinder plugin budgets for it.

## Proposals (API 1.3)

A plugin can propose more than places. `proposals.submit(batch)` sends, per photo, one **kind** of
value with a confidence (0–1) and an optional reason (up to 200 characters, shown on the review):

| Kind | Value | Needs | Accepting it |
| --- | --- | --- | --- |
| `location` | latitude, longitude | `places.write` | fills a photo that has **no** place, marked *Estimated*, named from Lumen's atlas; a photo that has one is skipped |
| `date` | wall-clock `YYYY-MM-DDTHH:MM:SS` | `dates.write` | sets the capture time (the photo's time zone is kept) |
| `tags` | up to 20 tags | `tags.write` | adds them to the photo's keywords, keeping the ones it has |
| `label` | `Red`, `Yellow`, `Green`, `Blue` or `Purple` | `tags.write` | sets the colour label |
| `person` | a face id Lumen detected on that photo, and a name | `people.write` | moves that face to the person of that name (any case), or to a new person; nobody is renamed |
| `title`, `caption` | text (200 / 2000 characters) | `captions.write` | sets it |

At most 200 proposals per call (the Rust SDK's `proposals::submit_all` splits a longer list); one
bad or ungranted proposal refuses the whole call. One proposal per plugin, photo and kind: sending
again replaces it, except that a proposal the person **rejected** stays rejected while its value is
the same. A photo or face that doesn't exist is skipped, never an error. The permissions' install-sheet
words are *Suggest dates / tags and labels / titles and captions / names for faces Lumen found*.

Nothing is written until the person accepts. Lumen's review page (`/review/location`, `/review/date`,
`/review/tags`, `/review/label`, `/review/person`, `/review/title`, `/review/caption`, with
`?plugin=<id>`) shows each photo with what it has now beside what you propose, and the person can
*Accept*, *Edit* (the value, then accept), *Reject*, or *Accept all* above a confidence they choose.
Every action has a 6 s undo, and *Accept* touches Lumen's own records only: **no original file is
opened or changed**. Link to it from a contribution (`action = "open-review:date"`) or answer a tool
with `review: "/review/date"`.

In the Rust SDK this is the `proposals` feature (world `proposals-plugin`, used alone, with `tools`
and `library`): implement `Tools`, call `export_proposals!(Tools)`, and build batches with
`proposals::{propose, location, person, date_string}`. `plugins/fixtures/proposals_ok/` is a small
example. Wayfinder keeps its richer Place review (`/review/places`: moments, anchors, move and
split) through `places.suggest`.

Two first-party plugins are worked examples of it (`plugins/first-party/`, MIT, both with a pure `logic.rs`
that `cargo test` covers and a wasm-only `lib.rs`): **Clockwork** (`dates.write`) reads the dates and cameras
with `library.assets` and `library.details` (to skip a date that came from the file), looks for the one
clock shift that lines a camera up with a trusted one, and proposes `date`; **Captioner** (`captions.write`,
`storage`) builds `title` and `caption` from `library.details`' people, scene words and text with the
place and date, in several calls when the library is big (it keeps its cursor in `storage`).

## GPS in originals

The `originals.gps` permission lets *Write N locations* put the GPS into the **original file** as well as
the library. Lumen does the writing, never the plugin: only a file that has no GPS (checked from the
file right before writing, EXIF and XMP), never an existing GPS tag, no other tag changed, only JPEG,
HEIC / HEIF, PNG, TIFF, DNG and NEF. It writes a temp copy beside the file, adds `GPSLatitude`,
`GPSLongitude` and their refs with `GPSProcessingMethod = ESTIMATED:<plugin id>` so an estimate stays
distinguishable from camera GPS, checks the copy (every other tag equal, the image still decodes, a NEF's
preview unchanged) and only then replaces the original; any failure leaves it untouched and the place stays
in the library only. Videos and other RAW formats are library-only. There is no kept copy: undo removes the
same tags the same verified way, and a file that changed since is skipped and listed. The install sheet says
"Add GPS to original files that have none".

## Providers

API 1.1 (`api = "1.1.0"`) lets a plugin with no code replace Lumen's file dialogs and *Show in folder*
with another program. A `[[providers]]` table names a program found by a `[[detect]]` entry (below):

```toml
[[providers]]
kind = "file-picker"          # or "reveal"
app = "disk-atlas"            # a [[detect]] id
args = ["--pick", "{mode}", "--title", "{title}", "--start", "{start}", "--name", "{name}", "{filters...}", "--parent", "{parent}"]
modes = { open-file = "open", open-multiple = "open-multiple", open-folder = "folder", save = "save" }
output = "json-stdout"
enabled_by = "use_picker"     # a bool setting of this plugin; off = the system dialog
```

- Lumen's dialog kinds are `open-file`, `open-multiple`, `open-folder` and `save`; `modes` maps each to the
  program's own word. A kind missing from `modes` isn't provided: the system dialog opens.
- Placeholders: `{mode} {title} {start} {name} {filters...} {parent} {path} {folder}`. `{filters...}` becomes
  one `--filter "Label|*.a;*.b"` per filter; an argument pair whose placeholder has no value (`--name`
  outside *save*, `--start` unset) is dropped, and `{parent}` (the main window's handle) exists only on
  Windows. Any other `{...}` is refused when the plugin is installed. The program is started with an
  argument list, never through a shell.
- A picker answers with one line of JSON on standard output, `{"paths":["..."],"filter":0}`, and exit
  code `0` (picked), `1` (cancelled: Lumen shows nothing more) or `2` (bad arguments). Anything else (a
  missing program, another exit code, bad JSON, no answer in 10 minutes) is logged and Lumen opens the
  system dialog instead.
- A `reveal` provider is started with its `args` (for example `["--select", "{path}"]`, or `["{folder}"]`
  for a program that can't select) and fails over to the system the same way. When two plugins provide
  one kind, Settings → Plugins asks which one answers.
- Only a program a `[[detect]]` entry resolved can be started. A provider asks for no permission; the
  install sheet says *Open <app> for file dialogs / Show in folder*.

`plugins/first-party/disk-atlas/` is a real one.

## Detecting an app

```toml
[[detect]]
id = "disk-atlas"
name = "Disk Atlas"
windows = { exe = "disk-atlas.exe", paths = ["%LOCALAPPDATA%/Programs/Disk Atlas", "%ProgramFiles%/Disk Atlas"],
            path_env = true, registry = ["HKCU/Software/Microsoft/Windows/CurrentVersion/App Paths/disk-atlas.exe"] }
linux   = { exe = "disk-atlas", path_env = true }
macos   = { app = "Disk Atlas.app" }
version = ["--version"]       # optional: the first line of its output is shown
```

Lumen looks, in order, at the executable the person chose with *Locate…*, the registry keys, the `paths`
folders (environment variables expanded) and the `PATH`. `exe` is a file name, never a path. A chosen
executable must exist and have that name. The result (found path, version, or *Not found*) is shown on the
plugin's page and used by providers. `detect` needs `api = "1.1.0"`.

## Settings pages

A plugin with `[[settings]]`, `[[ui.section]]` or `[[detect]]` gets its own page: Settings → Plugins → the
plugin's name (`?section=plugins&plugin=<id>`). It shows the icon, name, version, author and the enable
switch; one status row per detect entry (*Disk Atlas found · 0.4.2 · C:/...* or *Not found*, with
*Locate…* and the plugin's `homepage`); then your sections; then the details (permissions, licence,
log) and *Turn off* / *Remove*. Every control's label and help is searched by Settings' search box and
jumps to the page.

### `[[ui.section]]` (API 1.2)

`[ui]` needs `api = "1.2.0"` (and `lumen` at the first release that speaks it). 1.0 and 1.1 manifests
keep parsing; a flat `[[settings]]` list is read as sections too (one per `group`, untitled without one).

```toml
[[ui.section]]
title = "Scanning"                 # optional
description = "How a scan runs."   # optional

[[ui.section.control]]
type = "toggle"                    # the kind, one of the ten below
key = "deep"                       # where the value is kept; the plugin reads it with host.setting
label = "Deep scan"
help = "Reads every file."         # optional second line
default = false                    # required for value controls; must fit the kind
# requires = "other_key"           # show only while another value control is on / non-empty
# app = "disk-atlas"               # disable while that [[detect]] app isn't found
```

| `type` | Needs | Value stored |
| --- | --- | --- |
| `toggle` | `key`, `default` | `true` / `false` |
| `select`, `segmented` | `key`, `choices`, `default` (one of them) | the chosen string |
| `slider` | `key`, `min` < `max`, `default`, optional `step` | a number inside the bounds |
| `number` | `key`, `default`, optional `min`, `max`, `step` | a number |
| `text` | `key`, `default` | a string |
| `button` | `action` | none; calls the plugin's `ui.action(action)` |
| `markdown` | `body` | none; static text |
| `stat` | `key` | none; the plugin's `ui.value(key)`, plain text |
| `list` | `key` | none; `ui.value(key)`, a JSON array of strings or of `{"text", "detail"}` |

Values live in `plugins.json` under the plugin's `settings`; a stored value that doesn't fit its control
is refused. `button`, `stat` and `list` need code (a `[[tools]]` or `[importer]`) that exports `ui`, and run
under the usual 30 s / 512 MB limits; a plugin that is off or stopped shows an empty line instead of
being called. A plugin with no code (providers) gets `toggle`, `select`, `segmented`, `slider`,
`number`, `text` and `markdown`. `requires` is a control key; the old detect-id meaning of a
`[[settings]]` table's `requires` is `app` on a control.

In Rust, the SDK's `ui` feature (alone, with `export_ui!(Tools, Ui)`) binds the `ui-plugin` world; `Ui`
has `action(name)` and `value(key)`. The first-party Disk Atlas and Wayfinder use only toggles, so
their manifests are the smallest examples.

## Contribution slots

A plugin can add an entry to a few places in Lumen (API 1.2). It describes the entry; Lumen draws it
with its own components, on theme, and only while the plugin is on and ready (off, stopped, broken or
too new adds nothing, and removing the plugin removes the entry).

```toml
[[ui.contribute]]
slot   = "map.toolbar"            # map.toolbar | library.selection-menu | viewer.menu | palette
label  = "Find places"            # 1-60 characters; the same label twice in one slot is refused
icon   = "map-pin"                # optional, a Tabler name from the list below
action = "tool:find_places"       # or open-review:<id>, or an action name your wasm answers
```

- `map.toolbar` is a button beside *Fit library*; `library.selection-menu` an item in the menu of a
  photo or a selection; `viewer.menu` an item in the viewer's photo menu; `palette` only the command
  palette. **Every** contribution, whatever its slot, also gets a palette entry (grouped under the
  plugin's name), once per plugin and label.
- `tool:<name>` runs one of your `[[tools]]` (the name must be declared) with its **scope** as the
  arguments: `{"scope":"library"}` from `map.toolbar` and `palette`, `{"scope":"selection"}` from
  `library.selection-menu` and `viewer.menu` (with `"assets":[<ids>]`, the photos the menu was opened
  on), `{"scope":"album","album":<id>}` from a page that knows its album. Read the photos with
  `library.query` (a `selection` filter reads those `assets` for that call; with none it reads the
  selection the app reports, Develop's filmstrip). The tool may be resumable: answer `{"status":"working"}` and Lumen calls again
  until `{"status":"done", "suggested": n, "review": "/review/<id>", "message": "…"}`. With
  `suggested > 0` and a `review`, Lumen offers *Review* (always your own review page, whatever
  `plugin=` you write); otherwise it shows `message`. Before the first call Lumen asks to download
  your `[[models]]` if any are missing. No `ui` export needed.
- `open-review:<id>` opens that review page (`/review/<id>?plugin=<plugin id>`); `places` is the
  Place review today. Any other action is passed to the plugin's `ui.action(name)` (so it needs
  `[[tools]]` or an `[importer]` and a `ui` export, like a settings button); its answer, a line of
  text, is shown as a toast.
- The plugin's page in Settings shows each contribution as a button too.
- Icons: `map-pin`, `map-search`, `search`, `sparkles`, `wand`, `photo`, `calendar`, `tag`, `user`,
  `world`, `download`, `refresh`, `settings`, `list-check`, `check`, `folder`, `scan`, `bolt`. An
  icon off the list is refused; a plugin's own image is not supported yet.

## Developer mode

Settings -> Plugins -> *Developer mode* (off by default) lets you work on a plugin without packing it:

- *Load a folder...* loads an unpacked plugin in place (listed with a *Dev* badge), with the same checks, install
  sheet and sandbox as an installed plugin. *Reload* re-reads it and restarts its instance.
- Unsigned packages install.
- *Log* shows the last 200 lines of `host.log` and the plugin's output.
- Turning it off unloads dev folders and unsigned plugins without deleting them.

To keep a throwaway app data folder while testing, start Lumen with the environment variable
`LUMEN_APP_DATA_DIR=<folder>`; plugins live in `<folder>/plugins/`.

## pack.mjs

`scripts/plugins/pack.mjs` (Node, no build step) makes, signs and checks packages:

```sh
node scripts/plugins/pack.mjs pack <folder> [--key <file>] [--out <file>]
node scripts/plugins/pack.mjs keygen <name> [--dir <folder>]
node scripts/plugins/pack.mjs digest <folder|file> --out <file>
node scripts/plugins/pack.mjs verify <file> --pub <file>
```

- `pack` writes `<folder-name>.lumenplugin` (or `--out`), the same bytes every time, applying the package
  rules above and checking the manifest's licences (a `LicenseRef-*` needs a `LICENSE`). With `--key` it
  signs; without, it says the package is unsigned. `target/` and `.git/` folders are left out. When
  [`wasm-tools`](https://github.com/bytecodealliance/wasm-tools) is installed it also refuses a `plugin.wasm` that
  imports an interface its permissions don't cover; without it, it says it didn't check (Lumen checks again at install).
- `keygen` makes an author key (`<name>.key`, `<name>.pub`) in your user config folder under
  `JL-Studios/plugin-keys`, and refuses to write inside a git repository.
- `digest` writes the 32-byte package digest; `verify` checks a signed package against a `.pub` file.

## Signing

`SIGNATURE` is a minisign (Ed25519) signature over the package digest: blake3 over the sorted list of
`path \0 blake3(file) \n` for every file except `SIGNATURE`, so repacking the zip doesn't break it.

- **Author key.** Any valid signature shows *Signed by <author> - key A1B2-C3D4*. The key is pinned to the
  plugin's id at first install, and an update signed by a different key is refused.
- **From JL Studios.** Only JL Studios' own plugin key, built into Lumen. The id prefix `studio.jl.` is
  reserved for it. A plugin from JL Studios gets no extra permissions.
- **Unsigned** installs only in Developer mode.

Keep your `.key` file out of every repository. The key under
`src-tauri/crates/photos-plugins/tests/fixtures/test-key/` is a published test key for Lumen's own tests; never
use it for anything you ship.

## Quick-starts

Each takes an afternoon or less. Start from the steps below, change the id, and pack it.

### A theme

See [THEMES.md](THEMES.md): a `.lumentheme` is a `theme.toml`, an optional `theme.css` and its fonts and images.

### A preset pack

1. Set up Develop the way you want, then write the preset as JSON (a preset file in `presets/`):
   the values you change in `develop`, and the panels they belong to in `panels`.
2. Or export Lightroom presets as `.xmp` and put them in `presets/`; Lumen converts them.
3. Add a `look` to a JSON preset when AI assistants should be able to offer it.
4. Pack and install; the presets appear in Develop in a group named after your plugin.

### An importer

1. Start a Rust crate on the `lumen-plugin` SDK with the `importer` feature; set the id, label and `hint` in `plugin.toml`.
2. In `enumerate`, list what the chosen folder holds with `chosen::list` and return one `Item` per photo. Give each a
   `path` (relative to the chosen folder) so Lumen opens the file itself; leave `path` out only for something that isn't
   a file, and return its bytes from `read`.
3. In `extras`, return what you know about a photo (title, caption, albums, keywords, date, place) or `None`.
4. Call `chosen::progress` as you go: it is how Lumen cancels you.
5. `cargo build --release --target wasm32-wasip2 -p <name>`,
   copy `plugin.wasm` beside `plugin.toml`, pack, and load the folder in Developer mode. Run it on a **copy** of your
   photos first.
