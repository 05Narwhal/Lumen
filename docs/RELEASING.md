# Releasing Lumen

Round 17; one release path since release simplification phase 2 (2026-11-14). A release is **a tag**: pushing `v26.X.Y-<channel>` makes `.github/workflows/release.yml` check, build and publish it. Nothing is built or published from a PC. Windows only for now: the updater code is cross-platform and other platforms simply find no entry in a channel's manifest, and macOS / Linux artefacts wait for a self-built FFmpeg with its source (spec decision 4a, then 4d). On those systems people build from source.

## Versions

`26.X.Y-<channel>`: **X** a major release (only Julian says when it goes up; Y resets to 0), **Y** most updates, **channel** `alpha`, then `beta`, then `stable`. Every published build gets a new Y, promotions included, so one number is one build. A plain `26.1.0` is never published: semver ranks it above `-stable`.

**The order is checked within a line (`26.X`), not globally.** X goes up with the first alpha after a stable, so `26.3.5-stable` < `26.4.0-alpha`. A stable hotfix `26.3.5-stable` follows `26.3.4-stable` even when `26.4.2-alpha` is already out; `26.4.0-alpha` follows the newest `26.3.*`. A line with no build yet must start at Y = 0, and a line cannot be started behind a newer one (`previousInLine`, `isNextVersion`).

The version lives in `package.json`, `src-tauri/tauri.conf.json` (plus the numeric WiX version), the workspace `src-tauri/Cargo.toml`, and Lumen Setup's three fields (`installer/`). Change it only with:

```sh
node scripts/release/set-version.mjs 26.1.1-alpha
```

`src-tauri/tests/version_fields.rs` fails if the fields disagree.

## Making a release

1. On `main`: `node scripts/release/set-version.mjs 26.1.1-alpha`, write its section in `docs/CHANGELOG.md` (the text Lumen shows as the notes), run `yarn run licenses`, commit.
2. Push `main` and wait for CI (`frontend`, `scripts`, `rust (windows-latest)`) to be green on that commit: the `checks` gate reads it.
3. Tag and push: `git tag v26.1.1-alpha && git push origin v26.1.1-alpha`.

That is all. A tag whose gates fail stops before anything is built or published; fix it, delete the tag and tag the corrected commit (the version number can be reused only if nothing was published: a draft release for the tag is deleted first).

## Shipping a security fix

A fix people should get without asking installs by itself (Settings → Updates → *Install security fixes automatically*, on by default): Lumen downloads it in the background and installs it at the next close or the next start, before the library opens, once per version. It never restarts Lumen while it is open. What makes a build a *security fix* is one field in the channel manifests, `minSafe`, and everything below exists to set it safely.

**A stable line lives on `release/26.X`.** When `26.X.Y-stable` ships, cut the branch from its tag and push it (branch protection on `release/*`, see *One-time GitHub setup*):

```sh
git branch release/26.3 v26.3.4-stable && git push origin release/26.3
```

**To ship a fix:**

1. Prepare it privately if the problem is not public yet: the repository's *Security* → *Advisories* → the report → *Start a temporary private fork*. Merge it into the public repository only when the release is ready.
2. Land the fix on `main` first (pull request, DCO sign-off, CI green).
3. Cherry-pick it onto `release/26.X` (`git cherry-pick -x <sha>`, by pull request into the branch). **No migration:** the `no-migration` gate refuses a security build whose `photos-core/migrations` differs from the line's previous tag, because a silent install must never rewrite a schema on a stable line. If the fix needs one, ship it as an ordinary release instead.
4. On `release/26.X`: `node scripts/release/set-version.mjs 26.X.(Y+1)-stable`, and write the section as **`## 26.X.(Y+1)-stable · security`** in `docs/CHANGELOG.md` (plain text for the person using Lumen; say what was wrong in one honest sentence). Add the same section to `main`'s changelog too, below the newer entries. Commit, push, wait for CI on that commit.
5. Tag from the branch: `git tag v26.X.(Y+1)-stable && git push origin v26.X.(Y+1)-stable`. The workflow runs every gate as for any tag (`tag-ref` accepts a commit on `release/*`; `next-version` compares within the line).
6. Tag the next alpha (and beta, if one is out) from `main` with the fix in it, as usual. A preview may carry a migration; the safety copy still runs first.

**What the release step does.** The `· security` heading makes `channels.mjs` set `"minSafe": "26.X.(Y+1)-stable"` in `stable.json`, `beta.json` and `alpha.json`, the channels a stable build reaches. A channel that holds a *newer* build (a `26.4.2-alpha`) keeps it (never lower, above) and only gains the `minSafe`. `minSafe` is copied forward by every later release and only ever goes up, so someone who skipped the fix is still caught by a later ordinary manifest. A person running a build **below** `minSafe` is offered the channel's *newest* build as a security update, even with *Automatic updates* off (the manifest names one build, not the exact patched one). A build at or above `minSafe` is untouched.

**Check before you tag.** Is the section's heading exactly `· security`? Is `migrations/` unchanged (`git diff v26.X.Y-stable HEAD -- src-tauri/crates/photos-core/migrations`)? A tag with the heading but a failing gate stops before anything is published: fix and re-tag as in *Making a release*. After publishing, open `https://github.com/05Narwhal/lumen/releases/download/channels/stable.json` and check `version` and `minSafe`.

**Rolling `minSafe` back.** It should not be needed. If a `· security` mark was wrong, publish the next build without the marker: `minSafe` stays (it only goes up), so a wrong mark is fixed by making the marked build actually safe, not by editing the `channels` release by hand.

### One-time GitHub setup (Julian)

Nothing to buy, no new key, secret or repository: the same updater key signs security builds and they use the same `channels` release.

1. **Branch protection on `release/*`**, like `main`: Settings → Branches → *Add branch ruleset* (or classic rule) with the pattern `release/*`: require a pull request, require the CI checks `frontend`, `scripts` and `rust (windows-latest)`, require signed-off commits (the DCO check), block force pushes and deletion. Without it a stable line can be rewritten.
2. **Private vulnerability reporting on**: Settings → Code security → *Private vulnerability reporting* → Enable (`SECURITY.md` sends reporters to it).
3. **Security advisories**: used to prepare a fix in a *temporary private fork* before it is public (step 1 above); nothing to switch on beyond step 2.

## What the workflow does

| Job | Does |
|---|---|
| `ffmpeg-sources` | Uploads the exact source of the FFmpeg build Lumen ships (and ExifTool's) once, as the release `ffmpeg-sources-n9.0.2-47`, made by `scripts/release/ffmpeg-sources.mjs` from `sources.json`; skipped when that release exists. The GPL asks for this source; every other source is the tagged repository. |
| `notes` | The version's section of `docs/CHANGELOG.md` plus a fixed footer: **`Lumen-Setup-<version>.exe` is the download**, the NSIS installer is in the release because the updater installs it, Windows only, unsigned (SmartScreen warns). |
| `preflight` | Windows. Fetches the sidecars (composite action `.github/actions/windows-sidecars`) and runs `node scripts/release/preflight.mjs before <version>`: the gates in the first block below. |
| `build` | Windows. `node scripts/release/setup-build.mjs <version>` (the order below), then `preflight.mjs after`: the second block, then makes the **draft** release and uploads the NSIS installer, its `.sig` and `Lumen-Setup-<version>.exe`. |
| `publish` | Publishes the draft, then `scripts/release/channels.mjs` points the channel manifests at it, last, so a half-done release is never offered. One channel update runs at a time across tags (`concurrency: release-channels`), because it reads each manifest before writing it. |

### Preflight gates

`scripts/release/preflight.mjs` (`evaluatePreflight`, pure; the CLI gathers the facts from the checkout, `git`, `gh`, `cargo` and the sidecar) prints each gate and exits 1 on any failure. Rules are in `gates.mjs`.

| Gate | Phase | Holds when |
|---|---|---|
| `version`, `version-fields`, `next-version` | before | The tag follows the scheme, every field says it, and it comes after the last published release **of its own line** (`26.X`). |
| `no-migration` | before | Only for a security build (changelog heading `· security`) that is `-stable` or on `release/*`: `src-tauri/crates/photos-core/migrations` is identical to the line's previous tag, so a silent install never rewrites a schema on a stable line. It refuses when git cannot compare. A preview, or a build that is not a security fix, always passes. |
| `tag-ref` | before | The tag is `v<version>` and its commit is on `main` or `release/*`. |
| `changelog` | before | `docs/CHANGELOG.md` has `## <version>`. |
| `notices` | before | `yarn run licenses` changes nothing. |
| `checks` | before | CI's `frontend`, `scripts` and `rust (windows-latest)` jobs succeeded on the tagged commit (`checksFromRuns`). |
| `env` | before | None of `TEST_OVERRIDES` (`LUMEN_BAKED_*`, `LUMEN_UPDATE_TEST_BASE`, `LUMEN_SETUP_PRODUCT`, `LUMEN_SETUP_IDENTIFIER`) is set: they make a build use other folders or another update server. |
| `rawler` | before | `cargo tree -i rawler` shows only `photos-raw` depending on it, and its library is in `bundle.resources` (the replaceable library). |
| `ffmpeg-source` | before | The FFmpeg sidecar is the build `scripts/release/sources.json` records (version and SHA-256), **every `--enable-*` in its own configuration** has its source listed, and each source file is in the source release with its SHA-256. A new FFmpeg build with a new library refuses until its source is listed. |
| `source-bundle` | before | ExifTool and Strawberry Perl sources are in the source release (`photos-raw`'s source is the tagged repository). |
| `library-check` | after | The built `lumen.exe --check-library` opens every **synthetic** library, one per published schema (`PUBLISHED_SCHEMAS` in `src-tauri/tests/library_fixtures.rs`, generated into a temp folder; never a real library). Julian can still run `--check-library` on a **copy** of his own library by hand; it is not a gate. |
| `setup-sizes` | after | The full Setup is at most the NSIS installer + 6 MiB, the slim Setup at most 6 MiB (`setupSizeGate`). |

Not gates any more: `clean`, `main` (a tag in CI is clean, and `tag-ref` covers the branch), `signing` (the build fails without the key), `releases-repo`, `sidecars` as a local check, `stable-only` (a `-stable` tag is held to the same gates as any other).
## Where releases live

Releases live in the public source repository `05Narwhal/lumen`; the models stay in the public `05Narwhal/lumen-models`. The updater's default URL, `publish.mjs` and `channels.mjs` name `05Narwhal/lumen`. **The local `26.1.1-alpha` backup build still polls `05Narwhal/lumen-releases`, so it never sees an update: reinstall it once from a build of the new repo.**

Each build is a release `v26.1.1-alpha` (a GitHub pre-release unless `-stable`) with the NSIS installer, its `.sig` and the full Lumen Setup (`Lumen-Setup-<version>.exe`). A permanent release named `channels` holds `alpha.json`, `beta.json` and `stable.json`, each in `tauri-plugin-updater`'s static format and naming the newest build that channel may take. A `-stable` updates all three channels, a `-beta` updates beta and alpha, an `-alpha` updates alpha. Lumen reads `https://github.com/05Narwhal/lumen/releases/download/channels/<channel>.json`; no token is in the app.

**A channel is never lowered.** `channels.mjs` reads each manifest it would write first and writes only when the new build is not lower, so a `26.3.5-stable` hotfix published after `26.4.2-alpha` updates `stable.json` and leaves `alpha.json` (and a higher `beta.json`) alone. Re-running the same version is allowed (it finishes a half-done update). A manifest that cannot be read stops the whole update rather than risk overwriting it; the `channels` release is never edited by hand. A manifest with no `minSafe` is exactly what it was before the security track.

**Why a channel with no build holds a placeholder.** It holds `0.0.0-<channel>`, a version below every real one, so no install takes it. The placeholder is why a 404 can be read as *could not check* (`check_fail`) rather than as "no update": every manifest exists from the first publish on (`updateChannels`, `placeholderManifest`).

The third-party source is no longer gathered per release: there is no written offer of source and no `photos-raw` archive. The source is public, Settings → Licences links to `https://github.com/05Narwhal/lumen`, and the FFmpeg source release above carries the one part that is not in the repository.

## Keeping Actions free

GitHub Actions minutes are free and unlimited on a **public** repository and metered on a private one (2,000 a month, Windows at 2x, macOS at 10x). So every workflow (CI, `release.yml`, dry runs) runs only on the public `05Narwhal/lumen`; a private repository, including a scratch repository for a dry run, has Actions turned off. A dry run goes in a **public** scratch repository and is deleted afterwards. In the account's Billing → Budgets and alerts, keep the Actions budget at $0 with *Stop usage when the budget is exceeded*, so nothing can be billed. Dependabot is set to one grouped monthly pull request per ecosystem, no major versions (`.github/dependabot.yml`), because each of its pull requests is an email.

## Installer and Lumen Setup

A release carries two Windows files: `Lumen_<version>_x64-setup.exe` (the NSIS installer; the channel manifests and the updater point **only** at this one) and `Lumen-Setup-<version>.exe` (the full Lumen Setup, the branded window that runs that installer silently: **the file people download**). The build runs in `scripts/release/setup-build.mjs`, whose order is fixed by one rule, that the payload sits inside the signed file and nothing is appended after signing (the workflow runs it instead of `tauri-action`, so the order stays in one tested script):

1. `vite build` for Setup's window, then the **slim Setup** (`installer/src-tauri`, no payload, about 5 MB), signed.
2. `tauri build --bundles nsis` with a `--config` overlay file (`dist-release/<version>/nsis-overlay.json`) that adds the slim Setup as a bundle resource landing at **`$INSTDIR\lumen-setup.exe`** (exactly that name: the hook `NSIS_HOOK_POSTINSTALL` makes it Lumen's uninstaller only when it is there) and the updater artefacts. Tauri signs the installer **during** this build (`bundle.windows.signCommand`), because the updater's minisign `.sig` is taken over the finished file; signing afterwards would invalidate it.
3. The installed size is added up from the merged Tauri config (Lumen, the sidecars, every resource, the slim Setup, the uninstaller) and baked into the full Setup as `LUMEN_SETUP_INSTALLED_BYTES`; Setup's free-space check and the progress bar use it (installed size + the installer written to temp + 5 %).
4. The **full Setup** is built with `--features payload` and `LUMEN_SETUP_PAYLOAD=<the signed installer>` (`include_bytes!`), signed, and written beside the installer as `Lumen-Setup-<version>.exe`.

It writes `dist-release/<version>/sizes.json` (what `setup-sizes` reads) and `built.json`; `collectArtifacts` refuses an installer without its `.sig`. The updater's key comes from the environment as the key's **text** (`TAURI_SIGNING_PRIVATE_KEY`), never from a file path (`updaterKeyFromEnv`), so the same build runs anywhere there is the secret.

**Windows code signing (SignPath placeholder).** `setup-build.mjs` signs with `signtool` when `LUMEN_SIGN_CERT_SHA1` names a certificate (`LUMEN_SIGN_TIMESTAMP_URL` overrides DigiCert's timestamp server). CI has none, so releases are unsigned and the notes say SmartScreen will warn. SignPath Foundation plugs into `signCommand` (`tauriSignCommand`) once the project is approved; the order above does not change. It is separate from the updater's minisign key.

**Lumen Setup's version** is `installer/package.json`, `installer/src-tauri/tauri.conf.json` and `installer/src-tauri/Cargo.toml`; `set-version.mjs` writes them and `src-tauri/tests/version_fields.rs` fails if they disagree with Lumen's (Setup shows that version and compares it with the installed one).

**Test identities never reach a release.** `LUMEN_SETUP_PRODUCT` and `LUMEN_SETUP_IDENTIFIER` (a *Lumen Test* Setup) join `LUMEN_BAKED_*` in the `env` gate, and the build clears `LUMEN_SETUP_PAYLOAD` / `LUMEN_SETUP_INSTALLED_BYTES` for the slim Setup.

**Third-party notices.** Setup's Rust and npm dependencies are all already in `src/assets/licenses/notices.json` (checked 2026-11-06: tauri, tauri-plugin-dialog, reqwest, tokio, winreg, windows-sys, sysinfo, ts-rs, react, Inter), and the full Setup carries Lumen's installer with its Licences screen, so `scripts/licenses/generate.mjs` still reads only `src-tauri/Cargo.toml`. A new Setup-only dependency must be added to that script's graph first.

**Workspace or standalone:** standalone (decided in phase 3: cold test build 1 min 16 s; Cargo honours `[profile.release]` only at a workspace root and Setup needs `opt-level = "s"` + LTO + `panic = "abort"`). A cold release build of Setup takes about 4.5 min with its own `installer/src-tauri/target`; Lumen's builds are not slowed by it.

## The signing key (once)

The updater only runs files signed by Lumen's key (minisign, through `tauri signer`), and Tauri accepts **one** public key (`plugins.updater.pubkey`). There is one updater keypair and no second key. **Losing the key strands every install**: they could never update again. So:

1. **Make it** in your own terminal (it asks for a password; use a long random one from a password manager): `yarn tauri signer generate -w $env:USERPROFILE/.lumen-release/lumen.key`
2. **Commit only the public key**: `lumen.key.pub`'s content goes in `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, and never changes. Never commit, paste or log the private key or its password.
3. **Keep it in three places.** (a) The GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` (the key file's **text**) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: write-only, they cannot be read back, so never the only copy. (b) A password-manager entry holding the key file's text and its password, with the manager's emergency access set for the successor (see `GOVERNANCE.md`). (c) Two offline copies of the encrypted key file: a USB stick in a safe and a printout (it is a few hundred characters), with the password stored apart from them.
4. **The plugin key** (the *From JL Studios* package signature; `src-tauri/plugin-pubkey.txt` is its public half, tracked) is a separate minisign key with a separate blast radius, kept in the same three places. The Discord application id is tracked in `src-tauri/discord_client_id.txt`; an environment value overrides either file.

**Changing the password** without changing the key: `minisign -C -s lumen.key`, then update the GitHub secret and the manager entry. **Changing the key** is only for a leak: ship a release signed with the old key that carries the new public key, then retire the old one.

CI signs with the two secrets (`.github/workflows/release.yml`). There is no local release path. To try a build on a PC, put the key's **text** in that terminal's environment only (`$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content ... -Raw`, never into a file in the repository) and run `node scripts/release/setup-build.mjs <version>`; it builds into `dist-release/<version>/` (git-ignored) and publishes nothing.

## Before going public (checklist)

Checked in Round 26 A2 (nothing was pushed, tagged or exported for real):

- [x] `node scripts/oss/export-public.mjs <new empty folder>` ends with no finding (the new plugin and theme icons and screenshots are allowed by `mediaAllow`; the five CC0 Wikimedia photos in `photos-ml/tests/fixtures/wayfinder/` keep their photographers' Artist / Copyright tags through `exifAllow`, credited in that folder's `SOURCES.md`). Re-run it on the final commit: the scrub is the gate.
- [x] README, CONTRIBUTING and GOVERNANCE point only at files that are in the snapshot; `docs/ARCHITECTURE.md` is the scrubbed public copy of the architecture spine. `CLAUDE.md`, `docs/STATE.md` and the specs still name plan files under `docs/superpowers/plans/`, which the snapshot leaves out; they are maintainer notes and are left as they are.
- [x] **Updater endpoint:** nothing to change. `tauri.conf.json` keeps `endpoints: []` on purpose; `updater.rs` sets the endpoint at run time to `https://github.com/05Narwhal/lumen/releases/download/channels/<channel>.json` (the public repo's rolling `channels` release, written by `scripts/release/channels.mjs`).
- [x] **EOX credit:** the satellite layer's own attribution text (*EOxCloudless … by EOX IT Services GmbH, Contains modified Copernicus Sentinel data*) is built into `src/lib/map-style.ts` and shown on the map; the layer is listed under Settings → *Licences*; `docs/LICENSING-AUDIT.md` records CC BY-NC-SA 4.0 and the stop rule.

Still yours:

1. **Discord:** read the current [Discord Developer Policy](https://support-dev.discord.com/hc/en-us/articles/8563934450327-Discord-Developer-Policy) and Terms for the Application in `src-tauri/discord_client_id.txt` (name, icon, the *Competing in Lumen* status, nothing about photos). I could not confirm a Rich Presence clause from the web, so this is a read, not a finding. Run the status once against a real Discord (not done in any round).
2. The signing key, its two secrets and the plugin key (*The signing key*, above); `actionlint` and one dry run of `release.yml` on a throwaway tag in a fork.
3. Re-pack Disk Atlas, Wayfinder, Takeout, Film Lab, Clockwork and Captioner with `--key %APPDATA%/JL-Studios/plugin-keys/jl-studios.key`, re-run `add-plugin.mjs` for each, create `05Narwhal/lumen-plugins` and its first release (the index URLs must match the tag and the bytes), and upload the Wayfinder model files as `wayfinder-model-1.0`. Rebuild Wayfinder (its `Cargo.lock` still names `lumen-plugin 1.3.0`).
4. The export's manual steps (printed at the end of the dry run): rename the private repo to `lumen-dev-archive`, create the public `05Narwhal/lumen`, push the snapshot, make `lumen-models` public, turn on Discussions, private vulnerability reporting and branch protection (CI and DCO required), apply to SignPath Foundation, set up GitHub Sponsors.
5. Real-app checks still open from earlier rounds (see `CLAUDE.local.md`): a `.lumentheme` double-click, Appearance *Remove* / *Undo*, a layout theme in WebView2, Clockwork on a copy of your library, Wayfinder's NEF / HEIC write on a real file.

## Before the first tag

- The signing key above exists and its two secrets are set in `05Narwhal/lumen`.
- `05Narwhal/lumen` is public.
- `actionlint` has been run on `.github/workflows/release.yml` (phase 2 could not run it) and the Windows job has had one dry run on a throwaway tag in a fork (nothing was run in CI yet; see the plan's phase 2 hand-off).
- The CPM *Publish release* script lives outside this repository and calls the deleted `publish.ps1`: remove it from the Lumen project.

The decision (spec 4a): keep bundling BtbN's build with its 782 sources on Windows, ship Windows only, build a trimmed FFmpeg in CI later (4d, its own spec).
