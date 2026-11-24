#!/usr/bin/env node
// Adds or updates a plugin's entry in index.json from a packed .lumenplugin (Round 24 C3).
//
//   node plugins/store/add-plugin.mjs <file.lumenplugin> --url <release asset url>
//        [--index <index.json>] [--jl] [--api 1.0.0|1.1.0] [--summary <text>] [--icon <url>]
//        [--homepage <url>] [--kinds theme,tools,…] [--readme <url>] [--screenshots <url>,<url>,…]
//        [--featured]
//
// id, name, author, version, permissions and the size and blake3 come from the package itself.
// The manifest's `api = 1` is written as "1.0.0"; pass --api 1.1.0 for a plugin that uses the
// 1.1 interfaces. --jl claims "By JL Studios" (validate.mjs --download refuses it unless the
// package is signed by the JL key). --readme, --icon and --screenshots are the https URLs the
// package's README.md, icon.png and screenshots/ files are published at (Lumen downloads and caches
// them; the page never loads them from the web); authorUrl and repository come from plugin.toml.
// --featured makes the plugin the one Discover leads with. Run validate.mjs on the result.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { loadPackTools, manifestInfo, validateIndex } from './validate.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));

const KIND_OF_TABLE = { themes: 'theme', presets: 'presets', apps: 'apps', prompts: 'prompts', importer: 'importer', tools: 'tools' };

/** The entry for a package's bytes. `opts`: url, jl, api, summary, icon, homepage, kinds, released. */
export async function entryFromPackage(bytes, opts, pack) {
  const { blake3 } = await import('@noble/hashes/blake3.js');
  const files = pack.unzipFiles(bytes);
  const raw = files.get('plugin.toml');
  if (!raw) throw new Error('The package has no plugin.toml');
  const info = manifestInfo(raw.toString('utf8'));
  for (const k of ['id', 'name', 'version', 'author']) if (!info[k]) throw new Error(`plugin.toml has no ${k}`);
  const kinds = opts.kinds ?? [...new Set(info.tables.map((t) => KIND_OF_TABLE[t]).filter(Boolean))];
  const entry = {
    id: info.id,
    name: info.name,
    author: info.author,
    jl: Boolean(opts.jl),
    summary: opts.summary ?? info.description,
    kinds,
    permissions: info.permissions,
    ...(opts.icon ? { icon: opts.icon } : {}),
    ...(opts.homepage ? { homepage: opts.homepage } : {}),
    ...(opts.readme ? { readme: opts.readme } : {}),
    ...(opts.screenshots?.length ? { screenshots: opts.screenshots } : {}),
    ...(info.author_url ? { authorUrl: info.author_url } : {}),
    ...(info.repository ? { repository: info.repository } : {}),
    latest: {
      version: info.version,
      api: opts.api ?? (info.api === '1' ? '1.0.0' : info.api),
      url: opts.url,
      blake3: Buffer.from(blake3(bytes)).toString('hex'),
      size: bytes.length,
      released: opts.released ?? new Date().toISOString().slice(0, 10),
    },
  };
  return entry;
}

/** Replaces the entry with the same id, or appends. Returns a new index. */
export function upsert(index, entry) {
  const plugins = index.plugins.some((p) => p.id === entry.id)
    ? index.plugins.map((p) => (p.id === entry.id ? entry : p))
    : [...index.plugins, entry];
  return { ...index, plugins };
}

async function main(argv) {
  const o = { _: [] };
  const valued = ['url', 'index', 'api', 'summary', 'icon', 'homepage', 'kinds', 'readme', 'screenshots'];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--jl') o.jl = true;
    else if (a === '--featured') o.featured = true;
    else if (a.startsWith('--') && valued.includes(a.slice(2))) o[a.slice(2)] = argv[++i];
    else if (a.startsWith('--')) throw new Error(`Unknown option ${a}`);
    else o._.push(a);
  }
  if (o._.length !== 1 || !o.url) throw new Error('usage: add-plugin.mjs <file.lumenplugin> --url <release asset url> [options]');
  if (o.screenshots) o.screenshots = o.screenshots.split(',').map((u) => u.trim());
  if (o.kinds) o.kinds = o.kinds.split(',').map((k) => k.trim());
  const indexFile = path.resolve(o.index ?? path.join(here, 'index.json'));
  const index = JSON.parse(fs.readFileSync(indexFile, 'utf8'));
  const entry = await entryFromPackage(fs.readFileSync(o._[0]), o, await loadPackTools());
  const next = upsert(index, entry);
  if (o.featured) next.featured = entry.id;
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
