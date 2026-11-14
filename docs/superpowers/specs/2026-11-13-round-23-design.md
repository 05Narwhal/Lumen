# Round 23 — design: plugins and themes (`26.2.0-alpha`)

Julian's brief (2026-11-13): Round 23 is themes and plugins; an extension store comes later. It
ships as `26.2.0-alpha`. A plugin must never reach `originals/` or `library.db`, and originals are
read-only. Lumen stays offline and local-first, with no telemetry. Everything a plugin loads must
allow commercial use.

Markers, as in Round 22: **J:** is Julian's decision. **Call** is the spec author's call, for
Julian to check while reading. **Builder:** is left to the lane that builds it.

## §0 Intent

**J:** Julian writes the first plugins himself; later anyone can write their own, so the
permission model is built now, not later. Everything in 26.2.0 is **free**: a lot of testing is
still to be done. (Amended 2026-11-14: there is no Pro, ever; see the open-source plan.)

What the round is for: Lumen can be extended without a Lumen release. That covers a new look, a
pack of Develop presets, an editor that the built-in table misses, an importer for another app's
export, and a tool for an AI assistant. A plugin made by a stranger must be as safe to install as
one made by JL Studios.

**Success:**
- With the SDK, Julian can write each of these in an afternoon: a theme, a preset pack and an
  importer.
- He installs a plugin from a file and sees exactly what it can do before it does anything.
- He can turn a plugin off, or remove it with undo.
- Nothing in his library changes, except the photos he imports and the drafts he (or an
  assistant through a plugin tool) makes.

**Anti-goals:** each of these counts as failure even if everything else works:
- a plugin that can read or change the library;
- a plugin that slows Lumen's start or the grid;
- any request leaving the machine because of a plugin;
- a theme that makes Lumen unreadable;
- a JL Studios plugin with powers a stranger's plugin can't get.

**Non-goals for 26.2.0** (revisited with the store):
- the store and any catalogue fetched from the network;
- network access for plugins;
- plugin UI (panels, pages, arbitrary CSS or JS in Lumen's window);
- plugins inside the Develop render pipeline (new tools, LUTs);
- export targets;
- scheduled or background plugin jobs;
- paid gating of any kind (there is no Pro);
- automatic plugin updates;
- granting only some of a plugin's permissions;
- plugins per library;
- plugins answering while Lumen is closed (`Lumen --mcp` lists none then).

## §1 What a plugin can be

A plugin is one package that can contain any mix of the kinds below.

| Kind | Code? | What it adds | Where it shows |
|---|---|---|---|
| **Theme** | no | one or more themes, each built on one of the four built-ins | theme picker, after the four |
| **Presets** (Develop looks) | no | Develop presets; any preset may also describe itself as an MCP **look** | Develop's presets list, a group per plugin; the MCP looks catalogue |
| **Apps** | no | entries for *Edit in* / *Open with*, in the built-in table's shape | menus, Settings → External apps (*added by <plugin>*) |
| **MCP prompts** | no | prompt texts for AI assistants | the MCP server's prompts, while AI assistants are on |
| **Importer** | yes | a source that reads another app's export (a folder or a file the person picks) | Import page, a card *From <label>* |
| **MCP tools** | yes | tools for AI assistants over Develop's filmstrip | the MCP server's tools, while AI assistants are on |

**Data kinds run no code.** They are files that Lumen reads, checks and applies itself. **Code
kinds** ship one WebAssembly component, `plugin.wasm`, that runs in the sandbox (§2).

## §2 Where it runs and what it can touch

**J:** option A, a WebAssembly sandbox inside Lumen. Rejected:
- **separate helper programs:** native code can open `library.db`, and blocking that needs a
  different OS sandbox on each platform;
- **JS in a web view:** the work leaves Rust, and the web view can reach the network and sits
  beside `invoke`.

- **Runtime:** `wasmtime` with its component model; the plugin's interface is written in WIT. It
  runs inside Lumen's process, in a new crate `photos-plugins`, on its own thread pool
  `lumen-plugins` at below-normal priority (like job workers). Nothing runs on the main thread,
  and every command stays `async`.
- **Instances:**
  - Instances start on demand and are dropped after 60 s idle.
  - One instance per import. One shared instance per plugin for MCP tools, with its calls run one
    at a time.
  - Compiled code is cached under app data `cache/plugins/`, keyed by the hash of the `.wasm` and
    the wasmtime version.
- **Limits:**
  - 512 MB of memory per instance.
  - 30 s per call, enforced with wasmtime epoch interruption. Each importer `progress` call resets
    the clock.
  - 3 traps or timeouts in one session stop the plugin until restart: state *Stopped working*,
    never an error dialog.
- **WASI:** the host links a WASI context with no preopened folders, no environment, no
  arguments and **no sockets or name lookup**. Clocks and random are allowed. `stdout` and
  `stderr` go to the plugin's log (§5).
  **Builder:** prove it with fixtures that try to open a socket and a file, and fail.
- **Capabilities are host functions.** A plugin can do only what its permissions (§3) give it. At
  install, Lumen reads the component's imports. An import of an interface the manifest doesn't
  ask for refuses the package (*uses X without asking for it*). At run time, the linker defines
  only the granted interfaces.
- **What a plugin can never get, because no host function for it exists:**
  - a path inside a library;
  - a byte of `originals/`;
  - `library.db`;
  - the network;
  - a process;
  - the clipboard;
  - another plugin's data;
  - Lumen's settings.

  An MCP tool sees photos only as previews rendered by Lumen and as the metadata in §3. That is
  stricter than "originals are read-only" (**Call**): no listed kind needs original bytes.
- **The folder chosen for an import** is refused if it is inside a registered library's folder,
  or contains one. A path that leaves the chosen root (through `..`, an absolute path, or a
  symlink or junction that resolves outside it) gets an error. With `pick = "file"`, the root is
  that one file: `list` returns only it, and `read` and `stat` work on it.
- **Data kinds:**
  - Themes are checked and turned into CSS in Rust (§6).
  - Presets go through the existing recipe checks.
  - App entries go through §7's rules.
  - Prompts are plain text up to 32 KB.
- **Boot:** the only start-up work is reading `plugins.json` and the enabled themes, budget
  < 20 ms. Code plugins start only when used, so first grid paint (§15 of the architecture spec)
  is unchanged.

## §3 Permissions, and what Julian sees before installing

API 1 has four permissions. The manifest asks for them, the person grants **all or none**
(**Call**: partial grants make plugins fail in ways their authors never tested), and the host
enforces them.

| id | Shown as | Host interface |
|---|---|---|
| `import.read-chosen` | *Read the folder or file you choose when you import with it* | `chosen`: list, stat and read below the chosen root; `progress` |
| `photos.read` | *See the photos in Develop's filmstrip: previews up to 1024 px, and their date, camera, lens, size, rating, label, flag and place* | `photos`: scope, metadata, preview (current draft applied), recipe |
| `develop.write-drafts` | *Change Develop drafts of those photos (every change can be undone)* | `drafts`: a merge-patch through the same path as the MCP `edit` |
| `storage` | *Keep its own files (up to 50 MB)* | `storage`: read, write and remove named files in `plugins/data/<id>/` |

Always present, no permission needed: `host.log` and `host.setting` (the values of the plugin's
own settings, §5).

- `photos.read` and `develop.write-drafts` work only while AI assistants are on, and only within
  the MCP scope (Develop's filmstrip, `mcp/scope.rs`), exactly like the built-in tools.
- A draft written by a plugin gets a history step labelled `AI: <plugin>: <label>`, and the page
  shows it live (`mcp/write.rs`).

**The install sheet** (a modal; it asks for consent, so it is not a confirm of a reversible
action):

```
┌───────────────────────────────────────────────┐
│ [icon]  Google Takeout          1.0.0         │
│         JL Studios · ✓ From JL Studios        │
│                                               │
│ Adds                                          │
│   An importer: "Google Takeout"               │
│ It can                                        │
│   • Read the folder or file you choose when   │
│     you import with it                        │
│ It can never                                  │
│   Touch your originals or your library, use   │
│   the internet, or start other programs.      │
│ Licence  MIT                                  │
│                         [Cancel]  [Install]   │
└───────────────────────────────────────────────┘
```

- **Data-only plugins:** *It can* reads *It runs no code.*
- **Trust line:**
  - *✓ From JL Studios* for JL Studios' key;
  - *Signed by <author> · key A1B2-C3D4* for an author key;
  - *⚠ Unsigned* (Developer mode only).
- **Problems** (unsupported API, a licence that isn't accepted, a theme that fails contrast, an
  undeclared import) replace *Install* with the reason and *Close*.
- **Updates:** the sheet heads *Update 1.0.0 → 1.1.0*, and new permissions are marked *New*. An
  older version reads *Older than the installed 1.1.0*.

**After install:** Settings → **Plugins** (a new sixth group after Connections, **Call**:
it grows into the store) lists each plugin.
- Each row shows: icon, name, version, trust line, kind chips, a switch, and a ⋯ menu (*Details*,
  *Settings*, *Log* in Developer mode, *Remove*).
- Empty state: *No plugins yet.* with action *Install from file…*.
- **Turning a plugin off** is instant:
  - its theme falls back to its base theme;
  - its presets, apps and importer card disappear;
  - its MCP tools and prompts leave, with `notifications/tools/list_changed`.
- **Remove** never confirms (UX rule). It moves the folder to `plugins/removed/<id>/` with a 6 s
  undo toast. The folder and the plugin's `data/` are deleted at next start.
- **What stays after removal:** photos it imported, drafts it wrote, and presets the person
  duplicated (they belong to the person now).

## §4 The package and the manifest

**Package:** a `.lumenplugin` file, a zip (**Call**: TOML manifest, friendlier to hand-write than
JSON). It contains:

```
plugin.toml            manifest (required)
icon.png               ≤ 256 px, optional
plugin.wasm            only for importer / tools
themes/*.toml          presets/*.json | *.xmp     apps.toml
prompts/*.md           assets/**                  LICENSE, NOTICES (optional)
SIGNATURE              §8
```

**Package limits:**
- 50 MB per package, 2,000 files, no nested archives;
- every path relative, with no `..`, no absolute paths and no drive letters;
- names are compared case-insensitively, so two names that differ only in case refuse the
  package.

```toml
id = "studio.jl.takeout"        # reverse-DNS, [a-z0-9.-], 3–64 chars, unique
name = "Google Takeout"
version = "1.0.0"               # semver
author = "JL Studios"
description = "Imports a Google Photos Takeout with its dates, places and albums."
license = "MIT"                 # SPDX expression, §9
homepage = "https://…"          # optional; shown as text, never fetched
lumen = ">=26.2.0"              # semver range of Lumen versions
api = 1
permissions = ["import.read-chosen"]

[[themes]]                      # data kinds point at files
file = "themes/nord.toml"

[presets]
dir = "presets"

[apps]
file = "apps.toml"

[[prompts]]
name = "film-look"
description = "Retouch toward a warm film look"
file = "prompts/film-look.md"

[importer]                      # code kinds are declared so the sheet can list them
label = "Google Takeout"
pick = "folder"                 # folder | file
extensions = ["zip"]            # for pick = "file"
hint = "Choose the folder Google Takeout made"

[[tools]]
name = "level_horizon"
description = "…"

[[assets]]                      # every bundled file that carries its own licence
path = "assets/fonts/Lora.woff2"
license = "OFL-1.1"

[[settings]]                    # rendered by Lumen; read by the plugin through host.setting
key = "albums_from_folders"
label = "Use Takeout's folders as albums"
type = "bool"                   # bool | number | text | choice
default = true
```

- The binary checks the manifest with `photos_plugins::manifest::parse`.
- **Rules:**
  - unknown top-level keys are an error;
  - a code kind without `plugin.wasm`, or `plugin.wasm` without a code kind, is an error;
  - a tool `name` matches `[a-z0-9_]{1,40}`.
- **API version:** `api` above what this Lumen supports, or a `lumen` range that excludes this
  version, gives state *Needs a newer Lumen* (installable, not loaded).
- The WIT package is `lumen:plugin@1.0.0` (api 1).

## §5 On disk, settings and log

Everything sits under `config_dir/JL-Studios/Lumen/plugins/`:

```
plugins.json                  state: per id {version, enabled, granted, key, installedAt, dev, settings}
                              + {developerMode, devFolders}
installed/<id>/               unpacked package (Lumen never writes into it after install)
data/<id>/                    the plugin's `storage`, 50 MB quota
removed/<id>/                 waiting for undo; deleted at next start
```

- **Installing:**
  - unpack to `installed/<id>.tmp/`, check everything, then rename;
  - an update replaces the folder the same way and keeps `data/<id>/`;
  - `plugins.json` is written atomically (temp file + rename).
- Plugins are app-wide: every library sees the same plugins.
- **No migration:** nothing about plugins lives in a library, so **0040 stays the next
  migration**.
- **Settings** declared in the manifest are rendered by Lumen with its own controls. The values
  are stored in `plugins.json`, and the plugin reads them through `host.setting(key)` (JSON).
- **Log:** `host.log` lines and WASI stdout/stderr go to Lumen's log with `plugin=<id>`. The last
  200 lines are kept in memory for *Log* (Developer mode).

**Developer mode** (Settings → Plugins, off by default) is for people making plugins:
- it allows unsigned packages and *Load a folder…* (an unpacked plugin, listed with a *Dev* badge,
  and a *Reload* item that re-reads it and restarts its instance);
- it runs the same checks, the same install sheet and the same sandbox (**Call**);
- turning it off unloads dev folders and unsigned plugins; it doesn't delete them.

## §6 Themes

A theme file:

```toml
id = "nord"
name = "Nord"
tagline = "Arctic blues on slate."
base = "darkroom"            # gallery | darkroom | paper | glass
default_scheme = "dark"

[light]
bg-base = "oklch(96% 0.01 240)"
accent = "#5e81ac"
# …

[dark]
bg-base = "oklch(24% 0.02 240)"
# …

[font]                       # optional heading font
file = "assets/fonts/Lora.woff2"
weight = 600
style = "normal"
```

- **Tokens:** a theme may set exactly the tokens the four built-in partials set (UX rule: surfaces,
  foregrounds, accent, borders, radii, tile gap, shadows, blur, heading font, wash). Any token it
  leaves out comes from `base`.
  **Builder:** take the list from the union of `src/styles/themes/_*.scss` and keep it in a single
  Rust table.
- **Values are typed, never raw CSS:**
  - colours: `oklch()`, `#hex`, `rgb()` or `hsl()`, with alpha;
  - lengths: px within a range per token (radius 0–32, tile gap 0–24, blur 0–40, border width
    0–3);
  - shadows: up to 3 layers of offset, blur and colour;
  - `wash`: up to 3 radial gradients.

  No `url()`, no `var()`, no `;` or `}`. Rust generates the CSS.
- **Applying a theme:**
  - the DOM keeps `data-theme="<base>"` and gains `data-plugin-theme="<plugin>/<id>"`;
  - one `<style id="plugin-themes">` holds the rules for every enabled plugin theme:
    `[data-theme="<base>"][data-plugin-theme="…"]`, and the `[data-mantine-color-scheme="dark"]`
    variant;
  - the style comes from the command `plugin_theme_css`;
  - component styles never branch on theme, so nothing else changes.
- **Contrast:**
  - Rust runs the same pairs and thresholds as `src/styles/contrast.test.ts`, in both schemes.
  - A theme that fails is refused at install, with the failing pair and its ratio. In Developer
    mode it loads with the failures shown under *Details*, so authors can iterate.
  - Rust and TS stay in lockstep through a golden, `src/styles/contrast-golden.json`, regenerated
    by a Rust test and read by `contrast.test.ts`.
  - **Builder:** if the TS test computes from the SCSS, the golden holds the pairs and thresholds
    plus sample results.
  - Rust converts every accepted colour form (hex, rgb, hsl, oklch) and is the reference.
  - "Readable" means the existing text and accent pairs on their surfaces. Borders and the wash
    aren't checked (**Call**).
- **Fonts:**
  - `woff2` only, ≤ 2 MB, listed in `[[assets]]` with a licence from §9;
  - served at `photos://plugin-asset/<id>/<path>`, a path that must be one of the manifest's
    assets;
  - the generated `@font-face` points only there.
- **Settings:** the theme choice becomes `ThemeChoice` in `photos-core/src/settings.rs`. It is
  `Builtin(ThemeId)` or `Plugin { plugin, theme }`, it serializes back-compatibly (an old settings
  file still reads), and a missing or disabled plugin theme shows its `base`.

## §7 Presets, looks, apps and prompts

- **Presets:**
  - The files are Lumen's own preset JSON (the shape of `photos_core::develop_presets::Preset`'s
    `develop` and `panels`, with a `recipeVersion`), or Lightroom `.xmp` converted through the
    existing converter (`photos_core::lightroom::map::convert`) whenever the package is read: at
    inspect and at load, never written into `installed/<id>/`. What wasn't converted is listed on
    the install sheet.
  - They are read from `installed/<id>/` and shown read-only in their own group named after the
    plugin, like built-ins. Nothing is copied into `develop_presets`.
  - *Duplicate to my presets* lets Lumen write a normal preset row at the person's request.
  - A plugin preset carries whatever a Lumen preset can carry today, masks included.
  - **Builder:** plugin preset ids must be stable across restarts and never collide with built-in
    (negative) or database ids. A string key `plugin:<id>/<file>` beside the numeric id is
    acceptable.
- **Looks:**
  - A preset JSON may add `look = { intent, hues, mono, key, skinSafe, likesSky }`. It then joins
    the MCP looks catalogue (`mcp/looks.rs`) and is scored like the built-in looks, with its
    `develop` as the patch.
  - It is never the `safe` look.
  - The playbook's limit of 4 offered looks still holds.
- **Apps:**
  - `apps.toml` lists entries in the built-in table's shape (`AppProfile`, Round 22 §7) and is read
    by a third `AppSource`, `PluginApps`.
  - Detection is Lumen's; an app that isn't installed isn't shown.
  - Plugin entries name only detection identifiers (registry keys, bundle ids, `.desktop` names,
    install-folder globs). They give no argument templates: Lumen passes the file as the only
    argument.
  - An entry whose target resolves to a shell or script host is refused (`cmd`, `powershell`,
    `pwsh`, `wscript`, `cscript`, `mshta`, `rundll32`, `sh`, `bash`, `zsh`, `python*`, `node`,
    `osascript`; **Builder:** one list, tested; the final review added `py`, `pyw`, `env`, and on macOS Terminal, iTerm, Script Editor and Automator by name and bundle id). A resolved target is also refused when it lies in Lumen's plugins folder or the plugin's own folder, and a glob must start in an install root (`%ProgramFiles%`, `%ProgramFiles(x86)%`, `%LocalAppData%/Programs`).
  - A plugin app always **gets a copy**, whatever its `writes`, unless the person changes its send
    mode in Settings → External apps (**Call**: a plugin can't make Lumen hand over an original).
- **Prompts:**
  - They are added to the MCP server's prompts as `<plugin-short>__<name>` (**Builder:**
    derive the short form from the id's last segment, fall back to the full id with `.` → `_`
    on collision; the same rule names tools).
  - They are offered only while AI assistants are on.

## §8 Signing, trust and updates

> **Amended 2026-11-14 by the open-source plan.** The later store is Round 24's registry: a public git repo with a static `index.json`, whose CI lists only OSI licences, refuses ND and treats NC as asset-only. Automatic updates and any store traffic come with it, over the same kind of static manifest as Lumen's updater. The official key stays *From JL Studios*, held by Julian.

- **Signature:** `SIGNATURE` is a minisign (Ed25519) signature over the package digest. The
  digest is blake3 over the sorted list of `path\0blake3(file)\n`, every file except
  `SIGNATURE`, so a repacked zip still verifies. Verification uses `minisign-verify` (MIT). The
  signing key format is the same as the updater's (`tauri signer`).
- **Three trust levels.** The sandbox is the protection; a signature proves only who made a
  plugin and that an update comes from the same hands.
  - **From JL Studios:** signed by the JL Studios plugin key. Its public key is embedded at build
    time (`LUMEN_PLUGIN_PUBKEY`, else a git-ignored key file, like the updater key).
    **Call:** this is a key separate from the release key, so a leak of one doesn't compromise
    the other. With no key built in, no package is *From JL Studios*. The key doesn't exist yet;
    Julian creates it with the release key.
  - **Author key:** any other valid signature. The key's fingerprint is shown, and it is pinned
    to the id on first install. An update signed by a different key is refused (*This update was
    signed by a different key.*).
  - **Unsigned:** refused unless Developer mode is on.
  - A plugin from JL Studios gets no extra permissions.
- **Reserved ids** (**Call**): ids starting with `studio.jl.` belong to JL Studios.
  - When a JL Studios key is built in, such an id installs only with that key's signature
    (Developer mode excepted).
  - The JL Studios key may replace the pinned key of a `studio.jl.*` id, and of no other id. That
    lets packages first signed with a local author key be re-signed later.
- **Keys before the JL Studios key exists** (**Call**):
  - Test fixtures are signed with a throwaway key committed under the fixtures and marked
    test-only.
  - D1 and D2 packages are signed with Julian's own author key, made by `pack.mjs keygen` and
    kept outside the repo. They show *Signed by JL Studios · key …* until the real key is built
    in.
  - Nothing in 26.2.0 waits on the real key.
- **Updates in 26.2.0** (**Call**): installing a newer package of the same id (same key, sheet
  shows what's new). Automatic plugin updates come with the store, over the same kind of static
  manifest as Lumen's updater. 26.2.0 makes **no new network request** of any kind.
- **Install routes:**
  - Settings → Plugins → *Install from file…*;
  - opening a `.lumenplugin` file (a file association in `tauri.conf.json`; the single-instance
    check forwards the path to the running Lumen);
  - the palette's *Install plugin from file…*.

  All three open the sheet.

## §9 Licences

> **Amended 2026-11-14 by the open-source plan (Task 2).** Lumen no longer polices which licence a plugin uses, so the table below is superseded. `photos-plugins::licences::check` requires a declared, well-formed SPDX expression (`AND`, `OR`, parentheses, `WITH`; each identifier shaped like an SPDX id or `LicenseRef-<name>`), and a `LicenseRef-*` needs a `LICENSE` file in the package. GPL, NC and unknown identifiers install; the sheet shows the licence. A plugin's code runs in a sandbox behind a WIT interface, so it is not linked into Lumen. Which licences the registry lists is the registry's CI policy (Round 24). Lumen itself is `GPL-3.0-or-later`; the plugin SDK, WIT and examples are `MIT OR Apache-2.0`.

- **Lumen's own new dependencies:** `wasmtime`, `wasmtime-wasi` (Apache-2.0 WITH
  LLVM-exception), `minisign-verify` (MIT), `zip`, `toml` and `semver`. Each goes into
  `docs/LICENSING-AUDIT.md`, followed by `yarn run licenses`.
  **Builder:** confirm each version's licence.
- **Size check:** report the growth of the binary and the installer. Growth over +20 MB stops lane
  A2 for Julian.
- **A plugin's licence:**
  - `license` must be an SPDX expression, and each `[[assets]]` entry has its own.
  - Lumen shows the licence on the sheet and refuses a package outside these lists (**Call**:
    enforced, not just shown, so a package can't slip through as "commercial use by mistake"):

    | | Accepted |
    |---|---|
    | code and the package | MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, Unlicense, MPL-2.0, CC0-1.0, and `LicenseRef-<name>` with the text in `LICENSE` (closed or paid plugins for the later store) |
    | data and assets only | the above, plus CC-BY-4.0, CC-BY-SA-4.0, and OFL-1.1 (fonts) |
    | refused | anything with NC or ND; GPL, AGPL or LGPL for `plugin.wasm` (copyleft code inside Lumen's process); identifiers Lumen doesn't know |

  - Expressions with `OR` pass if one branch passes; with `AND`, every part must pass.
- Settings → *Licences* gains a *Plugins* section, built from the installed manifests (not
  `notices.json`).
- **Builder:** the example and first-party plugins carry MIT and their assets' own licences.

## §10 Code kinds: importer and MCP tools

The whole WIT package, both exports included, is written by lane A2 with its fixtures; A3 and A4
build the host-side adapters over it.

**Importer** (the export `importer` in WIT; **Builder:** exact WIT shape):
- `enumerate() -> result<list<item>>`, where an item is `{key, name, size, modified?, path?}`;
- `read(key, offset, len) -> result<bytes>`, only for items without `path`;
- `extras(key) -> option<extras>`, where extras are `{favourite, albums, title, caption,
  keywords, capturedAt?, lat?, lon?}`, mapped onto `photos_import::ItemExtras`;
- `sidecars(key) -> list<item>`.

How it plugs in:
- An item with `path` (relative to the chosen root) is opened by the host itself, a fast path
  with no copy through wasm.
- The binary wraps the plugin as an `ImportSource` (`src-tauri/src/plugins/importer.rs`), so
  ingest, verify, sidecars, skips and history are unchanged.
- The batch gets a new `photos_core::batches::SOURCE_PLUGIN` kind (a constant, no migration), and
  its label is the importer's label.
- On the Import page, the card *From <label>* asks for the folder or file, then runs the normal
  flow and progress.
- `is_connected` is true while the chosen root exists.

**MCP tools** (the export `tools`):
- `call(name, argsJson) -> result<string>`. The names and descriptions come from the manifest;
  the input schema comes from a `tools()` export, checked at load time against the manifest.
- Results are text or JSON only (**Call**): an assistant that wants a picture calls Lumen's own
  `view`.
- They are listed next to the built-in tools as `<plugin-short>__<name>`, behind the same guard,
  scope and per-call log (`tool, id, ms, ok`), with the 30 s call limit.
- Settings → AI assistants reads *Plugins add N tools*.

## §11 Commands, types and events (names used across lanes)

- **Crate:** `src-tauri/crates/photos-plugins` (depends on `photos-core` only, never `tauri`).
  - Modules: `manifest`, `package`, `signature`, `licences`, `store`, `theme`, `host/` (engine,
    limits, linker), `wit/lumen-plugin.wit`.
- **Binary:** `src-tauri/src/plugins/` with `mod.rs` and `state.rs` (the loaded store in app
  state, events), `themes.rs`, `data.rs` (presets, looks, apps, prompts glue), `importer.rs` and
  `mcp_tools.rs`. Commands live in `src-tauri/src/commands/plugins.rs` (C1), except
  `plugin_theme_css` / `list_plugin_themes` in `commands/plugin_themes.rs` (B1),
  `list_plugin_importers` / `start_plugin_import` in `commands/plugin_import.rs` (A3) and
  `duplicate_plugin_preset` beside the preset commands in `commands/develop.rs` (B2), so each
  lane owns whole files.
- **Kind checks:** every kind is checked by exactly one checker. Theme checks (`theme.rs`) and the
  imports check (`host/`) live in the crate; presets, apps and prompts are checked in the binary
  (`plugins/data.rs`) through a `KindCheck` trait the store calls. A kind that no checker claims
  is a problem (*<kind> isn't supported by this Lumen*), so the store fails closed while lanes
  are still merging.
- **Types (ts-rs):**
  - `PluginInfo { id, name, version, author, description, license, trust, kinds, permissions,
    enabled, dev, state, iconUrl, settings, assets }` (`assets`: `{ path, license }` per
    `[[assets]]`, for Settings → Licences)
  - `PluginTrust = JlStudios | Author { name, fingerprint } | Unsigned`
  - `PluginKind = Theme | Presets | Apps | Prompts | Importer | Tools`
  - `PluginPermission = ImportReadChosen | PhotosRead | DevelopWriteDrafts | Storage`
  - `PluginState = Ready | Off | NeedsNewerLumen | Problem { reason } | Stopped { reason }`
  - `PluginInspect { info, adds, problems, notes, update: Option<{ from, newPermissions }>,
    older: Option<installed version> }` (`notes`: what a `.xmp` lost in conversion, and in
    Developer mode the contrast failures)
  - `PluginSetting { key, label, kind, value, choices }`
  - `PluginImporter { plugin, label, pick, extensions, hint }`
  - `ThemeChoice`, and `PluginThemeMeta { plugin, id, name, tagline, base, defaultScheme }`
- **Commands:**
  - `list_plugins`
  - `inspect_plugin_package(path)`
  - `install_plugin(path)`
  - `set_plugin_enabled(id, on)`
  - `remove_plugin(id) -> token`, `undo_remove_plugin(token)`
  - `set_plugin_setting(id, key, value)`
  - `set_plugin_developer_mode(on)`
  - `load_dev_plugin(folder)`, `reload_dev_plugin(id)`
  - `plugin_log(id)`
  - `plugin_theme_css`
  - `list_plugin_themes`
  - `list_plugin_importers`
  - `start_plugin_import(plugin, chosen)`
  - `duplicate_plugin_preset(key)`
  - `take_pending_plugin_package` (C1: the path a cold start was opened with) and
    `plugin_tool_count` (A4: *Plugins add N tools*), added by the plans session
- **Routes:** `photos://plugin-icon/<id>` and `photos://plugin-asset/<id>/<path>`.
- **Events:** `plugins://changed` (install, remove, toggle, reload) and
  `plugins://open-package { path }` (the file association).
- **Errors** (`AppError.code`): `plugin_invalid`, `plugin_signature`, `plugin_key_changed`,
  `plugin_unsigned`, `plugin_licence`, `plugin_api`, `plugin_permission`, `plugin_trap`,
  `plugin_timeout`, `plugin_chosen_root`.

## §12 Amendments to apply before code (the plans session does this first)

*Applied by the plans session, 2026-11-13.*

- **Architecture spec (`2026-09-17-photos-app-design.md`):**
  - §2: remove *Plugin system* from the non-goals.
  - §3: add row **15 | Plugins | WebAssembly components (wasmtime) in Lumen's process, with
    host-function capabilities only (no files, no network, no library); data kinds are files
    Lumen checks; app-wide under `config_dir/JL-Studios/Lumen/plugins/`; spec
    `2026-11-13-round-23-design.md`.**
  - §4.3: add `photos-plugins`.
- **CLAUDE.md:**
  - non-negotiable 2: add `src → photos-plugins → photos-core`;
  - non-negotiable 7: themes from plugins are the one source of colour outside `globals.scss`:
    typed tokens checked and turned into CSS in Rust;
  - UX rules: "four themes" becomes "four built-in themes, plus plugin themes that override the
    same tokens";
  - a gotcha: wasm fixtures are rebuilt by `scripts/plugins/build-fixtures.mjs`.

## §13 Testing

- **TDD** in `photos-plugins` (manifest, package, signature, licences, store, theme) and for the
  hooks and components.
- **Wasm fixtures:**
  - Small components built from `plugins/fixtures/*` by `scripts/plugins/build-fixtures.mjs` and
    checked in under `photos-plugins/tests/wasm/`, so `cargo test --workspace` needs no wasm
    target.
  - A test compares a hash of the WIT the fixtures were built against, and fails with *run
    build-fixtures*.
  - The fixtures cover: a well-behaved importer; tools; one that opens a socket; one that reads a
    file; an endless loop; a memory hog; an undeclared import; a trap.
- **Toolchain** (**Call**):
  - Guests are native components for `wasm32-wasip2` (stable `rustup target add`), using
    `wit-bindgen`. `wasm-tools` is used for the imports check in tests and in `pack.mjs`.
  - A2 and D1 may install these on Julian's machine.
  - `cargo test --workspace` and `yarn test` need none of them.
- **Temp dirs only:** temp app data and temp libraries. A test proves the chosen root refuses a
  registered library folder. Nothing runs against the real library (CLAUDE.md).

## §14 Lanes

Each session stays at or under ~70k tokens and owns the files listed. A file is owned by exactly
one lane.
- **Shared exception:** the `generate_handler!` list and the `mod` lines in `src-tauri/src/lib.rs`,
  plus C1's setup lines there (managing the plugin state, the single-instance forwarding). Each
  lane appends its own lines, and the merge keeps them all. `docs/LICENSING-AUDIT.md` and
  `src/assets/licenses/notices.json` take rows from A1, then A2, then D1, never two at once.
- **Skeleton:** A1 creates `src-tauri/src/plugins/mod.rs` declaring every module in §11, and the
  crate's `theme.rs` and `host/mod.rs`; each file holds only the stub entry points named in the
  roadmap's *Interfaces*, failing closed, so later lanes own whole files. After A1, the crate's
  `Cargo.toml` belongs to A2 (the only wave-2 lane that adds dependencies).
- **Order:** A1 → {A2, B1, B2, C1} in parallel → {A3, A4, C2, D1} → D2 → D3 → final chat.
- **Opus** for A1 (store and signature), A2 (host) and A4 (the plans' judgement: the second
  permissioned surface, drafts written by a stranger's code); the rest Sonnet.

| Lane | What | Owns |
|---|---|---|
| **A1** store (Opus) | manifest, package (zip limits, paths), signature (digest, minisign, three trust levels, key pinning), licences (§9 lists, SPDX expressions), store on disk (§5, atomic install / update / remove / undo, `plugins.json`, `asset_path`); all ts-rs types of §11 except B1's two; the `KindCheck` trait; the skeleton (crate and binary) | `crates/photos-plugins/{Cargo.toml, src/lib.rs, manifest.rs, package.rs, signature.rs, licences.rs, store.rs, types.rs}`, `tests/fixtures/` (packages, test key, digest golden), workspace `Cargo.toml`, `src-tauri/Cargo.toml` (the dependency), `src-tauri/src/plugins/mod.rs` + stub module files |
| **A2** host (Opus) | wasmtime engine and compile cache, the whole WIT `lumen:plugin@1.0.0` (importer and tools exports included), linker per permission, WASI without sockets, memory and time limits, trap counting, `host` / `storage` / `chosen` interfaces (root confinement), the `photos` / `drafts` host traits, the imports check at install, fixtures and their script | `crates/photos-plugins/{Cargo.toml (after A1), src/host/**}`, `wit/`, `tests/wasm/`, `plugins/fixtures/`, `scripts/plugins/build-fixtures.mjs` |
| **B1** themes | theme file checks, the token table, CSS generation, Rust contrast with the golden, `ThemeChoice`, `PluginThemeMeta`, `plugin_theme_css` / `list_plugin_themes`, the `@font-face` pointing at `photos://plugin-asset`, the picker and `data-plugin-theme` | `crates/photos-plugins/src/theme.rs`, `photos-core/src/settings.rs`, `src-tauri/src/plugins/themes.rs`, `src-tauri/src/commands/plugin_themes.rs`, `src/hooks/use-theme.ts`, `src/components/theme-picker/`, `src/components/theme-preview/`, `src/types/theme.ts`, `src/lib/plugin-theme.ts`, `src/styles/contrast.test.ts`, `src/styles/contrast-golden.json` |
| **B2** data kinds | plugin presets (read-only group, `.xmp` read through the converter, *Duplicate*), looks in the catalogue, `PluginApps` with the shell refusal and *gets a copy*, prompts (checked and served; A4 lists them) | `photos-core/src/develop_presets.rs`, `photos-core/src/external_apps/{mod.rs, profile.rs, plugin_source.rs}` (an owned, runtime-built profile beside the `&'static` table), `src-tauri/src/plugins/data.rs`, `src-tauri/src/mcp/looks.rs`, `src-tauri/src/commands/develop.rs` (the preset commands), `src-tauri/src/commands/external_apps.rs`, `src/components/develop-sidebar/presets.tsx`, `src/routes/settings/external-apps.tsx`, `src/lib/menus/presets.ts`; B2 widens the preset id (string key) in one place, through every preset type and command |
| **C1** app wiring | `state.rs` (load at start, `plugins://changed`, change listeners), `commands/plugins.rs` (the §11 commands not given to B1, B2 or A3), both `photos://plugin-icon` and `photos://plugin-asset` (through `Store::asset_path`), file association and single-instance forwarding, the *Removed* sweep after start | `src-tauri/src/plugins/state.rs`, `src-tauri/src/commands/plugins.rs`, `src-tauri/tauri.conf.json` (fileAssociations), the plugin arms of `src-tauri/src/protocol.rs` |
| **A3** importer | the `ImportSource` adapter over A2's importer export, the library-folder refusal of the chosen root, `SOURCE_PLUGIN`, `start_plugin_import`, `list_plugin_importers`, the *From <label>* card | `src-tauri/src/plugins/importer.rs`, `src-tauri/src/commands/plugin_import.rs`, `photos-core/src/batches.rs` (one constant), `src/components/plugin-import-card/`, one insertion in `src/routes/import/gui.tsx` |
| **A4** MCP tools (Opus) | the tools adapter over A2's export, dynamic tool and prompt lists, naming, scope, the `photos` and `drafts` host traits implemented, `list_changed`, *Plugins add N tools* | `src-tauri/src/plugins/mcp_tools.rs`, `src-tauri/src/commands/plugin_tools.rs` (`plugin_tool_count`), `src-tauri/src/mcp/{tools.rs, mod.rs}`, `src/routes/settings/ai-assistants.tsx` |
| **C2** UI | Settings → Plugins (rows, switch, ⋯, Details, Settings form, Log, Developer mode, *Load a folder…*), the install sheet, undo toast, palette entries, `lib/menus/plugins.ts`, Licences' *Plugins* section, `/dev/ui` states, browser-shim mocks | `src/routes/settings/plugins.tsx` (+ `plugin-details.tsx`), `src/routes/settings/sections.tsx`, `src/components/plugin-install/`, `src/lib/menus/plugins.ts`, `src/lib/commands.ts` (palette entries), `src/lib/plugins.ts` (invoke wrappers), `src/hooks/use-plugins.ts`, `src/dev/browser-shim.ts` (plugin mocks), `src/components/licence-list/` |
| **D1** SDK and examples | the Rust guest crate `lumen-plugin` (its own Cargo workspace, outside `src-tauri`), `scripts/plugins/pack.mjs` (pack, sign, keygen, throwaway test key), `docs/PLUGINS.md` (author guide: kinds, manifest, permissions, licences, Developer mode), one small example per kind | `plugins/sdk/**`, `plugins/examples/**`, `scripts/plugins/pack.mjs` (+ `pack.test.mjs`), `docs/PLUGINS.md`, `.gitignore` (`plugins/**/target`), `scripts/dev/clean-build.ps1`, root `package.json` (devDependencies `@noble/hashes` and `fflate`, both MIT) |
| **D2** first real plugin (**Call**: Google Takeout importer; Julian may swap it) | `pick = "folder"`, *Choose the folder with your Takeout*; an unpacked Takeout: dates from `photoTakenTime`, places, titles, descriptions, albums from folders, the truncated `*.json` sidecar names, edited copies (`-edited`); items use the host's `path` fast path; tested on a synthetic Takeout in a temp folder | `plugins/first-party/takeout/**` |
| **D3** Takeout zips | the same folder holding `takeout-*.zip` files, read in the guest through `chosen.read` (central directory first, items streamed by `read`); a photo and its `.json` in different zips; within the 512 MB and 30 s limits per call | `plugins/first-party/takeout/**` (after D2) |
| **Final chat** | §12 amendments if the plans session hasn't applied them, merge, full checks, `26.2.0-alpha`, CHANGELOG, STATE.md *Plugins*, CLAUDE.md, build, local tag (Prompt 4) | — |

`scripts/dev/clean-build.ps1` also trims `plugins/**/target` (D1).

## §15 Left to the builder

- The exact WIT shapes.
- How wasm instances are pooled.
- The compile cache's eviction.
- Where preset string ids live.
- The short-name rule for collisions.
- The protocol route file's layout.
- The order of rows and controls inside the pages.
- Whether *Reload* also watches the dev folder.
