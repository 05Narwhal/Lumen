# Releasing Lumen

Round 17. Only Julian publishes. Windows builds only, made on his PC; the updater code is cross-platform and other platforms simply find no entry in a channel's manifest.

## Versions

`26.X.Y-<channel>`: **X** a major release (only Julian says when it goes up; Y resets to 0), **Y** most updates, **channel** `alpha`, then `beta`, then `stable`. Every published build gets a new Y, promotions included, so one number is one build. A plain `26.1.0` is never published: semver ranks it above `-stable`.

The version lives in `package.json`, `src-tauri/tauri.conf.json` (plus the numeric WiX version), and the workspace `src-tauri/Cargo.toml`. Change it only with:

```sh
node scripts/release/set-version.mjs 26.1.1-alpha
```

`src-tauri/tests/version_fields.rs` fails if the fields disagree.

## Releases from CI (2026-11-14)

Pushing a tag `v26.X.Y-<channel>` runs `.github/workflows/release.yml`: it uploads the FFmpeg source bundle once (release `ffmpeg-sources-n9.0.2-47`, made by `scripts/release/ffmpeg-sources.mjs`; the job skips when that release exists), builds Windows, macOS (universal) and Linux (AppImage and deb) with `tauri-apps/tauri-action` into a draft release whose notes come from `docs/CHANGELOG.md`, publishes the draft, then `scripts/release/channels.mjs` points the channel manifests at the Windows installer (the updater is Windows-only today). Secrets: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `LUMEN_PLUGIN_PUBKEY`, `LUMEN_DISCORD_CLIENT_ID`; a fork without them still builds with those features hidden. The Windows job needs `ffmpeg-x86_64-pc-windows-msvc.exe` and `ffprobe-x86_64-pc-windows-msvc.exe` to be the build `sources.json` describes: upload them to the `ffmpeg-sources-…` release once (the job takes them from there, else fetches one) and it stops if the hash differs from `ffmpeg.sidecarSha256`. Windows code signing goes through SignPath Foundation once the project is approved; until then the step only says so and the notes read *unsigned: SmartScreen will warn*. macOS is not notarized (the notes give *right-click → Open*). The local `release.mjs` flow above (gates, Lumen Setup, library check) still works for a build made on a PC; the workflow does not build *Lumen Setup*.

## Where releases live

**Amended 2026-11-14 by the open-source plan:** releases live in the public source repository `05Narwhal/lumen`, and the models stay in the public `05Narwhal/lumen-models`. The `releases-repo` gate checks `05Narwhal/lumen`. The updater's default URL, `publish.mjs` and `publish.ps1` name `05Narwhal/lumen` (Phase 3, Task 8 of `docs/superpowers/plans/2026-11-14-open-source.md`). **The local `26.1.1-alpha` backup build still polls `05Narwhal/lumen-releases`, so it never sees an update: reinstall it once from a build of the new repo.**

Each build is a release `v26.1.1-alpha` (a GitHub pre-release unless `-stable`) with the NSIS installer, its `.sig`, the full Lumen Setup (`Lumen-Setup-<version>.exe`), the notes, and the third-party source. A permanent release named `channels` holds `alpha.json`, `beta.json` and `stable.json`, each in `tauri-plugin-updater`'s static format and naming the newest build that channel may take. A channel with no build yet holds `0.0.0-<channel>`, which no install takes. Lumen reads `https://github.com/05Narwhal/lumen/releases/download/channels/<channel>.json`; no token is in the app.

A `-stable` updates all three channels, a `-beta` updates beta and alpha, an `-alpha` updates alpha.

## The signing key (once)

The updater only runs files signed by Lumen's key (minisign, through `tauri signer`). **Losing the key strands every install**: they could never update again. So:

1. In your own terminal (it asks for a password): `yarn tauri signer generate -w $env:USERPROFILE/.lumen-release/lumen.key`
2. Back up `lumen.key` and its password somewhere safe and offline. Never commit, paste or log either.
3. Put the public key (`lumen.key.pub`'s content) in `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, and commit that. Until it is set, Lumen shows *Updates are not set up in this build* and makes no network call.

To release, the private key goes in the terminal's environment only:

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $env:USERPROFILE/.lumen-release/lumen.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = Read-Host -AsSecureString | ConvertFrom-SecureString -AsPlainText
```

## Making a release

1. On `main`, clean: `node scripts/release/set-version.mjs 26.1.1-alpha`, write its section in `docs/CHANGELOG.md` (the text Lumen shows as the notes), run `yarn run licenses`, commit.
2. Set the key (above) and `LUMEN_RELEASE_LIBRARY` to a real library's folder. The script only reads it: the candidate migrates a temp copy of its catalogue.
3. Try it: `node scripts/release/release.mjs 26.1.1-alpha --dry-run`. It prints every gate. `--skip-checks` (dry run only) skips the full test suites.
4. Release: `node scripts/release/release.mjs 26.1.1-alpha`.

**From CPM:** the Lumen project's *Publish release* script runs `scripts/release/publish.ps1`, which does steps 2 to 4 in one window: it loads the key from `$env:USERPROFILE/.lumen-release/lumen.key` and asks for its password (both stay in that window's environment only), asks for the library (default `C:/Users/Sample/Pictures/Lumen Library`, only read), offers to make `lumen` public while it is private, runs the dry run, and publishes only after you type the version. Step 1 stays yours.

It refuses unless every gate holds:

| Gate | Holds when |
|---|---|
| `version`, `version-fields`, `next-version` | The version follows the scheme, every field says it, and it comes after the last published one. |
| `clean`, `main` | No changes in the tree, on `main`. |
| `changelog` | `docs/CHANGELOG.md` has `## <version>`. |
| `sidecars` | FFmpeg, ffprobe and ExifTool are in `src-tauri/binaries`. |
| `notices` | `yarn run licenses` changes nothing. |
| `checks` | `cargo test --workspace`, clippy with `-D warnings`, `yarn typecheck`, `yarn test`. |
| `signing` | A public key in `tauri.conf.json` and `TAURI_SIGNING_PRIVATE_KEY` set (only checked for presence). |
| `no-test-overrides` | None of `LUMEN_BAKED_APP_DATA_DIR`, `LUMEN_BAKED_LIBRARY_DIR`, `LUMEN_UPDATE_TEST_BASE` is set: they make a build use other folders or another update server. |
| `library-check` | The built `lumen.exe --check-library $LUMEN_RELEASE_LIBRARY` migrates a temp copy. |
| `rawler` | `cargo tree -i rawler` shows only `photos-raw` depending on it, and its library is in `bundle.resources` (the replaceable library; holds since `main` was merged in, 2026-11-02). |
| `ffmpeg-source` | The FFmpeg sidecar is the build `scripts/release/sources.json` records (version and SHA-256), **every `--enable-*` in its own configuration** has its source listed, and each source file is in the bundle with its SHA-256. A new FFmpeg build with a new library refuses until its source is listed. |
| `source-bundle` | ExifTool and Strawberry Perl sources are in the bundle (`photos-raw`'s source is the tagged repository). |
| `releases-repo` | `gh` can see `05Narwhal/lumen`. |
| `stable-only` | For `-stable` only: `scripts/release/gates.json` marks Jamendo and Esri resolved. |

Then it builds (`scripts/release/setup-build.mjs`, see *Installer and Lumen Setup*), checks the size budget, and publishes in this order, so a half-done release is never offered: upload everything to a **draft** → download it back and **verify** each file's SHA-256 → **publish** → update the **channel manifests last**. The first publish creates the `channels` release with all three manifests.

The third-party source is gathered into `dist-release/<version>/source/` (git-ignored) from the URLs and SHA-256s in `sources.json`. There is no written offer of source and no `photos-raw` archive any more: the source is public, and Settings → Licences links to `https://github.com/05Narwhal/lumen`. The FFmpeg source bundle (the exact source of the FFmpeg build Lumen ships, which the GPL still asks for) is uploaded once per FFmpeg build as the release `ffmpeg-sources-n9.0.2-47` (Task 9's workflow); every other source is the tagged repository.

## Installer and Lumen Setup

A release carries two Windows files, both uploaded: `Lumen_<version>_x64-setup.exe` (the NSIS installer; the channel manifests and the updater point **only** at this one) and `Lumen-Setup-<version>.exe` (the full Lumen Setup, the branded window that runs that installer silently, for people downloading Lumen the first time). The build order is fixed by one rule, that the payload sits inside the signed file and nothing is appended after signing:

1. `vite build` for Setup's window, then the **slim Setup** (`installer/src-tauri`, no payload, about 5 MB), signed.
2. `tauri build --bundles nsis` with a `--config` overlay file (`dist-release/<version>/nsis-overlay.json`) that adds the slim Setup as a bundle resource landing at **`$INSTDIR\lumen-setup.exe`** (exactly that name: the hook `NSIS_HOOK_POSTINSTALL` makes it Lumen's uninstaller only when it is there) and the updater artefacts. Tauri signs the installer **during** this build (`bundle.windows.signCommand`), because the updater's minisign `.sig` is taken over the finished file; signing afterwards would invalidate it.
3. The installed size is added up from the merged Tauri config (Lumen, the sidecars, every resource, the slim Setup, the uninstaller) and baked into the full Setup as `LUMEN_SETUP_INSTALLED_BYTES`; Setup's free-space check and the progress bar use it (installed size + the installer written to temp + 5 %).
4. The **full Setup** is built with `--features payload` and `LUMEN_SETUP_PAYLOAD=<the signed installer>` (`include_bytes!`), signed, and written beside the installer as `Lumen-Setup-<version>.exe`.

**Signing is optional.** Set `LUMEN_SIGN_CERT_SHA1` to the 40-digit SHA-1 thumbprint of an Authenticode certificate in your Windows certificate store (no secret goes in the environment; `LUMEN_SIGN_TIMESTAMP_URL` overrides DigiCert's timestamp server). `signtool` is taken from `PATH` or the newest Windows SDK. With it set, the slim Setup, the NSIS installer and the full Setup are signed; without it everything builds unsigned and the release prints that Windows SmartScreen will warn. It is separate from the updater's minisign key and is not a gate this round.

**Size gate `setup-sizes`** (checked after the build, before anything is uploaded): the full Setup is at most the NSIS installer + 6 MiB, the slim Setup at most 6 MiB. `gates.mjs` `setupSizeGate`.

**Lumen Setup's version** is `installer/package.json`, `installer/src-tauri/tauri.conf.json` and `installer/src-tauri/Cargo.toml`; `set-version.mjs` writes them and `src-tauri/tests/version_fields.rs` fails if they disagree with Lumen's (Setup shows that version and compares it with the installed one).

**Test identities never reach a release.** `LUMEN_SETUP_PRODUCT` and `LUMEN_SETUP_IDENTIFIER` (a *Lumen Test* Setup) join `LUMEN_BAKED_*` in the `no-test-overrides` gate, and the build clears `LUMEN_SETUP_PAYLOAD` / `LUMEN_SETUP_INSTALLED_BYTES` for the slim Setup.

**Third-party notices.** Setup's Rust and npm dependencies are all already in `src/assets/licenses/notices.json` (checked 2026-11-06: tauri, tauri-plugin-dialog, reqwest, tokio, winreg, windows-sys, sysinfo, ts-rs, react, Inter), and the full Setup carries Lumen's installer with its Licences screen, so `scripts/licenses/generate.mjs` still reads only `src-tauri/Cargo.toml`. A new Setup-only dependency must be added to that script's graph first.

**Workspace or standalone:** standalone (decided in phase 3: cold test build 1 min 16 s; Cargo honours `[profile.release]` only at a workspace root and Setup needs `opt-level = "s"` + LTO + `panic = "abort"`). A cold release build of Setup takes about 4.5 min with its own `installer/src-tauri/target`; Lumen's builds are not slowed by it.

## What still blocks the first release (updated 2026-11-06)

The notes for `26.1.0-alpha` are written (`docs/CHANGELOG.md`) and CPM's *Publish release* script exists.

- **The signing key** (Julian, above). `publish.ps1` stops with the command to make it while it is missing.
- **`05Narwhal/lumen` is private.** `publish.ps1` offers to make it public before the dry run (or: `gh repo edit 05Narwhal/lumen --visibility public --accept-visibility-change-consequences`). While it is private every check reads as *Could not check*.
- **The FFmpeg sidecars: Julian chose B (BtbN) on 2026-11-06.** Still to do, one session: take `ffmpeg-n9.0-latest-win64-gpl-9.0.zip` from a dated `autobuild-*` release (keep the zip and its `checksums.sha256` line in `dist-release/ffmpeg/`), run *Before swapping* below, rewrite `sources.json` → `ffmpeg` from that day's `scripts.d` commit, then replace both sidecars. Until then the `ffmpeg-source` gate refuses.
- **FFmpeg route B, progress (2026-11-07).** *Phase 1 done:* `autobuild-2026-11-06-12-50`, `ffmpeg-n9.0.2-47-gb204e94e90-win64-gpl-9.0.zip` (SHA-256 `8436c891…a0a8`, matches BtbN's `checksums.sha256`; the zip and the checksum line are in `dist-release/ffmpeg/`, git-ignored) was extracted elsewhere and run: `libx264`, `libx265`, `libaom-av1`, `h264_nvenc`, `h264_amf`, `h264_qsv`, `zscale`, `tonemap`, `xfade`, `overlay`, `concat` are all present. `cargo test -p photos-media --test ffmpeg` (all 5 including the ignored speed test, NVENC on this PC), the slideshow and export lib tests (24 incl. AVIF/HEIC/trim) and `slideshow_export` on six synthetic photos (21.0 s of 1080p, x264 and NVENC, duration exact) passed with `LUMEN_FFMPEG_DIR` pointing at it. AMF and QSV could not run here (NVIDIA only). *Found:* this build enables **75 `--enable-*` flags** (gyan's essentials had far fewer: libplacebo, librsvg, rav1e, libjxl, svtav1, vulkan, opencl, vaapi, …), so `sources.json` grows to ~120 components. *Phase 2, started:* `scripts/release/btbn-sources.mjs` (+ test) reads `scripts.d` at the scripts commit **`c63bb308a1de656b425c132a8102f8fd5389afc5`** (the Actions run that made this build; clone BtbN/FFmpeg-Builds and check it out), finds the stages the binary's flags need plus their `ffbuild_depends`, and gives each an archive URL; `btbn-plan.mjs <checkout> <ffmpeg -version file> [draft.json]` prints the plan (75 flags → 84 stages → 124 components, no unclaimed flag). `gates.mjs` now reads BtbN's `n9.0.2-47-g…-date` version string. **Still to do:** (1) a fetch step that downloads each archive with retries, fills `sha256`/`file` and writes `sources.json` → `ffmpeg` (`version` = `n9.0.2-47-gb204e94e90-20261106`, `options` = gpl, version3, pthreads, schannel; FFmpeg itself = FFmpeg/FFmpeg commit `b204e94e90` resolved to its full SHA; add BtbN/FFmpeg-Builds at `c63bb308…` as a component, because its `patches/` are applied to FFmpeg, aom and others); (2) the **6 submodule builds** (libjxl, libplacebo, mbedtls, glib, openssl, zimg): a GitHub archive leaves submodules out, so list each submodule's commit from `.gitmodules`; (3) **lame r7113 and xvid r2204 are Subversion revisions**, no archive exists: export them and host the tarball with the release's source, or take the nearest release tarball and say so; (4) **librsvg and rav1e are Rust**: their crates come from crates.io at build time, and each `Cargo.lock` carries every crate's SHA-256, so list them from it; (5) drop the gyan-only `others` rows (pixman, nettle, libsodium, expat, OpenSSL "linked into FFmpeg") and let the plan supply what this build links; (6) replace both sidecars with the extracted `ffmpeg.exe` / `ffprobe.exe`, set `sidecarSha256`, run `node scripts/release/release.mjs 26.1.0-alpha --dry-run --skip-checks` until `ffmpeg-source` holds; (7) `docs/STATE.md` says this machine has gyan's build: update it, and CLAUDE.md's summary. **The sidecars in `src-tauri/binaries/` are still gyan's 9.0.1** and `sources.json` is unchanged, so the gate still refuses.
- **FFmpeg route B, progress (2026-11-07, later session).** Built and committed (`c2c071b`): `btbn-fetch.mjs` (plan → rows with SHA-256; git submodules resolved with `git ls-tree`; Rust crates from the `Cargo.lock` of rav1e and librsvg, with crates.io URLs and the lock's checksums), `svn-export.mjs` (LAME r7113 and Xvid r2204 read over Subversion's HTTP interface into reproducible tarballs kept in `scripts/release/vendored-source/`; a component with `local` is copied into the bundle and checked by the gate). The **sidecars in `src-tauri/binaries/` are now BtbN's n9.0.2-47** (ffmpeg SHA-256 `a7653b63…8e2d`; gyan's 9.0.1 kept in `dist-release/ffmpeg/gyan-9.0.1-backup/`). `sources.json` is **not** rewritten yet: run `node scripts/release/btbn-fetch.mjs <FFmpeg-Builds checkout at c63bb308…> <ffmpeg -version file> --sidecar-sha <hex> --write` once these are settled. Open: (1) **38 archives answer 406** from gitlab.freedesktop.org, code.videolan.org (x264, dav1d, libplacebo, libbluray…), gitlab.com (libssh) and git.savannah.gnu.org (500/400): those hosts refuse the script's plain request; no browser-header workaround was used. Decision needed: mirror them, fetch by `git` and host our own tarballs, or ask the hosts. (2) **AMF** (a download of ~400 MB) reset 12 times in a row; `curl -C -` is wrong for GitHub archives (no range support: it appended a second copy), so retry whole. (3) `scripts/fetch-ffmpeg.ps1` still downloads gyan's build for new dev machines. (4) Then the dry run (`release.mjs 26.1.0-alpha --dry-run --skip-checks`) and STATE.md / CLAUDE.md summaries.
- **FFmpeg route B, dry run (2026-11-08).** `sources.json` rewritten (782 sources); `dist-release/26.1.0-alpha/source/` holds 731 files (the 578 that `missing.curl` fetched, plus the rest copied from `dist-release/ffmpeg/source-cache/` and `vendored-source/` by `source-bundle.mjs`, which now reuses the cache and times each download out at 20 minutes). `node scripts/release/release.mjs 26.1.0-alpha --dry-run --skip-checks` reports **`ffmpeg-source` ok and `source-bundle` ok** (every file present and equal to its recorded SHA-256), and also ok: main, changelog, version-fields, next-version, sidecars, no-test-overrides, rawler, licences-offer, releases-repo. Still refusing: `clean` (commit), `notices` (the file carries its generation date, so `yarn run licenses` must be re-run and committed on release day), `checks` (skipped by the flag), `signing` (`TAURI_SIGNING_PRIVATE_KEY`, Julian's key). Nothing was built or published.
- **AMF's source archive (398 MB) is flaky to fetch:** GitHub resets the connection part way. `release.mjs` retries each file five times and then lists it as missing; run it again (the files already fetched are kept) until the gate holds.

## The FFmpeg gate: ways out (2026-11-02, for Julian to pick)

**The problem.** The GPL asks Lumen, the one who hands out the FFmpeg program, for the exact source of what is inside it. gyan.dev's 9.0.1 essentials build names four libraries that no tag, release or findable commit matches: cairo 1.18.5, x265 `4.3-6-g9ddc216`, libva 2.25.0, libgme 0.6.6 (checked 2026-11-01 against each upstream; sources.json has the notes). Of the four, only **x265 is used by Lumen** (HEIC export and the HEVC trim export; `libx264` is the CPU H.264 encoder, `libaom-av1` is AVIF, `zscale` tone-maps HDR). cairo, libva (VAAPI is Linux-only) and libgme (game-music files) are in the build but not used.

**What Lumen needs from FFmpeg** (from `photos-media` and the Round 15 video prep): `libx264`, `libx265`, `libaom-av1`, the GPU H.264 encoders `h264_nvenc` / `h264_amf` / `h264_qsv` (the last through Intel's libvpl) plus `aac` (built in), `zscale` (libzimg) with the built-in `tonemap`, `xfade`, `overlay` and `concat`. It does **not** use subtitles, networking (srt, ssh, zmq), the audio codecs behind libopus/vorbis/mp3lame, or Media Foundation, so most of the 60-odd libraries in gyan's build are weight and paperwork, not features.

| | **A. Ask gyan.dev** | **B. BtbN's FFmpeg-Builds (recommended for the alpha)** | **C. Build FFmpeg here, trimmed** |
|---|---|---|---|
| Exact source | Only if gyan.dev answers with four commits. He may not have them (these are development versions of his own tree), may not answer, and the same hole reopens with every update. | Every library is pinned by commit in `scripts.d/*.sh` (e.g. x265 `5143a2a0…`, x264 `0480cb05…`, aom, zimg, AMF, libvpl, nv-codec-headers); the FFmpeg commit is in the file name (`n9.0.2-39-g7adfbe40dc`). The scripts' own commit for a given day's build is on that day's Actions run (2026-10-31: `5031c6f1`). | All of it, by construction: we choose the commits. |
| Licence | GPLv3 (`--enable-gpl --enable-version3`), as today. | GPLv3, same as today. The scripts repo is MIT. **Their LGPL variant is no way out:** it drops x264 and x265, which Lumen uses. | GPLv3 while x264/x265 stay; the build is ours to trim to what Lumen uses. |
| Size | Today: ffmpeg 98.1 MB + ffprobe 97.9 MB. | Measured from the zip's listing (the 2026-10-31 build, `n9.0.2-39`, GPL static): **ffmpeg.exe 166 MB, ffprobe.exe 166 MB**, i.e. ~136 MB more in the installer's input than today (the installer compresses it). BtbN also has `-shared` builds (82 MB zip: small exes plus DLLs both programs share), which would be smaller overall but means shipping DLLs beside the sidecars. | A trimmed build (the ~10 libraries above, no ffplay) should come out well under today's 98 MB. Not measured. |
| Encoders Lumen uses | Known to work: it is what Round 15 was tested on. | The scripts enable x264, x265, aom, zimg, AMF, nv-codec-headers (`--enable-ffnvcodec`) and libvpl (`--enable-libvpl`) for the GPL win64 variants. **Not verified by running the binary** (this session's sandbox refused to run a downloaded executable): before swapping, run `ffmpeg -encoders` and `cargo test -p photos-media --test ffmpeg` against it. | We choose them; nothing to guess, but every encoder is tested on a GPU of each make (only an NVIDIA/AMD/Intel mix of PCs can do that). |
| Work | Small: write the message (below), wait. If he answers, fill four entries and the gate passes. If not, we are back to B or C with days lost. | Medium: download, check, replace the two sidecars (`sidecarSha256`), re-fill `sources.json` from the scripts at that commit (~50 entries, each a URL + SHA-256: roughly the work done on 2026-11-01 for gyan's), run the media tests and a GPU-encoder check on this PC. Updating FFmpeg later repeats the `sources.json` step. | Large: a Docker or WSL toolchain, a pinned script per library (BtbN's scripts are the obvious starting point and are MIT), a first build of hours, then re-testing on other GPUs. Pays back with Mac and Linux builds, which need an FFmpeg of their own anyway. |
| Risk | Depends on one person's reply and memory. | Daily auto-builds are deleted after about two weeks (monthly ones stay), so **keep our own copy of the zip we ship, and its `checksums.sha256` line**, in `dist-release/`. Depends on BtbN's repo staying up for the *scripts*; the dependencies' sources are fetched from their own upstreams. | Our time. |

**Recommendation: B for the first alpha, C later.** B is the only way that makes the gate pass this week without waiting for a stranger, keeps the same licence class and encoders, and costs mostly one session and ~136 MB of installer size. A is not worth waiting on: even a yes leaves the same problem at the next gyan.dev update. C is worth doing before Mac and Linux builds, and would shrink the installer, but is too big to put in front of the first alpha.

**Before swapping (Julian's OK first):** (1) a session downloads the build into its own folder, runs `ffmpeg -encoders -filters` and the `photos-media` FFmpeg tests, checks a video prep and a slideshow export on a temp library on this PC (NVIDIA/Intel/AMD, whichever it has), (2) rewrites `sources.json` for it (`ffmpeg-source` gate), (3) replaces the sidecars. Nothing under `src-tauri/binaries/` has been changed.

**If Julian chooses A, the message** (to go out from him; this session sent nothing and contacted no one):

> Hello — I'm shipping Lumen, a photo app, with your FFmpeg 9.0.1 essentials build as a separate program, under the GPL. To give the exact source I need the commits behind four libraries your README names: cairo 1.18.5, x265 4.3-6-g9ddc216, libva 2.25.0 and libgme 0.6.6. None matches a tag or a commit I can find upstream. Could you tell me the git commit (or tarball) each was built from, and whether you keep them for each release? Thank you for the builds.
