# Lumen

![Licence: GPL-3.0-or-later](https://img.shields.io/badge/licence-GPL--3.0--or--later-blue)

Created by Julian · JL Studios.

A free, open-source, local-first photo and video library and RAW developer for macOS, Linux and Windows. No account, no cloud, no telemetry, no paid tier. Your library is a folder you can back up with any tool.

Tauri 2 · Rust workspace · React 19 + TypeScript · Vite · Mantine · SCSS · Motion.

## Prerequisites

- Rust stable (1.98 or newer) via rustup
- Node 24 and Yarn 1
- Platform build deps for Tauri: on Linux `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`; Windows and macOS need nothing extra beyond the usual toolchains.
- **FFmpeg and ffprobe** for HEIC and video. Fetch the sidecars into `src-tauri/binaries/` once:

  ```sh
  pwsh scripts/fetch-ffmpeg.ps1     # Windows
  scripts/fetch-ffmpeg.sh           # macOS and Linux
  ```

  They are git-ignored. At runtime Lumen looks for them in `LUMEN_FFMPEG_DIR`, then next to the executable, then `src-tauri/binaries/` (debug builds), then `PATH` — so a system-wide FFmpeg works too. Without it, HEIC and video are still catalogued; Settings → Playback & codecs explains what is missing and re-checks on demand.
- **iPhone import** needs Apple's device service: the Apple Devices app or iTunes on Windows, `usbmuxd` on Linux, built in on macOS.

## Develop

```sh
yarn install
yarn tauri dev            # runs Vite on :1420 and the Rust host
```

## Verify

```sh
yarn typecheck            # tsc -b
yarn test                 # vitest
cd src-tauri
cargo test --workspace    # also regenerates src/types/generated via ts-rs
cargo clippy --workspace --all-targets -- -D warnings
```

Set `LUMEN_APP_DATA_DIR` and `LUMEN_DEFAULT_LIBRARY_DIR` to point a dev build at scratch folders.

```sh
# UI only, in a normal browser with a fake catalogue (no Rust needed)
yarn dev            # then open http://localhost:1420/library?library=1&nosplash=1

# Real-file end-to-end: import a folder into a scratch library and render every thumbnail size
LUMEN_E2E_FOLDER="<a folder of test images>" cargo test --release -p lumen --test e2e_import -- --ignored --nocapture
```

## What works today

Import from folders, memory cards (any volume with `DCIM/`) and iPhones over USB, all incremental — a card or phone is only asked for what it has not given before. Copy with BLAKE3 dedup, EXIF and ffprobe metadata, per-file failure reporting, Live Photos paired into one item.

Search that answers in single-digit milliseconds over a quarter-million items, smart albums that keep themselves current, ratings, and rubber-band selection.

A map of everywhere you have been, drawn offline from a bundled coastline with no API key and no tile server, with clustering in Rust and place names from an atlas compiled into the binary. HDR photos keep their dynamic range in the viewer.

A virtualized justified grid with month sections, thumbnails at three sizes generated on demand, HEIC and video through the FFmpeg sidecar (poster frames, playback proxies when the webview cannot play a codec), a viewer with bounded zoom, filmstrip, info panel and video controls, quick look, selection with context menus and batch actions, favorites on double-click, manual albums, a 30-day trash, device sections, and portable libraries you can carry on an external drive between Windows, macOS and Linux.

## Where things live

| What | Where |
|---|---|
| App settings, library registry, logs | `<config dir>/JL-Studios/Lumen/` (`%APPDATA%\JL-Studios\Lumen` on Windows) |
| Default library | `<Pictures>/Lumen Library/` |

## Docs

- Architecture: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- Product and UX: `docs/superpowers/specs/2026-09-17-lumen-ux-design.md`
- As-built notes per area: `docs/STATE.md`
- Project rules for agents and humans: `CLAUDE.md`
- Contributing: [CONTRIBUTING.md](CONTRIBUTING.md)
- Writing a plugin: [docs/PLUGINS.md](docs/PLUGINS.md); a theme: [docs/THEMES.md](docs/THEMES.md)
- Name and logo: `TRADEMARKS.md`

## Support Lumen

Lumen is free and stays free. If it is useful to you, you can support its development through GitHub Sponsors. It is optional and unlocks nothing.
