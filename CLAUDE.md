# Lumen — CLAUDE.md

Lumen is a local-first photo and video library and RAW developer for macOS, Linux and Windows by JL Studios. Tauri v2, Rust workspace backend, React 19 + TypeScript + Vite + Mantine + SCSS + Framer Motion frontend. No cloud, no account, no telemetry.

## ⚠ A user's library is irreplaceable

A user's library and any connected phone, card or disk are irreplaceable. Never use them as a destination, never delete, move, rename, overwrite or re-encode anything under them, and never write to `library.db` directly. Run replays (`LUMEN_DB`, `LUMEN_REDATE_LIBRARY`, …) on a **copy**.

- A migration/backfill that rewrites rows is tested on a copy first, is idempotent, and changes only derived data, never `originals/`.
- Tests and synthetic libraries go in temp dirs. Never point `LUMEN_E2E_FOLDER`, import or export at a real library or a device.
- Devices are read-only sources. When in doubt, stop and ask.
- Personal paths and notes live in `CLAUDE.local.md` (git-ignored).

## Read these first

- `docs/superpowers/specs/2026-09-17-photos-app-design.md` — architectural spine (process model, transport, crates, schema, pipeline, roadmap). **Load-bearing**; a decision contradicting its §3 changes that document first, code second.
- `docs/superpowers/specs/2026-09-17-lumen-ux-design.md` — product and UX design.
- `docs/superpowers/plans/` — per-phase plans (most with an *As built* section). Work from the current phase's plan.
- **`docs/STATE.md` — as-built notes per area** (import, devices, Apple sources, edits, memories, slideshows, music, ML, people, labels, export, Develop, masks, search, map, dates, orientation, grid, FFmpeg/ExifTool, protocol routes, events, real-data replay commands). Read the section for the area you touch.
- `docs/VISION.md` — product vision (local-first, universal import/migration, free and open source).

## Current state (summary; details in `docs/STATE.md`)

Lumen is free GPL software (`docs/superpowers/plans/2026-11-14-open-source.md`): no Pro, no Cloud. Migrations 0001–0039 are taken; the next is **0040**. New work branches from `main`. Nothing from later phases may be started early; nothing before Phase 7 may depend on `photos-ml`. One line per round; the as-built notes are in STATE.md.

- Phases 0–8e and Round 4 (A–K, pets, duplicates, people recall) are built; roadmap `docs/superpowers/plans/2026-09-25-round-4-roadmap.md`.
- **Going public**: GPL-3.0-or-later, DCO sign-off, community files, CI scrub and licence checks, releases built from `v*` tags by `.github/workflows/release.yml` into the public `05Narwhal/lumen`; `node scripts/oss/export-public.mjs <new empty folder>` makes the scrubbed one-commit snapshot (`docs/RELEASING.md`, `CONTRIBUTING.md`).
- **Round 23** (plugins and themes, `26.2.0-alpha`): `photos-plugins` (package, signature, store, licence check, wasmtime host with 512 MB / 30 s / 3 traps and no filesystem, network or process), plugin themes, Settings → Plugins, the `lumen-plugin` SDK, `pack.mjs`, six examples, `docs/PLUGINS.md`, the first-party Takeout plugin.
- **Round 22**: oversized images from a reduced decode, Repair (recover what's readable), *Edit in <app>* and *Open with*, Windows Store apps as *Open with* targets.
- **Round 21**: Develop mask menu and layer order, area tools, *Import · Repair*, *Restore…* for files Lumen can't open, loading-screen and wake fixes.
- **Round 20**: a search is a tag sequence with `and` / `or` / `not`, *Add to album* as a tree, tabbed Memories and People & Pets pages, a new Home.
- **Round 19**: masks take every local Develop tool, boolean search, album collections and the smart album editor, rebindable shortcuts, verified import with sidecars and a skip list, *Check a folder / the library*.
- **Exact place names**: opt-in OpenStreetMap Nominatim lookup of a photo's coordinates (Settings → Connections → Map).
- **Phase 8f**: AI masks (Sky, Subject, Background, Person *n*) from Lumen Facet behind an Experimental switch.
- **Rounds 8–10**: faster pet scan, videos remuxed and prepared in the background, the face radar, typed dates, slideshow export.
- **Round 11–12, 14**: Lightroom presets and catalogs, Retouch and Auto models behind Experimental switches (Ray, Prism, Spectrum).
- **Round 13, 18 C2**: text in photos (OCR, PP-OCRv5) and search by what photos say.
- **Round 15**: stability (stall watchdog, Denoise thread, recovery, window drag).
- **Round 16, 18 D**: an opt-in local MCP server so an AI assistant can retouch photos in Develop's filmstrip, drafts only.
- **Round 17–18**: versions and updates (`26.X.Y-<channel>`), the installer *Lumen Setup*, RAW formats through Adobe DNG Converter or the camera JPEG, `photos-raw` loaded at run time, an opt-in Discord status, Settings in five groups, the redesigned Import page, files Lumen can't open (`assets.unreadable`).
- **First release `26.1.0-alpha`** and the release pipeline: `docs/RELEASING.md`, `docs/CHANGELOG.md`.

## Cross-cutting gotchas

- **Commands are `#[tauri::command(async)]`** — a sync command runs on the main thread and freezes the UI. Job workers and rayon run below normal priority.
- **Shell heredocs collapse backslashes** — write Rust/TS containing `\` with Write/Edit.
- `photos-ml` (ort, static) needs **MSVC 14.44+**.
- Missing FFmpeg / ExifTool / models is a **state, never an error**; every ML feature falls back to its pre-ML behaviour.
- Anything painting a photo decodes `derivatives::decode_display_full` (never the original path: `image::open` ignores the orientation tag).
- **Develop writes `develop_drafts`, never `edits`** (Round 6). The library, the viewer and thumbnails show `edits` only; exports render `edits::export_recipe` (draft first). Every export asks for a folder (`lib/export-folder.ts`).
- Dates: `choose_capture` priority order — never put EXIF `DateTime` before `DateTimeDigitized`. `captured_at` is wall-clock time.
- `ANALYZE` belongs on the maintenance lane, never in a migration. Group in Rust, not SQL `GROUP BY` on computed keys.
- Grid: never draw a row whose height disagrees with its summary; everything a tile receives must keep identity across renders.
- Never style a row's descendants by tag. Motion exit animations never finish in jsdom (`MotionGlobalConfig.skipAnimations = true`).
- Golden files keep Rust and TS maths in lockstep (`slideshow-golden.json`, `mask-golden.json`, `develop-defaults.json`) — regenerated by Rust tests, never edited by hand.
- Every dependency, model and API must allow commercial use (`docs/LICENSING-AUDIT.md`, `docs/THIRD-PARTY-MODELS.md`); Settings → *Licences* shows `src/assets/licenses/notices.json`, so **after changing dependencies run `yarn run licenses`** (plain `yarn licenses` is a Yarn built-in) or `src/lib/licenses.test.ts` fails.
- Vite watches an **allowlist** (`watchedTop` in `vite.config.ts`: `src/`, `index.html`, configs). Windows opens one blocking `fs.watch` per file, so a venv or worktree in the root once made `tauri dev` take 5 min; a new top-level folder the app needs (e.g. `public/`) goes in that list.
- Plugin wasm fixtures (`src-tauri/crates/photos-plugins/tests/wasm/`) are checked in and rebuilt by `node scripts/plugins/build-fixtures.mjs` (needs `wasm32-wasip2` and `wasm-tools`); a hash test fails with *run build-fixtures* when the WIT changes.
- Flaky under full-suite load: `src/routes/slideshows/gui.test.tsx` and a context-menu test (5 s timeout); a rerun passes; in `cargo test --workspace`, `plugins::state::tests::boot_budget` (20 ms median) and `plugins::importer::tests::trap_mid_import` fail now and then and pass alone (`cargo test -p lumen --lib plugins::`). A clean checkout also needs the git-ignored FFmpeg sidecars (and ExifTool on Windows) before `cargo test` compiles: `scripts/fetch-ffmpeg.*`, `scripts/fetch-exiftool.*`.

## Commands

```sh
yarn install                 # frontend deps
yarn tauri dev               # run the app (Vite on :1420 + Rust)
yarn tauri build             # release bundle
yarn typecheck               # tsc -b
yarn test                    # vitest (jsdom, Testing Library; Tauri invoke is mocked in src/test/)
cargo test --workspace       # from src-tauri/; also regenerates ts-rs types
cargo clippy --workspace --all-targets -- -D warnings
pwsh scripts/dev/clean-build.ps1   # trims src-tauri/target (debug/incremental and old artefacts); a full wipe is `cargo clean`
```

Windows is the primary dev machine; shell is PowerShell. Use forward slashes and quote paths (the project folder has a space). `yarn dev` + `src/dev/browser-shim.ts` runs the UI in a browser (`?library=1&nosplash=1&theme=glass`); `/dev/ui` shows every control and modal.

## Non-negotiables

1. **Two transports.** Metadata over `invoke` (JSON, pages of ~200). Bytes over `photos://` with `Range`/`206`. Never base64 an image or send bytes through `invoke`.
2. **One-way crate deps:** `src (tauri bin) → photos-import → photos-media → photos-core`; `photos-jobs → photos-core`; `photos-ml → photos-core`; `photos-ml → photos-models`; `src → photos-models`; `installer → photos-models`; `src → photos-plugins → photos-core`. `photos-core` and `photos-models` depend on no workspace crate. Only the two binaries (Lumen and Lumen Setup in `installer/`) depend on `tauri`. `photos-raw` is loaded at run time, depends on no workspace crate and nothing depends on it; it is the only crate that uses `rawler`.
3. **Rust does the work** (layout, hashing, decoding, resizing, clustering, search, export). The frontend never reads a file path or decodes an image.
4. **Commands are thin adapters** in `src-tauri/src/commands/<domain>.rs`, returning `Result<T, AppError>` (`{ code, message, detail? }`, stable `code`).
5. **TS types are generated** by `ts-rs` into `src/types/generated/`. Never hand-edit.
6. **Keyset pagination only.** No `OFFSET`; lists return `{ items, next_cursor }`.
7. **No colour outside `src/styles/globals.scss`** and its theme partials. Colours, radii, shadows, durations, z-indexes are tokens. Component styles never branch on theme. The one source of colour outside `globals.scss` is plugin themes: typed tokens checked and turned into CSS in Rust (`photos-plugins::theme`).
8. **Folder convention:** each component/route is a folder with `gui.tsx`, `styles.scss`, `index.ts`; `@/` imports; no importing another folder's stylesheet.
9. **No per-tile animation in the grid** (CSS opacity on load only).
10. **App data in `config_dir/JL-Studios/Lumen/`**, never Tauri's `app_config_dir`/`app_data_dir`. Bundle id `com.jlstudios.lumen`.
11. **Originals are never modified.** Import copies; edits are recipes; deletion goes to a 30-day trash.

## UX rules (UX spec is authoritative)

- Four built-in themes (`gallery`, `darkroom`, `paper`, `glass`), plus plugin themes that override the same tokens, × light/dark via `data-theme` + Mantine's scheme attribute; themes override only surfaces, foregrounds, accent, borders, radii, tile gap, shadows, blur, heading font.
- Bundled fonts (Inter Variable; Fraunces for Paper headings). Custom 38 px title bar everywhere.
- Every screen has an empty state (one sentence, one action). No full-page spinners; background work goes through the activity ring.
- Reversible actions never confirm; 6 s undo toast. Only *Delete now*, *Empty trash*, *Delete library* and deleting a non-empty album confirm.
- The viewer is an overlay (`?asset=<id>`), never a route.
- `Ctrl/Cmd K` palette, `1`–`8` navigation, `/` search, `Space` quick look. Every action has a palette entry; every menu comes from a factory in `src/lib/menus/`.

## Workflow

- **TDD** for Rust crates and frontend hooks/components.
- **Verify before claiming done**: run the relevant tests and typecheck, report results plainly.
- **Plans over improvisation**: follow the phase plan; if it is wrong, fix the plan first.
- **Session budget: ~70k tokens per session.** Every new plan is split into phases that each fit one session (≤ ~70k tokens of context, including reading). A phase plan names the files it reads and owns, so a session reads CLAUDE.md, the spec, its phase plan and only those files. Near 70k, finish the current step, commit, tick the plan's checkboxes, add a one-line hand-off (done / next / surprises) to the plan or roadmap, and stop so Julian can clear the session. Parallel sessions own disjoint files (the roadmap's lanes).
- Cross-platform is not a phase: paths, title bar, protocol and sidecars are written for all three OSes, tested on Windows before merge.
- Performance budget (architecture spec §15) is checked with benchmarks as you go.
- Commit messages: imperative one-line summary, body explaining why when not obvious.
- Product priorities when trading off: data safety → library integrity → cross-platform → large-library performance → non-destructive editing → import reliability → UX → advanced editing → cloud.

## Repo layout

```
src/                       React app (UX spec §12)
src-tauri/crates/          photos-core, photos-media, photos-import, photos-jobs, photos-ml
src-tauri/src/             Tauri binary: commands/, protocol, events
src-tauri/binaries/        ffmpeg / ffprobe / exiftool sidecars (git-ignored)
docs/superpowers/{specs,plans}
```

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
