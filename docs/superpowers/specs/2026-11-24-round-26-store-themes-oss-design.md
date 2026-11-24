# Round 26 — Store redesign, themes as their own format, bug fixes, open-source prep

Julian's list of 2026-11-24 (after Round 25), decided in chat the same day. Version stays
**26.2.0-alpha** until the first public release. Plan: `docs/superpowers/plans/2026-11-24-round-26-plan.md`.

## Why

Round 25 built plugin pages, Discover sections, contribution slots and proposals, but in the
real app Julian saw a store that "looks ass": no icons, no hero, duplicated sections, cramped
cards. Most of that was never visible because the live store is still the Round 24 GitHub index
(no icons, no `featured`, Wayfinder 1.0.0 / API 1.1 with no map button) and the repo's first-party
manifests need `lumen >=26.4.0`. So this round has to (1) make the real thing visible in a dev
build without publishing, (2) redesign the store so it looks professional, (3) make themes a
separate, lighter thing than plugins, (4) fix the bugs he hit, and (5) get the repo ready to go
public so he can make the first real release.

## Goals

1. **Bugs**
   - **B1 Overlap**: the viewer's quick-edit bar (`src/components/viewer/edit-toolbar.tsx`:
     undo / redo, Cancel, Done) sits under the window controls and the title-bar activity ring.
     It must never overlap them, on every OS and window width.
   - **B2 Develop spinner**: opening photos with edits in Develop, the bottom-right spinner spins
     forever and the image never shows. Root cause first (systematic debugging), with a
     regression test. Suspect: Round 24's render cancellation (a cancelled render that never
     resolves the pending state), but this is not established.
   - **B3 Open in Map**: the info panel's *Open in Map* (`src/components/viewer/info-panel.tsx`,
     links to bare `/map`) must open the map, fly smoothly to the photo and mark it with a pin.
2. **Wayfinder on the map** comes only through the plugin system (`[[ui.contribute]]` slot
   `map.toolbar`, already built in Round 25 P4a / P4b-1). Nothing Wayfinder-specific in `src/`.
   What's missing is getting the new package into the app (goal 6) and checking it works.
3. **Store redesign — direction B (tiles)**, approved from the mockup
   (`scratchpad/companion/…/store-with-themes.html`, recreated from this text if lost):
   - Page header: title (*Plugins* / *Themes*), segmented control **Plugins · Themes · Installed
     · Upload** with a sliding pill.
   - Search + kind chips (Plugins: All, Tools, Looks, Importers, Apps, Providers; Themes: All,
     Dark, Light, Colourful, Layout).
   - **Hero carousel**: the index's featured items (plugins: `featured`; themes: `featuredTheme`),
     text left (eyebrow, icon + name, summary, Get / Learn more), screenshot right with a slight
     3-D tilt that straightens on hover, dots, auto-advance that pauses on hover and honours
     reduced motion.
   - Sections with *See all ›* (a See-all view is the flat grid of that section).
   - **Plugin tile**: every tile the same size (fixed height, 4 columns, fewer on narrow
     windows); screenshot thumbnail on top (gradient from the icon's colours when none), icon
     overlapping the thumbnail's bottom edge, name + *JL* badge, author, 2-line summary clamp,
     footer with kind and Get / Open / Update. Hover: lift, shadow, accent border, thumbnail
     zoom. Load: staggered rise.
   - **Theme tile**: a live miniature Lumen window painted from the theme's own tokens (sidebar,
     title, tile grid), light at rest and dark on hover (cross-fade), a *Layout* badge when the
     theme has CSS, three colour dots, **Try** (live preview without installing, revert on leave
     or after 30 s unless kept) and **Get**.
   - The **plugin page** (`/plugins/:id`) and a new **theme page** in the same language: large
     icon header, screenshot carousel with lightbox, README, permissions (plugins only), facts.
   - Remove the duplication Julian saw: a plugin appears once per screen except in the hero.
   - Tokens only, all four built-in themes × light / dark, Framer Motion for motion,
     reduced-motion respected, empty states per section.
4. **Themes are not plugins**
   - New file format **`.lumentheme`** (zip): `theme.toml` (id, name, author, version, `lumen`
     range, base theme, light / dark token tables, optional `[listing]`), optional `theme.css`,
     optional `fonts/*.woff2`, optional `icon.png`, `README.md`, `screenshots/`.
   - **No code, no permissions**, no permission list in the install sheet, no signature required
     (store themes are still checked against the index's blake3).
   - **Checked, not trusted**: Rust validates tokens (existing typed-token + contrast checks in
     `photos-plugins::theme`) and parses `theme.css` with a CSS parser, allowing only
     selectors on Lumen's published **part hooks** (`[data-part="sidebar"]`, `toolbar`,
     `titlebar`, `grid`, `tile`, `viewer`, `panel`, `card`, `button`, … — the list is the
     contract, documented) and an allowlist of properties (layout, box model, position within the
     app body, radius, borders, shadows, filters, transforms, transitions / animations,
     backgrounds without `url()` except the theme's own fonts / images). Refused: any remote or
     `data:` `url()`, `@import`, `content:` text, selectors reaching dialogs / confirmations /
     the undo toast / window controls, `display:none` / `visibility` / `opacity:0` /
     off-screen on protected parts, `pointer-events` on protected parts, `position:fixed` over
     the title bar. Refusal names the line and the rule.
   - Installed themes live apart from plugins (`themes/installed/` under app data), are listed in
     **Settings → Appearance** (not Plugins), and are picked there like the built-in four.
   - Plugins can no longer carry themes: `[[themes]]` leaves the plugin manifest; the existing
     example theme plugin(s) are converted to `.lumentheme`. Nothing is public yet, so no
     compatibility path.
   - The store index gains a `themes[]` list (same fields as a plugin entry minus permissions /
     kinds, plus `dark` / `light` / `layout` tags); `featuredTheme`.
5. **New JL Studios packages** (all liked by Julian):
   - Plugins: **Takeout** (exists; list it), **Clockwork** (finds a camera whose clock was off by
     comparing its photos with others taken at the same time / place, proposes corrected dates
     via `proposals.submit` kind `date`), **Captioner** (proposes titles / captions from people,
     places and text in the photo, kinds `title` / `caption`), **Film Lab** (≈30 hand-made
     film-inspired looks for Develop; no brand names).
   - Themes: **Nocturne** (inky violet), **Coastline** (teal and sand), **Paper Mill** (warm
     paper, top bar layout).
   - Each with icon, screenshots and README, packed, in the local store, never published by Claude.
6. **Dev store**: a developer-only switch (env `LUMEN_STORE_DIR`, and Developer mode's *Use a
   local store folder…*) that reads `index.json` and packages / images from a local folder
   (`plugins/store/` + a git-ignored `plugins/store/dist/` that `pack.mjs` fills). Release builds
   ignore the env var unless Developer mode is on. This is how Julian sees the new store and
   Wayfinder's map button before he publishes.
7. **Version**: every version file back to `26.2.0-alpha` (package.json, src-tauri Cargo.toml and
   tauri.conf.json, installer's three files; the stray `26.3.0` at `tauri.conf.json:73`
   checked); the changelog's 26.3.0-alpha section folds into an *Unreleased* section;
   first-party manifests need `lumen >=26.2.0`. The newer-library refusal is schema-based
   (`CoreError::NewerSchema`), so this is safe for existing libraries.
8. **Open-source readiness** (audit 2026-11-24): the scrub must pass on the export
   (`node scripts/oss/export-public.mjs <empty folder>` ends with no finding), CI's scrub step
   passes, the updater endpoint question is answered, and Julian gets one checklist of what only
   he can do.

## Non-goals

- No push, no tag, no GitHub release, no store publish: Julian does those.
- No free (unchecked) CSS. No network for themes or plugins. No new plugin permissions except
  what Clockwork / Captioner need from existing ones (a host API read the builder finds missing
  is added as API 1.4 in the plugin's phase, with its own permission review).
- No Windows code signing (SignPath stays a placeholder).

## Anti-goals

- A store that looks like Round 25's: uneven card heights, a plugin repeated in three sections,
  placeholder puzzle icons where an icon exists.
- A theme able to relabel, hide or cover a destructive action, or to make Lumen touch the network.
- Any Wayfinder (or other plugin) name in `src/` outside test fixtures and the dev shim.

## Approaches considered

- **Discover layout**: A *Shelves* (Craft-style dense 3-up rows) vs **B *Tiles*** (identical
  cards with screenshots). Julian picked **B**: a photo app's store should be visual.
- **Themes**: (1) stay a plugin kind; (2) **own format, tokens + checked CSS on part hooks**;
  (3) own format with free CSS. Julian wants themes separate, lighter than plugins, and able to
  move things around, not only colours. (3) lets a theme phone home via `url()` and relabel or
  hide destructive actions; (2) gives layout freedom with no permission and no harm. Proposed
  and taken: **(2)** (Julian: "looks great, go ahead").
- **Seeing the new store before publishing**: publish to GitHub now vs **a dev store folder**.
  Publishing is Julian's step and the URLs are still placeholders, so a local folder.

## Constraints

CLAUDE.md non-negotiables: two transports (images over `photos://`, never base64), Rust does the
work (CSS checking in Rust), tokens only (theme CSS is generated / checked in Rust, the one
allowed colour source outside `globals.scss`), folder convention, generated TS types, GPL-3
compatible dependencies (`yarn run licenses` / `cargo` licence check after adding a CSS parser —
lightningcss is MPL-2.0, compatible), tests on temp libraries only, never the real library.

## Decided by Claude (Julian may override)

- Part-hook names and the exact property allowlist (documented in `docs/THEMES.md`, new).
- Theme fonts allowed as `woff2` ≤ 512 KB, checked by magic bytes, at most 2 families.
- *Try* reverts after leaving the page or 30 s; *Keep* installs.
- Open in Map: `/map?asset=<id>`; the map flies to zoom 15 over ~1.2 s, drops a pulsing pin on
  the photo, opens its preview card, then clears the param (Back returns to the viewer).
- Export excludes `CLAUDE.md`, `docs/superpowers/**` and `docs/STATE.md` (internal notes with
  personal paths); everything else that names a personal path is rewritten. Julian can say
  otherwise before the export.
- Takeout goes into the store index as-is (folders 1.0.0, zip 1.1.0 behaviour), re-packed.

## Builder-check answers (2026-11-24, Claude's calls unless noted)

1. **Try** runs the same Rust validator as install: the `.lumentheme` is downloaded (blake3
   checked), validated, unpacked to `cache/theme-try/<hash>/` and served like an installed
   theme; nothing is written to `themes/installed/` until *Keep* / *Get*.
2. **Part hooks are in scope.** Adding `data-part` to the shell, sidebar, title bar, toolbar,
   grid, tile, viewer, panels, cards and buttons is part of P2b. The app shell becomes a CSS grid
   with named areas on `[data-part="shell"]`, so a theme can move the sidebar to the top
   (`grid-template-areas`) without markup changes; the sidebar tolerates a horizontal layout.
   **Protected parts** (never hidden, covered, moved off-screen, relabelled or made
   click-through): `titlebar` window controls, `dialog`, `confirm`, `toast`, `menu`,
   `install-sheet`, `palette`. The list lives in `docs/THEMES.md` and in one Rust constant.
3. **Theme assets**: served at `photos://theme-asset/<id>/<path>`; `theme.css` names them by
   relative path (`url(fonts/serif.woff2)`, `url(images/grain.png)`), Rust checks the file is
   in the package and rewrites the URL to the route. Images: PNG / JPEG / WebP ≤ 2 MB.
4. **Clean removal** of plugin-carried themes: `[[themes]]` leaves the manifest (API 1.4, minor
   bump; nothing public, no migration), the SDK's theme helpers and Settings → Plugins' theme
   rows go, the example theme plugin(s) become `.lumentheme` files.
5. **Clockwork / Captioner data**: their phase starts by checking what `library.read` already
   gives (capture time, camera model, GPS, people, OCR text). If anything is missing, one shared
   API 1.4 read extension lands at the start of P4b, before both plugins. `date`, `title` and
   `caption` are already proposal kinds (Round 25 P4b-2a); their permissions are `dates.write`
   and `captions.write`.
6. **Index**: new fields are optional, so the Round 24 index keeps parsing. Icons / screenshots
   are fetched and cached by Rust (`store-image/<hash>`, Round 25 P1); the page never loads a
   remote URL. The dev store reads files from disk instead of downloading them.
7. **Export**: links in README / CONTRIBUTING / docs that point at excluded files are rewritten
   so nothing dangles (CONTRIBUTING points at `docs/PLUGINS.md`, `docs/THEMES.md` and the
   architecture spec, which is copied to `docs/ARCHITECTURE.md` scrubbed). Updater: if the
   release script doesn't inject the endpoint, `tauri.conf.json` gets the public repo's
   `releases/latest/download/latest.json`; editing config is not publishing.
8. Version sweep: grep for `26.3.0` / `26.4.0` across tests, goldens, scripts and manifests in P0.

## Left to the builder

Exact component split, the CSS parser crate, how the miniature preview is drawn (a small React
component fed the theme's tokens, no images), carousel timing, how Clockwork detects an offset
(a robust median of time deltas between a camera and the person's phone over shared places /
events, proposed only above a confidence), Captioner's wording rules, B2's root cause.
