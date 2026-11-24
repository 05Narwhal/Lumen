#!/usr/bin/env node
// Adds or updates a theme's entry in index.json from a packed .lumentheme (Round 26).
//
//   node plugins/store/add-theme.mjs <file.lumentheme> --url <release asset url>
//        [--index <index.json>] [--jl] [--summary <text>] [--icon <url>] [--homepage <url>]
//        [--readme <url>] [--screenshots <url>,<url>,…] [--tags dark,layout,…] [--featured]
//
// id, name, author, version, the size and blake3 come from the package; `tokens` (the swatches the
// store's miniature is painted with) from theme.toml's [light] / [dark] tables, anything they don't
// set from the base theme (base-swatches.json, written by a Rust test so it matches what Lumen
// computes). Tags default to theme.toml's [listing] tags, plus `layout` when there's a theme.css.
// `api` is the theme format's: 1.4.0. --featured makes it the theme the Themes tab leads with.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { loadPackTools, validateIndex } from './validate.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const TAGS = ['dark', 'light', 'layout', 'colourful'];
/** Swatch field ← theme token (theme_pkg.rs `swatch`). */
const SWATCH = {
  bg: 'bg-base',
  surface: 'bg-raised',
  sidebar: 'bg-sidebar',
  fg: 'fg-primary',
  fg2: 'fg-muted',
  accent: 'accent',
  placeholder: 'bg-sunken',
  radius: 'radius-md',
  gap: 'tile-gap',
};

/** Just enough TOML for theme.toml: `key = "text"`, one-line string arrays, `[table]`. */
export function readThemeToml(text) {
  const out = { '': {} };
  let table = '';
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.replace(/\s+#.*$/, '').trim();
    if (!line || line.startsWith('#')) continue;
    const head = /^\[([A-Za-z0-9_.-]+)\]$/.exec(line);
    if (head) {
      table = head[1];
      out[table] ??= {};
      continue;
    }
    const kv = /^([A-Za-z0-9_-]+)\s*=\s*(.+)$/.exec(line);
    if (!kv) continue;
    const [, k, v] = kv;
    if (v.startsWith('[')) out[table][k] = [...v.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map((m) => m[1]);
    else if (v.startsWith('"')) out[table][k] = /^"((?:[^"\\]|\\.)*)"/.exec(v)?.[1] ?? '';
    else out[table][k] = v;
  }
  return out;
}

/** The miniature's swatches: theme.toml's tokens over the base's. */
export function tokensOf(toml, bases = JSON.parse(fs.readFileSync(path.join(here, 'base-swatches.json'), 'utf8'))) {
  const base = bases[toml[''].base ?? 'gallery'];
  if (!base) throw new Error(`theme.toml: base "${toml[''].base}" isn't one of ${Object.keys(bases).join(', ')}`);
  const scheme = (s) => Object.fromEntries(Object.entries(SWATCH).map(([k, tok]) => [k, toml[s]?.[tok] ?? base[s][k]]));
  return { light: scheme('light'), dark: scheme('dark') };
}

/** The entry for a package's bytes. `opts`: url, jl, summary, icon, homepage, readme, screenshots, tags, released. */
export async function entryFromTheme(bytes, opts, pack) {
  const { blake3 } = await import('@noble/hashes/blake3.js');
  const files = pack.unzipFiles(bytes);
  const raw = files.get('theme.toml');
  if (!raw) throw new Error('The package has no theme.toml');
  const toml = readThemeToml(raw.toString('utf8'));
  const top = toml[''];
  for (const k of ['id', 'name', 'version', 'author']) if (!top[k]) throw new Error(`theme.toml has no ${k}`);
  const listed = Array.isArray(toml.listing?.tags) ? toml.listing.tags : [];
  const tags = opts.tags ?? [...new Set([...listed, ...(files.has('theme.css') ? ['layout'] : [])])].filter((t) => TAGS.includes(t));
  return {
    id: top.id,
    name: top.name,
    author: top.author,
    jl: Boolean(opts.jl),
    summary: opts.summary ?? top.description ?? top.tagline ?? top.name,
    ...(opts.icon ? { icon: opts.icon } : {}),
    ...(opts.homepage ? { homepage: opts.homepage } : {}),
    ...(opts.readme ? { readme: opts.readme } : {}),
    ...(opts.screenshots?.length ? { screenshots: opts.screenshots } : {}),
    tags,
    tokens: tokensOf(toml),
    latest: {
      version: top.version,
      api: '1.4.0',
      url: opts.url,
      blake3: Buffer.from(blake3(bytes)).toString('hex'),
      size: bytes.length,
      released: opts.released ?? new Date().toISOString().slice(0, 10),
    },
  };
}

/** Replaces the theme with the same id, or appends. Returns a new index. */
export function upsertTheme(index, entry) {
  const themes = index.themes ?? [];
  const next = themes.some((t) => t.id === entry.id) ? themes.map((t) => (t.id === entry.id ? entry : t)) : [...themes, entry];
  return { ...index, themes: next };
}

async function main(argv) {
  const o = { _: [] };
  const valued = ['url', 'index', 'summary', 'icon', 'homepage', 'readme', 'screenshots', 'tags'];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--jl') o.jl = true;
    else if (a === '--featured') o.featured = true;
    else if (a.startsWith('--') && valued.includes(a.slice(2))) o[a.slice(2)] = argv[++i];
    else if (a.startsWith('--')) throw new Error(`Unknown option ${a}`);
    else o._.push(a);
  }
  if (o._.length !== 1 || !o.url) throw new Error('usage: add-theme.mjs <file.lumentheme> --url <release asset url> [options]');
  if (o.screenshots) o.screenshots = o.screenshots.split(',').map((u) => u.trim());
  if (o.tags) o.tags = o.tags.split(',').map((t) => t.trim());
  const indexFile = path.resolve(o.index ?? path.join(here, 'index.json'));
  const index = JSON.parse(fs.readFileSync(indexFile, 'utf8'));
  const entry = await entryFromTheme(fs.readFileSync(o._[0]), o, await loadPackTools());
  const next = upsertTheme(index, entry);
  if (o.featured) next.featuredTheme = entry.id;
  const errors = validateIndex(next);
  if (errors.length) throw new Error(`The entry would not validate:\n  ${errors.join('\n  ')}`);
  fs.writeFileSync(indexFile, `${JSON.stringify(next, null, 2)}\n`);
  console.log(`${entry.id} ${entry.latest.version} → ${indexFile}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  main(process.argv.slice(2)).catch((e) => {
    console.error(`error: ${e.message}`);
    process.exitCode = 1;
  });
}
