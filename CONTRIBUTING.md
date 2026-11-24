# Contributing to Lumen

Thank you for helping. Lumen is a free, GPL-3.0-or-later photo and video library and RAW developer, kept alive by the people who use it. This page is the short version; [CLAUDE.md](CLAUDE.md) holds the rules in full and is written for people as much as for AI assistants.

## Before you start

- **Small fix or clear bug:** open a pull request.
- **Anything larger:** open an issue or a Discussion first. Large work follows a short written plan in the issue, and the plan comes before the code. Look for the *good first issue* label if you want somewhere gentle to begin.
- **A feature that could be a plugin** (an importer, theme, preset, look or app link) probably should be. See [docs/PLUGINS.md](docs/PLUGINS.md).
- Lumen has no account, no cloud, no telemetry and no paid tier, and that does not change. A feature that needs any of them will not be merged.

## Sign off your commits (DCO)

Lumen uses the [Developer Certificate of Origin](https://developercertificate.org/), not a CLA. Add a sign-off to every commit:

```sh
git commit -s -m "Fix the date order on the Develop filmstrip"
```

That adds `Signed-off-by: Your Name <you@example.com>`, which says you wrote the change or have the right to contribute it under the project's licence. A check on every pull request fails if a commit lacks it; `git rebase --signoff main` fixes an existing branch. Your contribution is licensed under `GPL-3.0-or-later`, except in the folders that are `MIT OR Apache-2.0` (the plugin SDK and the plugin WIT) and `photos-raw` (`LGPL-2.1`).

## Build and test

You need Rust stable, Node 24 and Yarn 1; on Linux also `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`. [README.md](README.md) has the details, including FFmpeg.

```sh
yarn install
yarn tauri dev                 # run the app
yarn typecheck                 # tsc -b
yarn test                      # vitest
cd src-tauri
cargo test --workspace         # also regenerates src/types/generated; commit what it changes
cargo clippy --workspace --all-targets -- -D warnings
```

`yarn dev` shows the UI in a browser with a fake catalogue, and `/dev/ui` lists every control. After changing dependencies run `yarn run licenses` and commit `src/assets/licenses/`.

## How we write code

- **Test first.** Write the failing test, watch it fail, then make it pass. Rust crates and frontend hooks and components are held to this.
- **Folder convention:** each component or route is a folder with `gui.tsx`, `styles.scss` and `index.ts`; imports use `@/`; no importing another folder's stylesheet.
- **The non-negotiables** are in [CLAUDE.md](CLAUDE.md#non-negotiables): two transports (metadata over `invoke`, bytes over `photos://`), one-way crate dependencies, Rust does the work, thin commands, generated TypeScript types, keyset pagination, colour only in tokens, originals never modified.
- **A user's library is irreplaceable.** Tests and synthetic libraries go in temp folders. Never point an import, export or replay at a real library or a device, and never write to `library.db` directly. Run replays on a copy.
- **No personal data in a commit:** no real names, photos, paths or device names in code, tests, fixtures or documents. CI runs a check (`node scripts/oss/scrub.mjs`) that fails on them; you can run it yourself on the files you changed.
- Commit messages: an imperative one-line summary, and a body that says *why* when it is not obvious.

## Reviews

A maintainer reviews every pull request. Expect questions about tests and about what happens to a user's originals. Maintainers are people who have given sustained, careful reviews; see [GOVERNANCE.md](GOVERNANCE.md).

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Security problems go through [SECURITY.md](SECURITY.md), not a public issue.
