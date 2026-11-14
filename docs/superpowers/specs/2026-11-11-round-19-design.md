# Round 19 — design (Julian's list of 2026-11-11)

Eighteen items in three lanes (roadmap: `plans/2026-11-11-round-19-roadmap.md`). Each section is
self-contained: a chat reads only the sections its plan block names. **J** marks what Julian
decided on 2026-11-11; **Call** marks a choice made while writing this, open to his veto; **Builder**
marks what the implementing session decides. Line numbers are from `main` at `ae3e26a` and drift.

Not in scope: per-mask vignette, per-mask ML Denoise, a move-import mode, Lumen deleting any
source file, cloud anything.

---

## §1 Masks take every local Develop tool (lane A)

**J:** a mask gets everything that can be local; selecting a mask takes over the right panel.

### Data
`LocalAdjust` (`photos-core/src/edits.rs:379`) keeps its 13 flat fields and gains, each
`#[serde(default, skip_serializing_if = "is_default")]` so a recipe written before them writes back
byte for byte:

| Field | Type (existing structs) | Panel |
|---|---|---|
| `vibrance` | `f32` ±1 | Light/Colour |
| `curve` | `Curve` | Curve |
| `hsl` | `[Hsl; 8]` | HSL |
| `grading` | `Grading` | Colour grading |
| `sharpen` | `Option<LocalSharpen { radius, detail, masking }>`; amount stays `sharpness` | Detail |
| `grain`, `glow`, `halation`, `motion_blur` | `Grain`, `Glow`, `Halation`, `MotionBlur` | Effects |

Photo-only, never in a mask: crop, rotate, straighten, flip, vignette, ML Denoise, Auto, profile.
`LocalAdjust` stops being `Copy` (`Curve` holds `Vec`s). No migration; **Builder** bumps the recipe
version only if `RecipeWire` needs it.

### Render (`photos-media/src/develop/`)
Today (`mod.rs:256-272`): gains → texture/clarity → dehaze → tones → `masks::apply` (`masks.rs:147`,
local Basic mixed by weight) → curve → HSL + grading → sharpen → motion blur / glow / halation /
vignette / grain.

- **Per-pixel local tools** (vibrance, curve, HSL, grading) run inside the masks stage, after the
  mask's local Basic, in that order, then mix by the mask's weight. The global curve and colour
  stages still run afterwards on the whole frame. New code goes in a new file `develop/local.rs`
  that reuses the global stages' per-pixel functions (no second copy of the maths).
- **Area local tools**: sharpen uses the mask's own radius/detail/masking when `sharpen` is set
  (else as today). Glow, halation and motion blur at the effects stage: `mix(img, effect(img), w)`
  per mask that sets one. Grain: per-mask amount and size, mixed by weight.
- Mask rasters are built once per render and reused at both stages.

Constraints: (a) a mask whose new fields are default costs nothing extra and renders bit-identical
to today (existing goldens and `mask-golden.json` unchanged); (b) one mask using every per-pixel
tool adds ≤ 30 % to the live preview's time (measure with the existing preview timing, report the
number); (c) rayon, below-normal priority as the rest; (d) `develop-defaults.json` is regenerated
by its Rust test, never edited.

MCP: the `mask` tool's schema (`src-tauri/src/mcp/write.rs`) accepts the new fields.

### Panel — mask mode takeover
```
+--------------------------------+
|# MASK  Sky            [ Done ] |  pinned bar in the mask colour; name editable (double click)
|# Opacity ------o----  Invert   |  ⋯ menu: Reset mask adjustments, Duplicate, Delete
|# [brush][linear][radial][AI] ± |  the shapes block that exists today
|#-------------------------------|
|# v Light        •              |  • = this section changes something
|# > Colour  > Curve  > HSL      |
|# > Colour grading  > Detail    |
|# > Effects                     |
+--------------------------------+   # = 3 px edge in the mask colour down the panel
```
- Photo mode: the *Masks* section lists masks and *New mask* as today. Selecting a mask
  (`maskUi.selected`, `src/lib/develop-masks.ts:41`) enters mask mode: Crop and the photo's own
  sections are hidden; *Done*, `Esc` or deselecting returns. Selection, undo and history unchanged.
- **Reuse, don't fork:** the panels take `{ session: DevelopSession }` and read
  `session.develop.<section>` / write via `session.apply` (`src/hooks/use-develop-session.ts:100`).
  Build `maskSession(session, maskId)` — an adapter whose `develop.*` is the mask's `adjust` and
  whose `apply` writes into that mask — and pass it to the existing `BasicPanel`, `CurvePanel`,
  `HslPanel`, `GradingPanel`, `DetailPanel`, `EffectsPanel` with `scope="mask"`, which hides
  photo-only controls (Auto, WB presets/picker if they read the photo, vignette, Denoise, profile).
- Colour: the panel root gets `data-mask-mode`; one token for the mask accent in
  `src/styles/globals.scss` (reuse the overlay colour's token if one exists) drives the bar, the
  edge and the slider fill through a CSS variable set on the root. No theme branches in components.
- The old 13-slider table (`develop-masks/gui.tsx:43-55`, `:382-391`) is removed; its values now
  live in the Light/Colour/Detail sections.

## §2 Submenu opens to the left (lane A)
`src/routes/develop/top-bar.tsx:118` hard-codes `<Menu.Sub position="left-start">`, so *Select by
label* covers the presets column. Use `right-start` (Mantine flips it when there is no room).
It is the only `Menu.Sub position=` in `src/`.

## §3 Keybinds: more defaults + rebinding (lane C)
**J:** both. Today `src/lib/keybinds.ts` is a read-only list for Settings → Keyboard; palette
commands carry `shortcut` (`src/lib/commands.ts`) dispatched by `src/hooks/use-hotkeys.ts`; each
screen handles its own keys by hand.

- **Registry:** every key gets a stable id (`grid.open`, `viewer.zoom-1-1`, command ids for palette
  commands), default keys, a scope (`everywhere`, `grid`, `viewer`, `develop`, `slideshow`, …).
  `keysFor(id)` returns the person's override or the default; `matches(event, id)` is what handlers
  call. `appCommands[].shortcut` stays the default for its command id.
- **Overrides** persist in the app settings JSON as `keybinds: { [id]: string[] }` (empty array =
  unbound). No DB, no migration.
- **Settings → Keyboard:** search box; each row shows its keys as caps; click → *Press keys…*
  (`Esc` cancels, `Backspace` clears); a clash in the same scope or with `everywhere` shows
  *Already used by X — Replace / Cancel*; per-row reset and *Reset all*. Single letters and digits
  never fire while typing (`src/lib/typing.ts`).
- **More defaults:** **Builder** lists palette actions and screen actions without a key and gives
  each a free default, following Lightroom where it is free (e.g. `G` Library, `D` Develop, `\`
  before/after, `R` crop, `K` brush, `O` mask overlay, `[` `]` brush size, `Mod+Shift+C/V`
  copy/paste settings, `Mod+N` new album). The table goes in the plan's *As built*; no approval
  gate, since every key is changeable.
- Develop's own handlers move to the registry in the final chat, after lane A merges.

## §4 Albums say which collection they are in (lane B)
One helper in `src/lib/album-collections.ts`: `albumPath(album) → ["Trips", "Europe"]`.
Rich contexts show the name with a dimmed second part, *Italy 2024 · in Trips › Europe*; plain
text contexts show `Italy 2024 (Trips › Europe)`. Only for albums inside a collection.
Used by **every** place that lists albums as a target or result: `album-picker`,
`collection-picker`, the *Add to album* menus (`src/lib/menus/`, `use-library-menus.tsx`,
`menus/connected.ts:49`), palette entries, the import page's album chooser, search chips and
suggestions (§7), the Albums-page search results (§9). Grep `albums` in `src/lib/menus` and
`src/components/*picker*` to find stragglers.

## §5 Add photos = the Library in pick mode (lane B)
**J:** Library page + extra button + a source tree like a file dialog.
```
+-----------+------------------------------------+
| Library   | Adding to: Italy 2024               |
| Favorites | [ search, tags, filters........ ]  |
| Albums    |------------------------------------|
|  v Trips  | [x][ ][x][ ][ ][x][ ][ ][ ][ ]     |
|    Italy  | [ ][ ][ ][x][ ][ ][ ][ ][ ][ ]     |
|  > Family | [ ][x][ ][ ][ ][ ][ ][ ][ ][ ]     |
| People    |------------------------------------|
| Places    |        [ Cancel ] [ Add 5 photos ] |
+-----------+------------------------------------+
```
- *Add photos* on an album (today `photo-picker` from `src/routes/albums/detail.tsx`) opens the
  real Library route in pick mode (`/library?pick=album:<id>`): same grid, search, tags and filters.
- A pinned bar: *Adding to: <album + collection path>*, *Cancel*, *Add N photos*. Clicking a tile
  toggles it; photos already in the album show a check and *In album*, and cannot be toggled.
- Left tree: Library, Favorites, collections › albums (nested), People, Pets, Places — only scopes
  that already exist as grid scopes (**Call**: no disk-folder node; Lumen has no folder browser).
  Choosing a node changes the grid's scope; the selection survives scope changes.
- Viewer, Develop, delete and drag are off in pick mode; `Esc` cancels; `Enter` adds.
- `photo-picker`'s other users (slideshows, Develop) keep it. **Builder**: switch them only if it
  is a few lines.

## §6 Search understands or / and / not (lane B)
**J:** plain lower-case words are operators; quotes make a phrase literal.

Grammar (precedence `not` > `and` > `or`; juxtaposition is `and`, as today):
```
expr := and ("or" and)*      and := unary (("and")? unary)*
unary := ("not" | "-")? atom atom := "(" expr ")" | "quoted phrase" | segment
```
- `or` / `and` / `not` count only as whole words between terms, any case; at the start or end of
  the text, or doubled, they are plain text. `-word` is `not word`.
- A **segment** is the text between operators and is parsed by today's `query_parse::parse`
  (`photos-core/src/query_parse.rs:701`) unchanged, so a leaf is today's `Parsed`: structured chips
  plus `rest` for CLIP (`src-tauri/src/search.rs:85-102`).
- Wire: the query gains an optional expression tree (`And` / `Or` / `Not` / `Leaf`); absent means
  today's behaviour and today's code path, untouched and as fast. Limits: depth ≤ 8, leaves ≤ 16.
  New parser code lives in a new file `photos-core/src/query_expr.rs`.
- Evaluation: a structured leaf compiles to the existing WHERE fragment; a leaf with `rest`
  resolves its CLIP hits to an id set first; fragments combine with AND / OR / NOT. A negated
  semantic leaf excludes that leaf's hits. Order: best positive semantic score when any positive
  leaf is semantic, else today's date order. Keyset pagination holds; no `OFFSET`; no SQL
  `GROUP BY` on computed keys.
- UI: operators and brackets appear as small connective chips between the term chips
  (`ChipKind::Op`), so the person sees how Lumen read the sentence. *Try* starters gain one example.
- Stretch (do only if `tag_search.rs` has one predicate builder to extend): a tag chip can be
  excluded (*Exclude* in its menu, shown as *not …*).

## §7 Search by album and collection (lane B)
`TagValue::Album { id }` (`tag_search.rs:46`) and `ChipKind::Album` exist; the vocabulary lists
manual albums (`query_parse.rs:97`).
- Album suggestions and chips carry the collection path (§4), so two *Italy* albums are told apart.
- New `Collection { id }` tag/chip: photos in any album of that collection, nested ones included
  (`album_collections`, migration 0026; `albums.collection_id`).
- Sentence forms: `album:italy`, `in italy album`, `collection:trips`; both work inside §6
  expressions and as smart-album rules (§8).

## §8 Smart album editor (lane B)
**J:** everything the Library search has; bigger, cleaner, more polished.
Today: `src/components/smart-album-editor/gui.tsx`, a `size="md"` modal with a text box and
`FilterChips`.
```
+--------------------------------------------------------------------+
| New smart album                                                  ✕ |
| Name [ Best of Italy                     ]                         |
| +- Rules -------------------------+ +- 248 items match -----------+ |
| | [ search sentence + chips ....] | | [ ][ ][ ][ ][ ]             | |
| | Try: beach or mountains not 2023| | [ ][ ][ ][ ][ ]             | |
| | Filters                         | | [ ][ ][ ][ ][ ]             | |
| |  Kind    [Photos][Videos][RAW]  | |  live preview, newest first | |
| |  Rating  [★3+]   Flag [Picked]  | |  (first ~60, not a grid     | |
| |  People [+] Places [+] Albums[+]| |   route: plain thumbnails)  | |
| |  Dates  [from] – [to]           | |                             | |
| +---------------------------------+ +-----------------------------+ |
|                                      [ Cancel ]  [ Create album ]  |
+--------------------------------------------------------------------+
```
- Size `min(1040px, 92vw) × min(720px, 86vh)`; one column under 760 px wide.
- The rules box **is** the Library's search box component (sentence, chips, suggestions, §6, §7)
  plus the Library header's filter/tag controls; no second rule vocabulary. The saved rule is the
  same query object the Library uses (with §6's tree when present).
- Live count and preview update as rules change (debounced); empty state: one sentence.
- Tokens only; no new colours. Run the `frontend-design` skill's judgement on spacing and
  hierarchy, not a new visual language.

## §9 Search on the Albums page + the "6.7." jump (lanes B and C)
**J:** he typed `6.7.` in the top bar's search on the Albums page and landed on another screen.
- **Bug (lane C, chat C1):** find the root cause before fixing (`systematic-debugging`). Leads:
  digits are navigation keys (`commands.ts`: `6` Slideshows, `7` People, `8` Develop) guarded by
  `isEditable` (`use-hotkeys.ts:17-41`); typing may move the route or the focus (a date-like text
  being parsed, a results popover stealing focus), after which the next digit is a hotkey.
  Reproduce in the real WebView or `yarn dev` + shim, fix at the cause, one regression test.
- **Feature (lane B):** while the route is `/albums`, the top search filters albums and
  collections by name as the person types (case- and accent-insensitive, word-prefix match),
  each result with its collection path (§4) and cover; `Enter` opens the first; a last row
  *Search photos for "…"* runs the normal photo search. Empty result: one sentence + that row.

## §10 A collection's photos, by album (lane B)
A *All photos* button on a collection page opens one grid of every photo in the collection's
albums (nested included), in sections headed by album name (with path when nested) and count, in
the collection's album order. A photo in two albums appears in both sections, so tile identity is
(section, asset). Rust builds the sections (the layout's summary defines them; never draw a row
whose height disagrees with its summary). Clicking a header opens that album. Viewer works across
sections in order.

## §11 Build folders: 745 GB → about 25 GB a checkout (lane C)
Measured 2026-11-11: `src-tauri/target` 539 GB (`debug/incremental` 250, `debug/deps` 213,
`agent/` 42, `release` 19) + 205 GB across eight merged worktrees. It is all compiler output;
dev models live in app data, sidecars in `src-tauri/binaries`.
**J:** delete all, keep slim.
- `cargo clean` in the main checkout and (`--manifest-path`) in each old worktree. Only ever
  `cargo clean`; never a recursive delete by hand.
- Codex's MCP entry runs `src-tauri/target/debug/lumen.exe`: rebuild debug right after.
- Dev profile: `debug = "line-tables-only"` for our crates, `debug = false` for dependencies
  (`[profile.dev.package."*"]`), same for `test`. Keep the existing `opt-level` lines.
- `scripts/dev/clean-build.ps1`: removes `target/debug/incremental` and artefacts older than
  14 days (`cargo sweep` when installed, else `cargo clean -p` the workspace crates), prints
  before/after sizes. One line in CLAUDE.md *Commands* (final chat).
- Report sizes after one full debug build + `cargo test --workspace`. Target ≤ 30 GB.

## §12 Renaming the root folder to `Lumen` (lane C prepares; Julian renames)
The rename happens **after** the final chat (worktrees removed, build folders clean).
What holds the old path:
- In the repo: `scripts/mcp-check/launch.ps1:7`, `scripts/ml/people_quality.py:19,116` (make both
  relative to the script), a comment in `scripts/release/collect.mjs:16`.
- Outside: `~/.claude.json` (project entry), `~/.claude/settings.json` (two permission notes),
  `~/.codex/config.toml:165` (Lumen MCP command), CPM's `projectData.json` under
  `%APPDATA%/JL-Studios/CPM (Coding Project Manager)/` and `… Dev/`, Claude's project folder
  `~/.claude/projects/X--Documents-Coding-Projects-Desktop-Apps-Tauri-Photos-App/` (memory lives
  there; the new name is `…-Tauri-Lumen`), `graphify-out/`, the Python venvs `.venv-ml` /
  `.venv-train` (their launchers embed the path; `python -m pip` keeps working), and `target/`.
- Deliver `scripts/dev/after-rename.ps1`: dry run by default (prints every file and line it would
  change), `-Apply` backs each file up as `*.before-rename`, rewrites the path, copies the Claude
  project folder to its new name, then runs `git worktree prune`, `yarn typecheck`, `cargo check`
  and `graphify update .`. It refuses to run while CPM or Lumen is open. The session runs only the
  dry run. Write the rename steps for Julian in the roadmap's *After the round*.

## §13 Nothing is lost on import (lane C)
**J:** he will delete source copies (folders on disks, camera cards; iPhone photos came straight
from the phone, so that source is gone already). Every part of every media file and each special
feature must be copied and read. A verify button + report, kept in Settings as a mostly temporary
tool. Lumen never deletes a source.

Known today: the importer hashes while staging, `sync_all`s, journals, then renames
(`photos-import/src/ingest.rs:365-382`, `:437-451`; recovery `recover.rs:25`); it does **not**
re-read the destination. Only media files are enumerated (`photos-media/src/kind.rs:16-75`,
`source.rs:117,147,179`); `.xmp`, `.aae`, `.thm` are dropped silently; Apple bursts and
not-downloaded items are counted but not listed (`apple/source.rs:102-114`). Originals are copied
byte for byte, so embedded metadata is never lost; what is not *read* can be re-read later.

1. **Audit** → `docs/IMPORT-AUDIT.md`: one table, rows = JPEG, HEIC/HEIF (gain map, depth, aux
   images), Live Photo pairs, ProRAW/DNG, NEF and other RAW, RAW+JPEG pairs, PNG/GIF/WebP/AVIF/JXL,
   MOV/MP4 (QuickTime `Keys`/`UserData` GPS, creation date with zone, rotation, HDR/Dolby Vision,
   slow-mo, time-lapse, cinematic, spatial), MTS, bursts, panoramas, sidecars (`.xmp`, `.aae` and
   the `IMG_E…` edited render, `.thm`). Columns: copied? / read into the library (which fields) /
   lost today / fix. Sources: folder, device (iPhone, card), Apple Photos, Lightroom.
   Fields to check per container: capture date (zone, sub-seconds), GPS (altitude, direction),
   orientation/rotation, dimensions, duration/fps/codec, HDR, make/model/lens, ISO/aperture/
   shutter/focal length, Live Photo content identifier, burst id, title/caption/keywords/rating/
   label, file created/modified times.
2. **Verify after copy:** before the journal commits, re-read the staged file and compare its
   BLAKE3 with the source's; a mismatch fails that file (kept in the report, source untouched).
   **Builder**: an unbuffered re-read on Windows if it is simple, else a plain re-read; say which.
3. **Sidecars are kept:** `.xmp`, `.aae`, `.thm` beside a media file with the same stem are copied
   next to its original and recorded (`asset_sidecars`); XMP rating, label, keywords, title and
   caption go through `extras::apply` (`photos-core/src/extras.rs`). Lightroom `crs:` develop
   settings in XMP are **not** applied (follow-up).
4. **Nothing is skipped silently:** every file a source holds that was not imported is recorded
   (`import_skips`: path, reason — unsupported type, duplicate of asset N, unreadable, burst frame,
   not downloaded, failed verify) and listed in the import summary with *Show in folder*.
5. **Fix the read gaps** the audit finds (each with one fixture test built in a temp dir with the
   bundled ExifTool/FFmpeg), then a *Re-read metadata* pass for existing assets on the Maintenance
   lane: idempotent, derived data only, replayed on a library **copy** first.
6. **Settings → Library → *Check before deleting***, two read-only tools on a job lane with the
   activity ring, cancellable:
   - *Check a folder or card*: choose a folder; every file is hashed and looked up by
     `assets.content_hash`, and the library's copy is re-hashed too. Result: **Safe** (in the
     library, both copies identical), **Not in the library** (with the reason when known),
     **Other files** (not media, not sidecars). One verdict line; *Save report…* (text, through
     `lib/export-folder.ts`).
   - *Check the library*: re-hash every original against `content_hash`; lists missing or changed
     files (none expected) and the date of the last full check. This is the proof for sources that
     no longer exist.
Migration **0037**: `asset_sidecars`, `import_skips`.

## §14 More cover icons (lane B)
`src/components/icon-cover/icons.ts` + `cover-editor`: add about 80 icons from the icon library
already in use (no new dependency) across travel, nature, family, food, sport, events, animals,
seasons, work, hobbies; the editor gets category chips and a search field. Stored ids of existing
icons must not change.

## §15 MCP: rating, label, flag; names in Develop (lane A)
- New MCP tool `mark { photo, rating?: 0–5, label?: none|red|yellow|green|blue|purple, flag?:
  none|pick|reject }`, filmstrip photos only (the check behind `mcp/scope.rs:33`), through
  `assets::set_rating` (`photos-core/src/assets.rs:781`) and `labels::set_color_label` /
  `set_flag` (`labels.rs:42,47`), emitting the same event the UI's own marks emit.
  `list_photos` returns each photo's `filename`, rating, label and flag.
- This is the first MCP write outside drafts: update the server instructions, the `edit_photo`
  prompt, Settings → *AI assistants* wording (*drafts, ratings, labels and flags*) and the docs.
- Names in Develop: the top bar shows the current photo's `filename` and *3 of 12*; filmstrip
  tiles show the name in a tooltip (`src/components/filmstrip/gui.tsx:94` has it as `aria-label`).

## §16 NEFs at full resolution (lane A)
**J:** just check. An ignored test taking `LUMEN_NEF=<a copy>` prints, for one standard NEF and one
HE NEF: the camera's pixel count (ExifTool), `decode_linear`'s size
(`photos-raw/src/lib.rs:232`, `photos-media/src/develop/raw.rs:82-92`), Develop's base
(`mod.rs:176 load_base_as`, `max_edge`), an *Original size* export, and the stand-in paths
(`stand_in.rs:85`: converter DNG, camera JPEG). A few pixels of sensor border are expected; fix
anything smaller and record the table in the plan. Never write next to the NEF.

## §17 Badges on thin portrait tiles (lane B)
`src/components/photo-tile/` (`styles.scss`): badges disappear on narrow tiles. Make them step
down instead: full label → compact (smaller type and padding) → a dot or glyph, by the tile's own
width (container query), in priority RAW > video length > Live > the rest; drop the lowest first.
`[data-compact]` (year view) stays badge-free. CSS only; no per-tile animation.

## §18 Remove merged worktrees (final chat)
For each worktree under `.claude/worktrees/`: if its branch is in `git branch --merged main` and
`git status --porcelain` there is empty → `git worktree remove` and `git branch -d`. Anything with
uncommitted or unmerged work is listed for Julian and left alone. Never `--force`.
