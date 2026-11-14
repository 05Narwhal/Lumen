# Writing Lumen plugins

Plugin API 1 (Lumen 26.2.0 and later). A plugin is a `.lumenplugin` file: a zip with a
`plugin.toml` manifest and the files the manifest names. Most plugins are data (a theme, presets,
an app list, a prompt) and run no code. An importer or an AI tool is a WebAssembly component that
runs in Lumen's sandbox. Everything on this page is checked by Lumen when a plugin is installed;
the examples in [`plugins/examples/`](../plugins/examples) are the quickest way to see each kind.

The authority for every rule is the code: `photos-plugins` (`manifest.rs`, `package.rs`,
`theme.rs`, `licences.rs`) and the design in
`docs/superpowers/specs/2026-11-13-round-23-design.md`. If this page and Lumen disagree, Lumen is right.

## Kinds

| Kind | What it adds | Runs code | Declared by |
| --- | --- | --- | --- |
| Theme | A look for the app (colours, radii, a heading font), on one of the four built-in themes | no | `[[themes]]` |
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
themes/*.toml          presets/*.json and *.xmp          apps.toml
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

[[themes]]                      # one table per theme file
file = "themes/nord.toml"

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
key = "albums_from_folders"
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

- **Adds**: one line per thing it adds (*A theme: "Nord"*, *3 Develop presets*, *An app: "GIMP"*,
  *A prompt for AI assistants: "film-look"*, *An importer: "Folder with notes"*, *1 tool for AI assistants*);
- **It can**: the sentences above for each permission, or *It runs no code.* for a data plugin;
- **It can never**: touch your originals or your library, use the internet, or start other programs;
- **Licence**, and for an update *Update 1.0.0 -> 1.1.0* with new permissions marked *New*.

A *problem* replaces **Install** with the reason: an unsupported API, a missing or malformed licence,
a theme that fails contrast, a `plugin.wasm` import the permissions don't cover, a preset that
doesn't read. Notes (for example Lightroom settings that weren't converted) are shown and don't block.

## Licences

`license` and each `[[assets]]` licence are SPDX expressions (`AND`, `OR`, parentheses and `WITH`).
Lumen only checks that you declared one and that it is well-formed; it doesn't decide which licence
you may use. A `LicenseRef-<name>` needs its text in a `LICENSE` file in the package. The install
sheet shows the licence as written.

A plugin list or registry may set a stricter rule (the planned community registry lists only
OSI-approved licences and no no-derivatives ones). That rule belongs to the registry, not to the app,
so a plugin you install from a file always installs.

Put the licence text of anything you bundle next to it (the Nord example keeps the Fraunces font's OFL
text in `assets/fonts/OFL.txt`).

## Themes

A theme file (`themes/<name>.toml`) sets design tokens on one of the four built-in themes:

```toml
id = "nord"                  # 1-40 of a-z, 0-9 and "-"; unique in the plugin
name = "Nord"                # at most 40 characters
tagline = "Arctic blues on slate."   # at most 100
base = "darkroom"            # gallery | darkroom | paper | glass
default_scheme = "dark"      # light | dark: used the first time it is picked

[light]
bg-base = "#e5e9f0"
accent = "#3b6a99"

[dark]
bg-base = "#2e3440"

[font]                       # optional heading font
file = "assets/fonts/Lora.woff2"   # .woff2, at most 2 MB, listed in [[assets]]
weight = 600
style = "normal"
```

Every value is text. Anything left out comes from `base`. There is no raw CSS: Lumen parses each
value and writes the CSS itself.

- **Colours**: `oklch()`, `#hex`, `rgb()` or `hsl()`, with alpha.
- **Lengths**: `px` within a range per token (`border-width` 0-3, `radius-sm|md|lg` 0-32, `tile-gap` 0-24,
  `blur` 0-40, `radius-pill` 0-999).
- **Shadows**: up to three layers of `x y blur colour`, for example `0 6px 16px #0003`, or `none`.
- **`wash`**: up to three radial gradients.
- **Tokens** you can set: `bg-base`, `bg-raised`, `bg-sunken`, `develop-stage-bg`, `bg-sidebar`,
  `bg-overlay`, `fg-primary`, `fg-muted`, `fg-subtle`, `fg-faint`, `accent`, `accent-hover`,
  `accent-fg`, `accent-soft`, `border`, `border-width`, `scrollbar-thumb`, `radius-sm`, `radius-md`,
  `radius-lg`, `radius-pill`, `tile-gap`, `shadow-sm`, `shadow-md`, `shadow-lg`, `blur`, `wash`.
  A theme that sets only `accent` gets `accent-hover` and `accent-soft` derived from it.
  The heading font, weight and style come from `[font]`.
- **No** `url()`, `var()`, `;` or `}` in any value.

**Contrast checks.** Lumen checks the theme's readable pairs in light and dark and refuses an
install that fails, naming the pair and its ratio (Developer mode loads it anyway and shows the
failures under *Details*). Text pairs need 4.5:1: `fg-primary`, `fg-muted` and `fg-subtle` on
`bg-base`, `bg-raised`, `bg-sunken` and `bg-sidebar`; `accent` on `bg-base` and `bg-raised`;
`accent-fg` on `accent` and `accent-hover`; `fg-muted` and `fg-primary` on `develop-stage-bg`.
`fg-muted` on `bg-sunken` (a disabled button) needs 3:1. Borders and the wash aren't checked.

**Fonts.** `woff2` only, at most 2 MB, listed in `[[assets]]` with its licence.
Lumen serves it from its own address; the generated `@font-face` points only there.

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

The WIT package is `lumen:plugin@1.0.0` (`src-tauri/crates/photos-plugins/wit/lumen-plugin.wit`), and
its worlds are `importer-plugin`, `tools-plugin` and `full-plugin` (both). A component imports only the
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
`scripts/plugins/build-examples.mjs` does this for the two examples.

The sandbox: no network, no sockets, no files except through `chosen` (below the folder the person
chose) and `storage` (its own 50 MB). Calls have time limits (an importer is stopped when `progress` reports
that Lumen wants it to stop, a tool call ends after 30 s), and memory is capped.

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

Each takes an afternoon or less. Copy the matching folder from `plugins/examples/`, change the id, and pack it.

### A theme

1. Copy `plugins/examples/theme-nord`. Change `id`, `name` and the theme file's colours.
2. Pick `base` by which built-in theme's shape (radii, shadows, blur, heading font) you want to keep.
3. Set both `[light]` and `[dark]`, at least the surfaces (`bg-*`), inks (`fg-*`) and `accent` with `accent-fg`.
4. `node scripts/plugins/pack.mjs pack <folder> --out <file>`, then Settings -> Plugins -> Developer mode ->
   *Load a folder...* or install the packed file. If the install sheet reports a failed pair, adjust that colour.
5. No font? Delete `[font]`, the `[[assets]]` entry and `assets/`. Want one? Use a `woff2` under 2 MB with an OFL
   (or another) licence, list it, and ship its licence text.

### A preset pack

1. Set up Develop the way you want, then write the preset as JSON (copy `presets-film/presets/faded-matte.json`):
   the values you change in `develop`, and the panels they belong to in `panels`.
2. Or export Lightroom presets as `.xmp` and put them in `presets/`; Lumen converts them.
3. Add a `look` to a JSON preset when AI assistants should be able to offer it.
4. Pack and install; the presets appear in Develop in a group named after your plugin.

### An importer

1. Copy `plugins/examples/importer-folder-notes`; change the id, label and `hint`.
2. In `enumerate`, list what the chosen folder holds with `chosen::list` and return one `Item` per photo. Give each a
   `path` (relative to the chosen folder) so Lumen opens the file itself; leave `path` out only for something that isn't
   a file, and return its bytes from `read`.
3. In `extras`, return what you know about a photo (title, caption, albums, keywords, date, place) or `None`.
4. Call `chosen::progress` as you go: it is how Lumen cancels you.
5. `node scripts/plugins/build-examples.mjs` (or your own `cargo build --release --target wasm32-wasip2 -p <name>`),
   copy `plugin.wasm` beside `plugin.toml`, pack, and load the folder in Developer mode. Run it on a **copy** of your
   photos first.
