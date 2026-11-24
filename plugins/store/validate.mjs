#!/usr/bin/env node
// Checks the plugin store's index.json (Round 24 C3).
//
//   node plugins/store/validate.mjs <index.json> [--download] [--base <old index.json>] [--jl-pub <file>]
//
// Without --download: the file against index.schema.json plus unique ids. With --download: every
// entry's package (only those new or changed against --base, when given) is fetched, its size and
// blake3 compared with the entry, its manifest id and version compared with the entry, and its
// SIGNATURE checked against the author key written in it. An entry with `"jl": true` must be signed
// by the key in --jl-pub (default: jl.pub next to this file). Lumen verifies the signature again on
// install; this keeps a bad entry out of the repo.
//
// The schema checker is a small subset interpreter (no dependencies) so the repo works on its own.
// --download needs pack.mjs (this folder, else scripts/plugins/ of the Lumen repo) and its
// dependencies `@noble/hashes` and `fflate`.

import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const schema = JSON.parse(fs.readFileSync(path.join(here, 'index.schema.json'), 'utf8'));

// ---------------------------------------------------------------- schema subset

const kindOf = (v) => (v === null ? 'null' : Array.isArray(v) ? 'array' : Number.isInteger(v) ? 'integer' : typeof v);

function check(node, value, at, errors) {
  if (node.$ref) node = node.$ref.split('/').slice(1).reduce((n, k) => n[k], schema);
  const bad = (why) => errors.push(`${at}: ${why}`);
  if ('const' in node && value !== node.const) return bad(`must be ${JSON.stringify(node.const)}`);
  if (node.enum && !node.enum.includes(value)) return bad(`must be one of ${node.enum.join(', ')}`);
  const kind = kindOf(value);
  if (node.type === 'object' && kind !== 'object') return bad('must be an object');
  if (node.type === 'array' && kind !== 'array') return bad('must be an array');
  if (node.type === 'string' && kind !== 'string') return bad('must be a string');
  if (node.type === 'boolean' && kind !== 'boolean') return bad('must be true or false');
  if (node.type === 'integer' && kind !== 'integer') return bad('must be a whole number');
  if (kind === 'string') {
    if (node.minLength && value.length < node.minLength) bad('must not be empty');
    if (node.pattern && !new RegExp(node.pattern).test(value)) bad(`does not match ${node.pattern}`);
  }
  if (kind === 'integer' && node.minimum !== undefined && value < node.minimum) bad(`must be at least ${node.minimum}`);
  if (kind === 'array' && node.maxItems !== undefined && value.length > node.maxItems) bad(`must have at most ${node.maxItems} items`);
  if (kind === 'array' && node.items) value.forEach((v, i) => check(node.items, v, `${at}[${i}]`, errors));
  if (kind === 'object' && node.type === 'object') {
    for (const k of node.required ?? []) if (!(k in value)) bad(`${k} is missing`);
    for (const [k, v] of Object.entries(value)) {
      const sub = node.properties?.[k];
      if (sub) check(sub, v, at ? `${at}.${k}` : k, errors);
      else if (node.additionalProperties === false) errors.push(`${at ? `${at}.` : ''}${k}: unknown field`);
    }
  }
}

/** Schema and uniqueness. Returns a list of messages, empty when the index is valid. */
export function validateIndex(index) {
  const errors = [];
  if (index?.version !== undefined && index.version !== 1) return ['version: must be 1'];
  check(schema, index, '', errors);
  const seen = new Set();
  for (const p of Array.isArray(index?.plugins) ? index.plugins : []) {
    if (typeof p?.id !== 'string') continue;
    if (seen.has(p.id)) errors.push(`${p.id}: duplicate id`);
    seen.add(p.id);
  }
  if (typeof index?.featured === 'string' && !seen.has(index.featured)) errors.push(`featured: ${index.featured} is not in the index`);
  const themes = new Set();
  for (const t of Array.isArray(index?.themes) ? index.themes : []) {
    if (typeof t?.id !== 'string') continue;
    if (themes.has(t.id) || seen.has(t.id)) errors.push(`${t.id}: duplicate id`);
    themes.add(t.id);
  }
  if (typeof index?.featuredTheme === 'string' && !themes.has(index.featuredTheme)) {
    errors.push(`featuredTheme: ${index.featuredTheme} is not in the index`);
  }
  return errors;
}

// ---------------------------------------------------------------- manifest

/** `id`, `name`, `version`, `author`, `description`, `api`, `permissions` and the section names, without a TOML library. */
export function manifestInfo(text) {
  const info = { permissions: [], tables: [] };
  let table = '';
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    const header = /^\[\[?\s*([\w.-]+)\s*\]\]?/.exec(line);
    if (header) {
      table = header[1];
      info.tables.push(table);
      continue;
    }
    if (table !== '') continue;
    const str = /^(id|name|version|author|description|license|author_url|repository)\s*=\s*"([^"]*)"/.exec(line);
    if (str) info[str[1]] = str[2];
    const num = /^api\s*=\s*([\d.]+|"[\d.]+")/.exec(line);
    if (num) info.api = num[1].replaceAll('"', '');
    if (/^permissions\s*=/.test(line)) {
      let body = line.slice(line.indexOf('=') + 1);
      while (!body.includes(']') && i + 1 < lines.length) body += ` ${lines[++i]}`;
      info.permissions = [...body.matchAll(/"([^"]*)"/g)].map((x) => x[1]);
    }
  }
  return info;
}

// ---------------------------------------------------------------- packages

export async function loadPackTools() {
  for (const candidate of [path.join(here, 'pack.mjs'), path.join(here, '..', '..', 'scripts', 'plugins', 'pack.mjs')]) {
    if (fs.existsSync(candidate)) return import(pathToFileURL(candidate).href);
  }
  throw new Error('pack.mjs not found next to validate.mjs or under scripts/plugins/');
}

/** Everything wrong with one downloaded package; empty when it matches its entry. */
export async function checkPackage(entry, bytes, { pack, jlPub }) {
  const errors = [];
  const at = entry.id;
  const { blake3 } = await import('@noble/hashes/blake3.js');
  const sum = Buffer.from(blake3(bytes)).toString('hex');
  if (bytes.length !== entry.latest.size) errors.push(`${at}: size is ${bytes.length}, the entry says ${entry.latest.size}`);
  if (sum !== entry.latest.blake3) errors.push(`${at}: blake3 is ${sum}, the entry says ${entry.latest.blake3}`);
  let files;
  try {
    files = pack.unzipFiles(bytes);
  } catch (e) {
    return [...errors, `${at}: not a plugin package (${e.message})`];
  }
  const manifest = files.get('plugin.toml');
  if (!manifest) return [...errors, `${at}: the package has no plugin.toml`];
  const info = manifestInfo(manifest.toString('utf8'));
  if (info.id !== entry.id) errors.push(`${at}: the package's id is ${info.id}`);
  if (info.version !== entry.latest.version) errors.push(`${at}: the package's version is ${info.version}, the entry says ${entry.latest.version}`);
  const sig = files.get('SIGNATURE');
  if (!sig) return [...errors, `${at}: the package has no SIGNATURE`];
  const text = sig.toString('utf8');
  const line = /^untrusted comment: lumen plugin key (\S+)/.exec(text)?.[1];
  let author;
  try {
    author = pack.readPublicKey(line ?? '');
  } catch {
    return [...errors, `${at}: the SIGNATURE does not carry an author key`];
  }
  if (!pack.verifyDigest(text, pack.digestOfFiles(files), author)) errors.push(`${at}: the signature does not match the files`);
  if (entry.jl) {
    if (!jlPub) errors.push(`${at}: "jl": true needs the JL Studios public key (jl.pub or --jl-pub)`);
    else if (author.line !== jlPub.line) errors.push(`${at}: "jl": true but the package is not signed by the JL Studios key`);
  }
  return errors;
}

// ---------------------------------------------------------------- command line

async function run(argv) {
  const opts = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--download') opts.download = true;
    else if (a === '--base' || a === '--jl-pub') opts[a.slice(2)] = argv[++i];
    else if (a.startsWith('--')) throw new Error(`Unknown option ${a}`);
    else opts._.push(a);
  }
  if (opts._.length !== 1) throw new Error('usage: validate.mjs <index.json> [--download] [--base <old index.json>] [--jl-pub <file>]');
  const index = JSON.parse(fs.readFileSync(opts._[0], 'utf8'));
  const errors = validateIndex(index);
  if (!errors.length && opts.download) {
    const pack = await loadPackTools();
    const jlFile = opts['jl-pub'] ?? path.join(path.dirname(path.resolve(opts._[0])), 'jl.pub');
    const jlPub = fs.existsSync(jlFile) ? pack.readPublicKey(fs.readFileSync(jlFile, 'utf8')) : null;
    const old = new Map((opts.base ? JSON.parse(fs.readFileSync(opts.base, 'utf8')).plugins : []).map((p) => [p.id, p.latest.blake3]));
    for (const entry of index.plugins) {
      if (old.get(entry.id) === entry.latest.blake3) continue;
      const res = await fetch(entry.latest.url);
      if (!res.ok) {
        errors.push(`${entry.id}: ${entry.latest.url} answered ${res.status}`);
        continue;
      }
      errors.push(...(await checkPackage(entry, Buffer.from(await res.arrayBuffer()), { pack, jlPub })));
    }
  }
  return errors;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  run(process.argv.slice(2)).then(
    (errors) => {
      errors.forEach((e) => console.error(`error: ${e}`));
      if (!errors.length) console.log('ok');
      process.exitCode = errors.length ? 1 : 0;
    },
    (e) => {
      console.error(`error: ${e.message}`);
      process.exitCode = 1;
    },
  );
}
