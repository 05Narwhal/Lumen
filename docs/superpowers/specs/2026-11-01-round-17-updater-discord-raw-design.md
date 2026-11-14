# Round 17: updates and versions, Discord status, every RAW format

Three parallel sessions, plus the leftovers of Sessions A, B and C (2026-11-01). Prompts: `plans/2026-11-01-round-17-session-prompts.md`.

## Why

- **Versions:** every build says 2026.1.0 and nothing has ever been released. Julian wants real versions he can track, and installs that keep themselves current.
- **Discord:** Lumen should show in Discord's status without disturbing a game's status or CPM's.
- **RAW formats:** some cameras' files can't be decoded by anything Lumen may legally ship (Nikon HE / HE\* is the known case; other makers' newest files and bodies are likely). Adobe's free DNG Converter, installed by the person, is the legal route. Session B built this for HE NEFs only; it should cover any RAW `rawler` refuses.

**Found while writing this:** a public release is Lumen's first real distribution, and `docs/LICENSING-AUDIT.md` lists `rawler` as **Blocking** (LGPL-2.1, statically linked, so people can't relink it) and the FFmpeg build as **Decide** (GPL-3.0). **Julian's decision: fix these first, then publish.** Jamendo and Esri block commercial use only, so they stay open and block `-stable` alone.

## Goals

1. Versions read `26.X.Y-<channel>`, and each release is one tracked, signed build.
2. Lumen finds, downloads and installs updates by itself, on a channel the person picks, without ever interrupting work or risking a library.
3. An opt-in Discord status in its own activity type, showing nothing about the photos.
4. Any RAW `rawler` refuses develops from Adobe's converter when installed, else from the camera's JPEG, with a plain note.
5. `rawler` becomes replaceable, FFmpeg's terms are met, and a release check refuses to publish otherwise.

## Non-goals

- No decoding of proprietary codecs by Lumen, no reverse engineering, no Nikon SDK.
- No macOS or Linux release builds yet, no release builds on GitHub Actions, no paid code-signing certificate. The updater code stays cross-platform.
- No batch conversion of a library: only photos opened in Develop are converted. Originals are never replaced by DNGs.
- No Wine route for Adobe's converter on Linux (Linux gets the camera JPEG).

## Anti-goals: a failure even if it technically works

- An update that interrupts an edit, loses a draft, or runs a migration with no safety copy.
- A half-published release being offered to anyone, or an unsigned file being run.
- A Discord status naming a file, album, person, place or date.
- Lumen shipping, downloading or silently installing Adobe's software.
- Anything written beside an original or inside a library folder by the RAW path.

## Rules for all three sessions

CLAUDE.md applies, above all its first section: **Julian's real library is irreplaceable**, tests use temp dirs, replays use a copy in the scratchpad. **No migration this round** (0032 stays free). The 70k budget is lifted for these three sessions. Nobody merges into `main` or publishes a release without Julian's OK.

---

## Session 1: updates and versions (`round-17-updater`)

### Version scheme (Julian's)

`26.X.Y-<channel>`, first release **`26.1.0-alpha`**.

- **X** = major release. Only Julian says when it goes up.
- **Y** = minor release, most updates.
- **channel** = `alpha`, then `beta` when more stable, then `stable` when publishable.

Semver order already does the right thing (`alpha` < `beta` < `stable`). *My calls:* Y resets to 0 on a new X. `26` stays until Julian changes it. Every published build gets a new Y, promotions included, so one number means one build. A plain `26.1.0` with no suffix is never published, because semver ranks it above `-stable`.

One source of truth: the release script writes `package.json`, `tauri.conf.json`, the workspace `Cargo.toml` and the numeric WiX version (`26.X.Y`, because an MSI allows numbers only). A test fails if they disagree. Replace `2026.1.0` in the browser shim. `store.rs`'s tests only use the string as an example.

### Channels and settings (Julian's)

Settings → new **Updates** section (`routes/settings/updates.tsx`):

- *Automatic updates*: on by default.
- *Update channel*: **Stable** by default; Beta; Alpha. A channel takes its own stability and anything more stable.
- Choosing Beta or Alpha shows a **development and unstable warning** beside the choice. *My call:* inline, not a confirm dialog, under the UX rule that reversible actions never confirm.
- The current version, *Check for updates*, and the state (*Up to date*, *Downloading*, *Ready: installs when you close Lumen*, with *Install now*, which restarts). A palette entry too.

**Consequence to tell Julian:** every first release is `-alpha`, so an install on the default Stable channel gets nothing until a `-stable` exists. He sets his own install to Alpha.

Moving from Alpha to Stable never downgrades: Lumen waits until Stable passes the installed version.

### How an update arrives (Julian's: download, install on close)

- *Auto on:* check after startup has settled, then every 6 hours while open (*my call*). Download at low priority through the activity ring. Stage it and tell the person once.
- *Auto off:* nothing happens until *Check for updates*, which then offers *Download*.
- **On close:** after Develop's flush (`window.rs`, `FLUSH_REQUESTED`) and any import hold, verify the signature and start the installer silently and detached, with no window and no UAC prompt. Lumen does **not** reopen itself; the next launch is the new version. *Install now* is the only path that restarts.
- **Close never blocks** (Round 15, `EXIT_GRACE`). If the install can't start in time, log it and retry at the next close.
- Check that `Lumen --mcp` bridges (Round 16) and FFmpeg children don't make the installer fail on locked files.
- After updating: a one-time *Updated to 26.1.4-alpha* toast with that version's notes.
- Failures are states, never errors: offline, GitHub's rate limit, a bad signature (file deleted, logged).
- Debug builds never check (*Updates are off in development builds*).
- The check is the first default-on network call. It sends no identifier or usage data, and the Updates section says so in one line. "No telemetry" stays true.

### Mechanism

- `tauri-plugin-updater` with minisign signatures, which can't be turned off (stronger than CPM's SHA-256 digest). **NSIS only** for updates, `installMode: currentUser`. *Builder's call:* whether a staged file or the plugin's in-memory install gets there, provided a signature check passes immediately before anything runs and a large installer isn't held in RAM for hours.
- **Hosting (Julian's):** public, assets-only `05Narwhal/lumen-releases`, like `cpm-releases` and `lumen-models`; no token in the app. Each build is a release `v26.1.0-alpha` (a GitHub pre-release unless `-stable`) with its installer, `.sig`, notes and third-party source. A permanent `channels` release holds `alpha.json`, `beta.json` and `stable.json`, each naming the newest build that channel may take. The app picks its endpoint at runtime by channel.
- **Windows, built on this PC (Julian's).** Other platforms simply find no entry in the JSON.

### Signing key

`tauri signer generate` to a path outside the repo. **Julian runs it himself** (it asks for a password), and it must be backed up: **losing the key strands every install**, which could then never update. The public key goes in `tauri.conf.json`. The private key is passed by environment only and is never committed or logged.

### Library safety (data safety is priority one)

- **Before any migration on an existing library:** `VACUUM INTO` a copy under app data (`backups/pre-migration/<library>/`), never in the library folder; keep the last 3 per library. `photos-core` gets the folder passed in. If the copy fails (disk full, say), the migration does not run and the library shows a plain state.
- **A library with a newer schema than this build is refused**, read-only, with a plain message. `apply_migrations` doesn't check this today.
- TDD on temp libraries. The existing `Db::open` callers and tests keep working.

### Release script and gates (`scripts/release/`, `docs/RELEASING.md`, `docs/CHANGELOG.md`)

One command, which **Julian runs**. It refuses unless all of these hold:

- clean tree on `main`; a CHANGELOG entry for this version; sidecars present; notices regenerated
- full checks green; the candidate opens a **copy** of a real library (in temp) and migrates
- **`rawler` appears in `cargo tree` only under `photos-raw`**, and the library is in the bundle (Session 3's contract)
- the source bundle exists for `rawler`, FFmpeg (with x264 / x265) and ExifTool / Perl, and Settings → *Licences* carries the offer
- for `-stable` only: `scripts/release/gates.json` shows Jamendo and Esri resolved

Publishing order: upload everything to a draft → download it back and verify → publish → **update the channel JSONs last**, so a half-done release is never offered. A `-stable` updates all three channels; a `-beta` updates beta and alpha; an `-alpha` updates alpha.

### Checks

Unit: channel ladder, version order, gate logic, the safety copy, the newer-schema refusal. **End to end:** a release build of an old version updates to a new one from a local server, signed with a throwaway key, with temp app data and a temp library (`LUMEN_APP_DATA_DIR`, `LUMEN_DEFAULT_LIBRARY_DIR`): close → installs → next launch is the new version, with drafts intact and a bad signature refused. Creating the public repo and the first publish wait for Julian.

---

## Session 2: Discord status (`round-17-discord`), plus worktree cleanup

### Cleanup first

Removes merged leftovers from Session C's list:

- `round-14-app`, `round-14-spectrum`, `round-15-s3`, `round-16-mcp`: only generated `graphify-out/` is untracked. Delete it, `git worktree remove` without `--force`, then `git branch -d` their branches and the merged `phase-7a-ml`.
- `round-14-prism`: compare its untracked plan with the tracked one; remove only if identical, else ask Julian.
- `round-11b`: **ask Julian** (`scripts/ml/change_code_gpt.py`, `gpt_zip_sending.py`, `lumen-ml-debug.zip`, `training-fix-backups/`, `.history/`).

### Status

Port CPM's design from `coding-project-manager-rs/src-tauri/src/modules/discord_presence.rs` into `src-tauri/src/discord_presence.rs`: a hand-rolled local IPC frame with no new crate, a pure "what to show" function, a pure scheduler (15 s rate limit), and a worker thread. Discord not running is the normal case and is never surfaced.

- **Its own category (Julian's): activity type 5, Competing**, reading *Competing in Lumen*. Games use Playing and CPM uses Watching. Lumen also has **its own Discord Application ID**, so it can't overwrite CPM's activity. Types 2, 3 and 5 being accepted comes from CPM's working code (3) and community library docs (2, 5), not Discord's own docs, so Julian's real check confirms it.
- **Application ID:** Julian creates the "Lumen" application in Discord's developer portal and uploads a `lumen_logo` image. Embedded at build time like CPM's (`LUMEN_DISCORD_CLIENT_ID`, or a git-ignored id file). It's a public identifier. With none set, Settings hides the section and no worker starts; that is a state, never an error.

### What it shows (*all my calls*; Julian can overrule)

- **Off by default.** Lumen's promise is local-first, and this publishes to a third party. (CPM's is on by default.) Off means no pipe is opened.
- Settings → **Discord status** (`routes/settings/discord.tsx`): the switch, *Show what I'm doing*, *Show elapsed time*, and a live preview line of what others see. Palette entry.
- Coarse activity only, from a fixed enum the frontend sends on route change: *Browsing the library*, *Looking through photos* (viewer), *Developing a photo*, *Watching a slideshow*. Everything else counts as browsing. With *Show what I'm doing* off: details are left out.
- **Never:** file names, album or people names, places, dates, counts, thumbnails. The enum carries no ids, so it can't leak them; a test holds this.
- Cleared when idle for 15 minutes and on close.
- All platforms: `\\.\pipe\discord-ipc-N` on Windows; `$XDG_RUNTIME_DIR` / `TMPDIR` on Unix, plus the Flatpak and Snap subfolders.

### Checks

TDD on the pure layers, including the type-5 frame, plus a fake pipe server for connect, send, clear and Discord going away. Browser shim entry; `/dev/ui` shows the section. Add a Discord row to `LICENSING-AUDIT.md`. **Julian's real check:** a second account or friend sees *Competing in Lumen*, and a running game and CPM keep theirs.

---

## Session 3: every RAW format (`round-17-raw`, on top of `develop-he-nef`)

### Land Session B first

In the `he-nef` worktree: merge `main` into `develop-he-nef` (conflicts in `CLAUDE.md` and `docs/STATE.md` only, both docs), run the full checks, then branch `round-17-raw` from it. Julian's HE NEF check moves to this branch's end and both merge together.

### Measure before generalising

**What `rawler` actually refuses is unknown.** The source shows per-format refusals and an "Unknown camera" path; a CRX decoder exists; one old forum post says C-RAW wasn't supported. So sweep real files: CC0 samples from raw.pixls.us (check each licence, keep them in the scratchpad, never in the repo) across Canon (CR2, CR3, C-RAW), Sony (compressed, lossless compressed, newest bodies), Nikon, Fujifilm (compressed, lossless), Panasonic, OM / Olympus, Pentax, Leica, Hasselblad and a phone DNG, including JPEG XL DNG.

For each: **decodes / refused / decodes wrongly** (compare against the embedded JPEG, because a silent wrong decode is worse than a refusal), and do thumbnails and the viewer work? Result: `docs/RAW-FORMATS.md`.

### Generalise the stand-in

`he_nef` becomes a general stand-in (name is the builder's call) with the same safeguards: cache under app data, per-hash lock, temp then rename, 180 s timeout, dies with Lumen, below normal priority, never beside an original or in a library.

- **Rule:** `rawler` stays first (faster, no install). **Any format-level refusal**, or a format the sweep marks wrong → Adobe's converter if installed → else the camera JPEG labelled *Camera rendering, not RAW* → else the plain refusal. I/O errors (missing file, no permission) do **not** trigger it.
- The sweep decides any further case: a preview path for bodies `rawler` doesn't know (ExifTool, the sidecar we have), missing extensions in `formats.rs`, and that the converter's flags still produce a DNG `rawler` reads (`-c -p0` was checked on 18.7 only). Export, Auto, Retouch, Denoise and MCP all follow, through `load_base_as`.
- **Generalise `HE_NEF_MESSAGE`**: it must not name Nikon for a Canon file.
- *My call:* the DNG cache is capped at 20 GB, oldest-opened first, since it's derived data and a heavy shooter could fill a disk. A failed hash retries next launch, so updating the converter counts.

### What the person sees

- Develop's note gains a way to act: *Camera rendering, not RAW* links to the new section.
- Settings → **RAW formats** (`routes/settings/raw-formats.tsx`): Adobe DNG Converter *Found* / *Not found*; *Get Adobe DNG Converter* opens Adobe's page in the browser; the cache's size with *Clear*. On Linux: Adobe makes no Linux version.

### The legal line

**Never ship, bundle, download or auto-install Adobe's software.** Lumen runs only the copy the person installed, and names Adobe only to say what it uses.

**Not yet verified:** I couldn't read the converter's EULA. The install folder has no licence file, Adobe's page refused automated fetching, and search only found the *DNG SDK* licence, a different agreement. Adobe does document a command-line interface. **Task:** get the real EULA text (the installer shows it; ask Julian if needed), record what it says about launching from another program in `LICENSING-AUDIT.md`, and **stop and ask** if it forbids this. The fallback then is that the person converts by hand and Lumen reads the DNG. This is engineering diligence, not legal advice; a paid release deserves a lawyer's look.

### Make `rawler` replaceable (Julian's decision)

Move every `rawler` use (only three files today: `photos-media`'s `decode.rs`, `metadata.rs`, `develop/raw.rs`) into a new workspace crate **`photos-raw`**, built as a **separate library** (`photos_raw.dll`, `libphotos_raw.so`, `libphotos_raw.dylib`) with a small C interface covering decode, preview and metadata, loaded at run time from the bundle (`bundle.resources`), with its source (LGPL-2.1) in the release's source bundle so anyone can rebuild and swap it. *Builder's call:* a helper program instead, if it measures better; either meets the terms the audit names, and S1's gate then checks for the helper.

- `photos-core` stays free of it; only `photos-media` talks to it. This changes the crate layout, so **update the architecture spec's crate list first** (`2026-09-17-photos-app-design.md`, the layout and dependency diagrams) and the CLAUDE.md one-way deps line.
- Catch panics at the boundary (`rawler` has `panic!` paths). A missing library is a state: RAWs fall back to ExifTool's previews and metadata, never a crash. Tests find the library by an env override or beside the test binary (*builder's call*).
- Measure decode speed before and after (a RAW open and a batch), and report it. Windows is what ships, but it must build on all three OSes.
- Update `LICENSING-AUDIT.md` and `notices.json` (`yarn run licenses`).

### Checks

TDD for the refusal rule, cache cap and message. Real checks on temp libraries: sweep samples imported, opened in Develop and exported, with the converter and with it hidden (`LUMEN_DNG_CONVERTER=C:/nothing.exe`). Same results before and after the `photos-raw` move. **Julian's check:** HE NEF and one Canon or Sony file in the app, with and without the converter, including an export.

---

## Parallel lanes

| | Owns | Touches lightly |
|---|---|---|
| **S1** | `updater.rs`, `commands/updates.rs`, `scripts/release/`, `docs/RELEASING.md`, `docs/CHANGELOG.md`, version fields, `photos-core::db` (safety copy, newer-schema), `window.rs` close path, `routes/settings/updates.tsx` | `Settings` struct, settings section list, `notices.json` (one new plugin) |
| **S2** | `discord_presence.rs`, `commands/discord.rs`, `build.rs` id embedding, presence hook, `routes/settings/discord.tsx`, worktree cleanup | `Settings` struct, settings section list, command registration |
| **S3** | `photos-media::develop::*`, new `photos-raw`, `formats.rs`, `kind.rs`, `develop_preview.rs`, Develop note, `routes/settings/raw-formats.tsx`, `docs/RAW-FORMATS.md`, `LICENSING-AUDIT.md` | settings section list, architecture spec's crate list, `tauri.conf.json` resources, `notices.json` |

S1 and S3 both touch `tauri.conf.json` (S1 the version and updater, S3 `bundle.resources`) and `Cargo.lock`; on merge, keep both and regenerate the lockfile and `notices.json`, never hand-edit them. New `Settings` fields use `#[serde(default)]`, so older settings files load. `docs/STATE.md` and CLAUDE.md *Current state*: add only your own section or line. Merge in any order; **the first public release comes after all three are in `main`**, since S1's gate needs S3's `photos-raw`.

## What only Julian can do

1. Generate and back up the signing key.
2. Create the Discord application and logo, and give the ID.
3. Say yes before the public repo is created and before the first publish.
4. Decide on `round-11b`'s leftover files.
5. Check all three in the app.
6. Earlier leftovers still open: Session A's in-app check and speed re-run; the 8f training-data question.
