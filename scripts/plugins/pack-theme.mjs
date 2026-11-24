#!/usr/bin/env node
// Packs a theme folder into a `.lumentheme` (Round 26 goal 4, docs/THEMES.md).
//
//   node scripts/plugins/pack-theme.mjs <folder> [--out <file>]
//
// The zip is deterministic (same bytes for the same files: sorted, fixed time and mode), so the
// index's blake3 stays put until the theme changes. No signature: a store theme is held to the
// index's blake3, and Lumen checks every token and every CSS rule on install. Without --out the
// file goes to plugins/store/dist/<folder name>.lumentheme (git-ignored), where the dev store
// (LUMEN_STORE_DIR / Developer mode) reads it. Lumen stays the authority on what it accepts; this
// refuses what it can see cheaply: files a theme may not carry, sizes, no theme.toml.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { collectFolder, PackError, zipFiles } from './pack.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
export const DIST = path.join(here, '..', '..', 'plugins', 'store', 'dist');
export const MAX_BYTES = 20 * 1024 * 1024;
export const MAX_FONT = 512 * 1024;
const MAX_IMAGE = 2 * 1024 * 1024;

const fail = (msg) => {
  throw new PackError(msg);
};

/** The files a `.lumentheme` may hold (theme_pkg.rs `allowed_file`). */
export function allowedFile(rel) {
  const lower = rel.toLowerCase();
  const ext = (...e) => e.some((x) => lower.endsWith(x));
  if (['theme.toml', 'theme.css', 'icon.png', 'readme.md', 'license', 'notices'].includes(lower)) return true;
  if (lower.startsWith('fonts/')) return ext('.woff2', '.txt');
  if (lower.startsWith('images/') || lower.startsWith('screenshots/')) return ext('.png', '.jpg', '.jpeg', '.webp');
  return false;
}

/** Checks a theme's files ([path, Buffer][]); throws PackError on the first problem. */
export function checkTheme(files) {
  const names = new Map(files);
  if (!names.has('theme.toml')) fail('The theme has no theme.toml');
  let total = 0;
  for (const [rel, data] of files) {
    if (!allowedFile(rel)) fail(`${rel}: a theme carries only theme.toml, theme.css, icon.png, README.md, LICENSE, NOTICES, fonts/, images/ and screenshots/`);
    total += data.length;
    if (rel.toLowerCase().endsWith('.woff2') && data.length > MAX_FONT) fail(`${rel} is over 512 KB`);
    if (/^(images|screenshots)\//i.test(rel) && data.length > MAX_IMAGE) fail(`${rel} is over 2 MB`);
  }
  if (total > MAX_BYTES) fail('A theme can be at most 20 MB');
}

export function packTheme(folder, { outFile } = {}) {
  const dir = path.resolve(folder);
  if (!fs.statSync(dir).isDirectory()) fail(`${folder} isn't a folder`);
  const files = collectFolder(dir);
  checkTheme(files);
  const out = path.resolve(outFile ?? path.join(DIST, `${path.basename(dir)}.lumentheme`));
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, zipFiles(files));
  return { out, files: files.length };
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  const args = process.argv.slice(2);
  const i = args.indexOf('--out');
  const outFile = i >= 0 ? args.splice(i, 2)[1] : undefined;
  try {
    if (args.length !== 1) fail('usage: pack-theme.mjs <folder> [--out <file>]');
    const r = packTheme(args[0], { outFile });
    console.log(`${r.out} (${r.files} files)`);
  } catch (e) {
    console.error(`error: ${e.message}`);
    process.exitCode = 1;
  }
}
