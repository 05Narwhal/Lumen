# Lumen Setup — design

**Date:** 2026-11-05 · **Round 18 A2** · **Status:** approved by Julian 2026-11-06, including every call in *Calls made for Julian to check*
**Mockups:** `2026-11-05-lumen-setup-mockups.html` beside this file (six screens; Julian picked the Glass look)
**Parents:** architecture spec (a second binary, see *Crates*), UX spec §1, §3.1, §4, §5. RELEASING.md for the release path.

## Why

Julian: "a custom installer for Lumen with a clean and modern design + a setup for essential stuff for the app if needed." Today a person runs Tauri's stock NSIS wizard, which looks like nothing else in Lumen. *Lumen Setup* is a small branded window in Lumen's own look that installs, updates, repairs and removes Lumen on Windows.

## Goals

- One click installs Lumen with sensible defaults; *Options…* shows the four choices.
- The window looks like Lumen (Glass theme tokens, light or dark following Windows), not like a wizard.
- Updates keep using the NSIS installer silently (`/S`), so the updater (A1) does not change.
- Uninstall is branded, and **never touches a library folder**, even when the person asks to remove settings.

## Non-goals

- No installer on macOS or Linux. Lumen's in-app first-launch screen (theme, library, first import; UX spec §4) stays and still runs after Setup.
- No theme, library or import choices in Setup. Those stay in the app.
- No Settings toggle for *Start with Windows* in Lumen this round (a later round may add one; Setup owns the setting for now).
- No admin / all-users install. Lumen stays per-user (`bundle.windows.nsis.installMode: currentUser`).

## Anti-goals (a failure even if it works)

- Anything under a registered library, or any folder holding `library.db` / `library.json`, is deleted, moved or written.
- A failed model download makes the install fail or shows as an error.
- Setup kills a running Lumen before it has had the chance to close normally (Develop drafts would be lost).

## Screens (mockups 1–6)

640 × 420, frameless, Lumen's 38 px title bar with minimise and close, draggable. The background wash brightens with progress (`--light` 0.45 → 1); with reduced motion it jumps.

1. **Welcome.** Wordmark, "Your photos, on your machine, fast.", **Install** (defaults) and *Options…*; small line: version, size on disk, "no account, nothing leaves your computer".
2. **Options.** Install folder (default `%LOCALAPPDATA%\Lumen`, *Change…*), then three switches: **Desktop shortcut** (on), **Start with Windows** (off), **Download smart features now** (on, with the real size from the embedded model list, about 500 MB today: search by description, faces, pets, music beats, denoise). Footer: space needed and free on that drive. A folder that needs admin rights, isn't empty and isn't a Lumen install, or lacks the space says so under the path and keeps Install off.
3. **Progress.** Two tasks: *Lumen* (the NSIS install, about 15 s) then *Smart features* (bytes and time left). Once Lumen is installed, **Open Lumen** is available and *Skip smart features* stops the download. Closing the window during the model download asks nothing: Lumen finishes it.
4. **Done.** "Lumen is ready", one line about smart features (*Everything is installed.* or *Smart features will finish downloading the next time Lumen is open.*), *Open Lumen now* (ticked), **Finish**.
5. **Already installed.** Reads `HKCU\…\Uninstall\Lumen` (`DisplayVersion`, `InstallLocation`). Older → **Update to X** (keeps settings, libraries, shortcuts, install folder, options; Options are not shown). Same → **Repair**. Newer → "This PC already has a newer Lumen" with *Open Lumen* only: an older Lumen refuses a library a newer one migrated (Round 17), so Setup never downgrades. Every case also offers **Remove…** (screen 6). If Lumen is running, the line says it closes itself first and reopens after.
6. **Remove Lumen.** A fixed panel: "Your photos and libraries are not touched." with every registered library path. One box, **off by default**: *Also remove Lumen's settings, caches and downloaded models (size)*. **Remove Lumen**, *Cancel*. Then a short progress and "Lumen is removed."

Copy rules from the UX spec: one primary action per screen, sentence case, errors say what happened and what to do, no apologies.

### Amendment (2026-11-06): deleting libraries on removal

Julian asked for it (some people want their libraries gone with Lumen); it replaces the first anti-goal **only** for this explicit, typed request. Everything else about removal stays.

- Screen 6 gains a second, separate row under the libraries: *Delete libraries…* (only when at least one registered library exists on disk). It never changes the screen's default: nothing is deleted unless the person goes through the next step.
- **6b. Delete libraries.** One library: its name, path, photo count and size, and a field *Type "<name>" to delete it*. Several: a list with a box per library (all off), *Select all*, the count and size of what is ticked, and a field *Type DELETE to delete them*. The confirm button stays off until the field matches exactly (case-sensitive) and, with several, at least one is ticked. *Back* returns to screen 6 with nothing chosen. Confirming returns to screen 6, which then says what will go ("1 library will be moved to the Recycle Bin: …") with *Undo* (clears the choice); **Remove Lumen** then does it.
- **What goes:** only Lumen's own parts of each library: `library.db` and its `-wal`/`-shm`/`-journal`, `library.json`, `originals/`, `derivatives/`, `trash/`, `exports/`, `music/`, `logs/`. Anything else in the folder stays; the folder goes only if that leaves it empty. A folder without `library.db`/`library.json`, a drive root, the user profile itself, a library that is or contains app data or the install folder, and a library whose folder is a link/junction are refused (shown, never deleted). Links inside are removed as links, never followed.
- **Recycle Bin first.** The parts go to the Recycle Bin. Before deleting, Setup compares their size with the room the Bin has on that drive (a drive with no Bin, like a network share, has none). If they don't fit, 6b says *These libraries are too big for the Recycle Bin (12 GB; it has room for 4 GB). They would be deleted for good.* with *Cancel* and **Delete for good**.
- **When:** after Lumen has closed and its own uninstall succeeded, as the last step of *Remove Lumen*. A library that fails part-way is reported by name with what stayed; the rest continue. If app data is kept, the deleted libraries leave `libraries.json` (rewritten atomically).

## Behaviour

**Install.** Setup writes the embedded NSIS installer to a temp file and runs it `/S /D=<folder>` (per-user, no UAC). Progress for the Lumen task is real: bytes in the install folder against the payload's known installed size, finished when the process exits 0. A non-zero exit shows the NSIS log line and *Try again*; nothing is half-registered (NSIS's own rollback).

**Options as outcomes** (the builder checks how Tauri's NSIS template handles shortcuts in silent mode and gets these results):
- Start menu shortcut: always.
- Desktop shortcut: present only when the switch is on; on update, kept as it was. A silent NSIS install always creates it (template: `CreateOrUpdateDesktopShortcut` under `${Silent}`), so Setup notes whether it existed before and removes it afterwards when it should not be there.
- Start with Windows: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Lumen = "<folder>\lumen.exe" --minimized`. **Julian's choice:** Lumen opens minimised to the taskbar. Lumen gains a `--minimized` flag (shows the main window minimised instead of normal). With `--quit` (below), these are the only two changes in the app itself, plus the single-instance check `--quit` needs (below).
- Smart features: Setup downloads the same files, from the same release URL, into the same folder Lumen's own downloader uses, with the same check (size then SHA-256), `.part` then rename. Files already there and valid are skipped. Any failure (offline, 404, bad hash, disk full, skipped) leaves a state Lumen already handles: it downloads them later. Never an error screen. *Builder's call:* the switch off means **not now**, not *never*. Lumen already fetches missing models at launch while smart features are on (settings default), so the line under the switch reads "Off: Lumen downloads them the first time it opens." Setup writes no Lumen settings.
- *Builder's call:* Setup and Lumen never download at once (Lumen's one-download guard is per process and both would write the same `.part`). **Open Lumen** during the download, *Skip* and closing the window all stop Setup's download first and keep the `.part`, which Lumen resumes.

**Running Lumen.** *Builder's call (the check found the normal close is not 4 s: a running import asks the person with no time limit, Develop flushes for up to 3 s, and an exit with a downloaded update starts that update's install with a 20 s grace).* Lumen gains `--quit`: a second launch with it is handed to the running Lumen by the single-instance check (*Julian's choice, 2026-11-06, phase 6*: Lumen had none, so it gains `tauri-plugin-single-instance`; a plain second launch now brings the open window to the front instead of starting a second Lumen on the same library, and `Lumen --quit` with no Lumen running exits at once), which closes the normal way (Develop flush, import question) but **skips install-on-exit**, since Setup is installing. Setup shows "Closing Lumen…" and waits. While Lumen asks about a running import, Setup says "Lumen is asking about the import that's running. Answer there and Setup continues." and waits with no time limit; *Cancel* ends Setup and changes nothing. Setup never kills Lumen; NSIS's Restart Manager step (`installer-hooks.nsh`) stays only as the last fallback for a Lumen that has already ended its window. After an update or repair Setup reopens Lumen if it was open.

**Uninstall (Julian's choice: branded).** Lumen's NSIS bundle carries a slim Setup (the same app built without the payload, `lumen-setup.exe` in the install folder). `NSIS_HOOK_POSTINSTALL` points `UninstallString` at `"<folder>\lumen-setup.exe" --uninstall` and sets `QuietUninstallString` to NSIS's `uninstall.exe /S`, so Windows' *Installed apps* opens screen 6 and every update (which reruns the hook) keeps it so. If the slim Setup is missing, the stock uninstaller stays. *Builder's call:* the stock interactive installer (still uploaded, and Setup's fallback without WebView2) runs `UninstallString` with `/UPDATE`, `/P` and `_?=<dir>` added when it installs over Lumen (template `PageLeaveReinstall`). The slim Setup recognises those arguments, shows no window, runs `uninstall.exe` with them and waits, so that path behaves as today. No fork of Tauri's NSIS template; if this proves impossible, the builder stops and brings Julian the fork question. Removal: the slim Setup copies itself to `%TEMP%` and runs from there, closes Lumen, runs `uninstall.exe /S`, removes the Run value and the desktop shortcut, then, only if the box is ticked, deletes app data by **allow-list**:
- Only entries Lumen is known to create under `config_dir/JL-Studios/Lumen/` (settings, registry file, logs, caches, models, and whatever `photos-core::paths` names; the builder lists them) and Tauri/WebView2's `%LOCALAPPDATA%\com.jlstudios.lumen`. Nothing found that isn't on the list is touched.
- Before deleting anything, read `libraries.json`; skip any path that equals, contains or lies inside a registered library, and any folder holding `library.db` or `library.json`. Never a recursive delete of the app-data root itself.
- The registry file `libraries.json` is removed only with the box ticked; the libraries it pointed at stay where they are, and Lumen offers *Open other…* if installed again.
- **Julian's choice:** the safety copies taken before a library upgrade (`backups/pre-migration/`, real copies of `library.db`) are **always kept**, box or not. Screen 6 says "Safety copies of your catalogues are kept in `<path>`." and the done line reads "Lumen is removed. Its safety copies stay in `<path>`." when any exist; the app-data folder stays with only that folder. `backups.json` (backup schedules; the backups themselves live where the person chose) goes with the box.

## Crates and build

- `installer/`: a small Tauri v2 app (`lumen-setup`), Windows only, its own binary. UI in React + SCSS **without Mantine**, importing Lumen's tokens from `src/styles/` (Glass theme, both schemes) so it cannot drift from the app. Vite dev port **1477**; the browser shim pattern for visual checks. Whether it joins `src-tauri`'s Cargo workspace (shared compile cache) or stands alone is the builder's call, measured on build time. It may depend on `photos-core` (paths) and the model list/verify code; nothing in the app depends on it. The architecture spec's crate section gets one line for it. *Builder's call (the check found `verify`, `part_path` and the resuming download in the app binary, `src-tauri/src/models.rs`, and the model list in `photos-ml`, which links `ort`):* a new small crate **`photos-models`** holds `models.json`, the manifest types, `verify`, `part_path` and the resuming download; it depends on no workspace crate. `photos-ml`, the app and Setup depend on it. The architecture spec §3 and non-negotiable #2 change first, in the same commit.
- Build order in `scripts/release/` (release.mjs's build step): slim Setup → `tauri build --bundles nsis` (the slim Setup as a bundle resource) → full Setup with the signed NSIS installer embedded (`include_bytes!` or a resource the binary reads; the payload must be inside the signed file, never appended after signing) → `Lumen-Setup-<version>.exe` beside `Lumen_<version>_x64-setup.exe`. Both are uploaded; the channel manifests keep pointing at the NSIS installer.
- **WebView2 missing** (rare on Windows 10/11): Setup runs the embedded NSIS installer in its normal interactive mode, which brings its own WebView2 bootstrapper.

**Size budget:** full Setup ≤ NSIS installer + 6 MB (≈ 87 MB today); slim Setup ≤ 6 MB; window shown within 1 s on this PC.

**Code signing (Julian's choice: optional, unsigned for now).** The release script signs the slim Setup, the NSIS installer and the full Setup with `signtool` when a certificate is configured (env var naming it), else builds unsigned and prints that SmartScreen will warn. Not a gate this round. This is Authenticode, separate from the updater's minisign key.

## Tests

- Unit (Rust): option → NSIS args; version comparison → screen 5's case; the uninstall allow-list and library exclusion on a synthetic app-data tree (library inside app data, library containing app data, stray unknown files, `library.db` in a cache folder); model download states (valid file skipped, bad hash, 404, offline → "later").
- UI (vitest): every screen renders its states; keyboard reaches every control; Install disabled with each folder problem.
- On this PC, **under a test identity only (Julian's choice)**: test builds carry their own product name (*Lumen Test*), identifier, uninstall key, install folder and app data baked to a temp folder (Round 17's `--config` overlay and `LUMEN_BAKED_*`, which survive NSIS's `/R` relaunch where env vars don't), so Julian's installed Lumen, his app data and his library are never touched and he keeps the build he has. In this order: **fresh install** of an older test version; **update** to a newer test version (settings, test library, shortcuts, folder kept; Lumen reopened; a running test Lumen closed by `--quit`, including one holding the import question); **repair**; **remove with the box unticked** (before/after listings of the test install folder and test app data, and a read-only listing of Julian's real install, app data and library folder, which must not change); **remove with the box ticked** (allow-list only; a test library inside and around the test app data survives; safety copies kept); the **stock interactive installer over a Setup install** (the `/UPDATE` pass-through); finally a test install and removal so nothing of the test identity is left. The model download runs only into the test app data. No test writes to the library folder or uses a real version number.
- Visual: headless Edge, light and dark, against the mockups; before/after screenshots to Julian.

## Calls made for Julian to check

- One-click Install on Welcome, Options optional.
- Smart features on by default; desktop shortcut on; start with Windows off.
- Never downgrade; a newer install offers only *Open Lumen*.
- Uninstall's box off by default; allow-list deletion; `libraries.json` removed only when ticked.
- React without Mantine for size; tokens shared with the app.
- After the builder check: *Download smart features now* off means "not now"; Setup stops its download whenever Lumen opens; Lumen gains `--quit` (normal close, no install-on-exit) and Setup waits with no time limit while Lumen asks about an import; the slim Setup passes the stock installer's `/UPDATE` uninstall through instead of forking Tauri's NSIS template; a new `photos-models` crate holds the model list and file check.
