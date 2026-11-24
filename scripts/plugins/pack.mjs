#!/usr/bin/env node
// Packs, signs and checks Lumen plugins (Round 23 D1).
//
//   node scripts/plugins/pack.mjs pack <folder> [--key <file>] [--out <file>]
//        (without --out: plugins/store/dist/<folder>.lumenplugin, git-ignored, the dev store's folder)
//   node scripts/plugins/pack.mjs keygen <name> [--dir <folder>]
//   node scripts/plugins/pack.mjs digest <folder|file> --out <file>     (32 raw bytes)
//   node scripts/plugins/pack.mjs verify <file> --pub <file>
//
// The digest is the one photos-plugins' `signature::digest` computes (blake3 over
// `path \0 hex(blake3(file)) \n`, paths in UTF-8 byte order, SIGNATURE excluded). The signature
// is minisign's prehashed `ED` form: Ed25519 over BLAKE2b-512(digest). Lumen stays the authority
// on what it accepts; this tool refuses what it can see cheaply.

import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { blake3 } from '@noble/hashes/blake3.js';
import { unzipSync, zipSync } from 'fflate';

export const MAX_BYTES = 50 * 1024 * 1024;
export const MAX_FILES = 2000;
export const AUTHOR_KEY_COMMENT = 'lumen plugin key';
const MAX_SIGNATURE = 64 * 1024;
const ARCHIVE_EXTENSIONS = ['zip', 'lumenplugin', '7z', 'rar', 'tar', 'gz', 'tgz', 'bz2', 'xz', 'zst', 'jar'];
const WINDOWS_RESERVED = [
  'con', 'prn', 'aux', 'nul', 'com1', 'com2', 'com3', 'com4', 'com5', 'com6', 'com7', 'com8', 'com9',
  'lpt1', 'lpt2', 'lpt3', 'lpt4', 'lpt5', 'lpt6', 'lpt7', 'lpt8', 'lpt9',
];
const SKIP_DIRS = new Set(['target', '.git']);
/** The wasm imports a permission covers (`host` is free). */
const INTERFACE_PERMISSION = {
  chosen: 'import.read-chosen',
  photos: 'photos.read',
  drafts: 'develop.write-drafts',
  storage: 'storage',
  // API 1.1 (Round 24)
  models: 'models',
  library: 'library.read',
  places: 'places.write',
};

export class PackError extends Error {}
const fail = (msg) => {
  throw new PackError(msg);
};

const hex = (buf) => Buffer.from(buf).toString('hex');
const byteOrder = (a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b));

// ---------------------------------------------------------------- digest

/** `files`: Map or array of [path, Buffer]. Returns the 32-byte digest. */
export function digestOfFiles(files) {
  const entries = [...(files instanceof Map ? files.entries() : files)]
    .filter(([p]) => p !== 'SIGNATURE')
    .sort(([a], [b]) => byteOrder(a, b));
  const parts = [];
  for (const [p, data] of entries) {
    parts.push(Buffer.from(p), Buffer.from([0]), Buffer.from(hex(blake3(data))), Buffer.from('\n'));
  }
  return Buffer.from(blake3(Buffer.concat(parts)));
}

// ---------------------------------------------------------------- Lumen's path rules

function checkPath(rel) {
  const bad = (why) => fail(`${JSON.stringify(rel)}: ${why}`);
  if (rel === '') bad('an empty path');
  if (rel.startsWith('/')) bad('paths must be relative');
  if (rel.includes('\\') || rel.includes(':')) bad("paths can't contain '\\' or ':'");
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f-\u009f]/.test(rel)) bad("paths can't contain control characters");
  for (const seg of rel.split('/')) {
    if (seg === '' || seg === '.' || seg === '..') bad("paths can't contain '..', '.' or empty parts");
    if (seg.endsWith('.') || seg.endsWith(' ')) bad("names can't end in '.' or a space");
    const stem = seg.split('.')[0].toLowerCase();
    if (WINDOWS_RESERVED.includes(stem)) bad('a reserved Windows name');
  }
}

/** Checks a list of relative file paths the way Lumen's package reader does. */
export function checkPaths(paths) {
  const lower = new Map();
  const dirs = new Map();
  let count = 0;
  for (const rel of paths) {
    checkPath(rel);
    const dot = rel.lastIndexOf('.');
    if (dot >= 0 && ARCHIVE_EXTENSIONS.includes(rel.slice(dot + 1).toLowerCase())) {
      fail(`${rel}: packages can't contain other archives`);
    }
    const low = rel.toLowerCase();
    if (lower.has(low)) fail(`${rel} and ${lower.get(low)} differ only in case`);
    if (dirs.has(low)) fail(`${rel} is both a file and a folder`);
    let at = 0;
    for (let i = rel.indexOf('/', at); i >= 0; i = rel.indexOf('/', at)) {
      const dir = rel.slice(0, i);
      const dlow = dir.toLowerCase();
      if (lower.has(dlow)) fail(`${dir} is both a file and a folder`);
      const seen = dirs.get(dlow);
      if (seen !== undefined && seen !== dir) fail(`${dir} and ${seen} differ only in case`);
      if (seen === undefined) dirs.set(dlow, dir);
      at = i + 1;
    }
    lower.set(low, rel);
    if (rel !== 'SIGNATURE' && ++count > MAX_FILES) fail(`A package can hold at most ${MAX_FILES} files`);
  }
}

// ---------------------------------------------------------------- the folder

/** Every file of a plugin folder as `[rel, Buffer]` in byte order; `target/`, `.git/` and a root SIGNATURE are left out. */
export function collectFolder(dir) {
  const out = [];
  const walk = (abs, prefix) => {
    for (const name of fs.readdirSync(abs)) {
      const rel = `${prefix}${name}`;
      const full = path.join(abs, name);
      const st = fs.lstatSync(full);
      if (st.isSymbolicLink()) fail(`${rel}: packages can't contain links`);
      if (st.isDirectory()) {
        if (SKIP_DIRS.has(name)) continue;
        walk(full, `${rel}/`);
      } else if (st.isFile()) {
        if (rel === 'SIGNATURE') continue;
        out.push(rel);
      } else fail(`${rel}: not a regular file`);
    }
  };
  walk(dir, '');
  out.sort(byteOrder);
  checkPaths(out);
  let total = 0;
  return out.map((rel) => {
    const data = fs.readFileSync(path.join(dir, ...rel.split('/')));
    total += data.length;
    if (total > MAX_BYTES) fail('A package can be at most 50 MB unpacked');
    return [rel, data];
  });
}

// ---------------------------------------------------------------- plugin.toml, just enough

/** Reads `license`, `permissions` and the assets' licences without a TOML library. */
export function readManifest(text) {
  const m = { license: null, permissions: [], assetLicenses: [] };
  let table = '';
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/^\s+/, '');
    const header = /^\[\[?\s*([\w.-]+)\s*\]\]?/.exec(line);
    if (header) {
      table = header[1];
      continue;
    }
    const str = /^license\s*=\s*"([^"]*)"/.exec(line);
    if (str && table === '') m.license = str[1];
    else if (str && table === 'assets') m.assetLicenses.push(str[1]);
    if (table === '' && /^permissions\s*=/.test(line)) {
      let body = line.slice(line.indexOf('=') + 1);
      while (!body.includes(']') && i + 1 < lines.length) body += ` ${lines[++i]}`;
      m.permissions = [...body.matchAll(/"([^"]*)"/g)].map((x) => x[1]);
    }
  }
  return m;
}

/** `author_url`, `repository` and `[listing] screenshots` of a manifest, without a TOML library. */
export function readListing(text) {
  const l = { authorUrl: null, repository: null, screenshots: [] };
  let table = '';
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].replace(/^\s+/, '');
    const header = /^\[\[?\s*([\w.-]+)\s*\]\]?/.exec(line);
    if (header) {
      table = header[1];
      continue;
    }
    const str = /^(author_url|repository)\s*=\s*"([^"]*)"/.exec(line);
    if (str && table === '') l[str[1] === 'author_url' ? 'authorUrl' : 'repository'] = str[2];
    if (table === 'listing' && /^screenshots\s*=/.test(line)) {
      let body = line.slice(line.indexOf('=') + 1);
      while (!body.includes(']') && i + 1 < lines.length) body += ` ${lines[++i]}`;
      l.screenshots = [...body.matchAll(/"([^"]*)"/g)].map((x) => x[1]);
    }
  }
  return l;
}

export const MAX_SCREENSHOTS = 6;
const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const isPng = (b) => b.length >= 24 && b.subarray(0, 8).equals(PNG_MAGIC);
const isJpeg = (b) => b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff;

/** The listing checks Lumen's package reader repeats: links, icon, README, screenshots. */
function checkListing(names, toml) {
  const l = readListing(toml);
  for (const [field, url] of [['author_url', l.authorUrl], ['repository', l.repository]]) {
    if (url !== null && !url.startsWith('https://')) fail(`${field} must be an https link`);
  }
  const icon = names.get('icon.png');
  if (icon) {
    if (!isPng(icon)) fail('icon.png must be a PNG');
    if (icon.length > 256 * 1024) fail('icon.png can be at most 256 KB');
    const [w, h] = [icon.readUInt32BE(16), icon.readUInt32BE(20)];
    if (w !== h) fail(`icon.png must be square (it is ${w}x${h})`);
    if (w < 128) fail(`icon.png must be at least 128 px (it is ${w})`);
    if (w > 512) fail(`icon.png can be at most 512 px (it is ${w})`);
  }
  const readme = names.get('README.md');
  if (readme) {
    if (readme.length > 64 * 1024) fail('README.md can be at most 64 KB');
    try {
      new TextDecoder('utf-8', { fatal: true }).decode(readme);
    } catch {
      fail('README.md must be UTF-8 text');
    }
  }
  if (l.screenshots.length > MAX_SCREENSHOTS) fail(`A plugin lists at most ${MAX_SCREENSHOTS} screenshots`);
  const seen = new Set();
  for (const shot of l.screenshots) {
    if (!shot.startsWith('screenshots/')) fail(`${shot}: screenshots must be under screenshots/`);
    if (seen.has(shot)) fail(`${shot} is listed twice`);
    seen.add(shot);
    const bytes = names.get(shot);
    if (!bytes) fail(`${shot} is listed but isn't in the package`);
    if (!isPng(bytes) && !isJpeg(bytes)) fail(`${shot} must be a PNG or JPEG`);
    if (bytes.length > 2 * 1024 * 1024) fail(`${shot} can be at most 2 MB`);
  }
}

function wasmToolsAvailable() {
  try {
    execFileSync('wasm-tools', ['--version'], { stdio: 'ignore' });
    return true;
  } catch {
    return false;
  }
}

/** The `lumen:plugin` interfaces a component imports, via `wasm-tools component wit`; `null` when the tool is absent. */
export function wasmImports(wasmPath) {
  if (!wasmToolsAvailable()) return null;
  const wit = execFileSync('wasm-tools', ['component', 'wit', wasmPath], { encoding: 'utf8' });
  return [...wit.matchAll(/^\s*import\s+lumen:plugin\/([\w-]+)/gm)].map((x) => x[1]);
}

/** Cheap checks Lumen will repeat; returns notes for what couldn't be checked. */
export function checkContent(files, { wasmFile } = {}) {
  const notes = [];
  const names = new Map(files);
  const toml = names.get('plugin.toml');
  if (!toml) fail('The package has no plugin.toml');
  if (/^\s*\[\[themes\]\]/m.test(toml.toString('utf8'))) {
    fail('[[themes]] left plugins in API 1.4: make the theme a .lumentheme (scripts/plugins/pack-theme.mjs, docs/THEMES.md)');
  }
  const m = readManifest(toml.toString('utf8'));
  checkListing(names, toml.toString('utf8'));
  const needsLicenceFile = [m.license, ...m.assetLicenses].some((l) => l?.startsWith('LicenseRef-'));
  if (needsLicenceFile && !names.has('LICENSE')) {
    fail('plugin.toml uses a LicenseRef-* licence, so the package needs a LICENSE file');
  }
  if (names.has('plugin.wasm')) {
    const imports = wasmImports(wasmFile);
    if (imports === null) {
      notes.push("wasm-tools isn't installed: plugin.wasm's imports weren't checked against permissions");
    } else {
      for (const iface of imports) {
        const need = INTERFACE_PERMISSION[iface];
        if (need && !m.permissions.includes(need)) {
          fail(`plugin.wasm imports ${iface}, which needs the permission "${need}" in plugin.toml`);
        }
      }
    }
  }
  return notes;
}

// ---------------------------------------------------------------- zip

export function zipFiles(files) {
  const when = new Date(1980, 0, 1, 0, 0, 0); // local fields, so the DOS time is the same everywhere
  const tree = {};
  for (const [p, data] of [...files].sort(([a], [b]) => byteOrder(a, b))) {
    tree[p] = [new Uint8Array(data), { level: 6, mtime: when, os: 3, attrs: 0o100644 << 16 }];
  }
  return Buffer.from(zipSync(tree));
}

export function unzipFiles(bytes) {
  const raw = unzipSync(new Uint8Array(bytes));
  return new Map(Object.entries(raw).filter(([p]) => !p.endsWith('/')).map(([p, d]) => [p, Buffer.from(d)]));
}

// ---------------------------------------------------------------- minisign

const SEED_PKCS8 = Buffer.from('302e020100300506032b657004220420', 'hex');
const SPKI = Buffer.from('302a300506032b6570032100', 'hex');
const b64 = (buf) => Buffer.from(buf).toString('base64');
const blake2b = (data, len = 64) => crypto.createHash('blake2b512').update(data).digest().subarray(0, len);

const edPrivate = (seed) => crypto.createPrivateKey({ key: Buffer.concat([SEED_PKCS8, seed]), format: 'der', type: 'pkcs8' });
const edPublic = (pk) => crypto.createPublicKey({ key: Buffer.concat([SPKI, pk]), format: 'der', type: 'spki' });
const dataLine = (text, n = 1) => {
  const lines = text.split(/\r?\n/).filter((l) => l.trim() !== '');
  return lines[n]?.trim();
};

/** scrypt parameters the way libsodium's pwhash derives them from ops/mem limits. */
function scryptParams(opslimit, memlimit) {
  const r = 8;
  const ops = Math.max(32768, opslimit);
  let log2 = 1;
  let p;
  if (ops < memlimit / 32) {
    p = 1;
    const maxn = Math.floor(ops / (r * 4));
    while (2 ** log2 <= maxn / 2 && log2 < 63) log2++;
  } else {
    const maxn = Math.floor(memlimit / (r * 128));
    while (2 ** log2 <= maxn / 2 && log2 < 63) log2++;
    const maxrp = Math.min(Math.floor(ops / 4 / 2 ** log2), 0x3fffffff);
    p = Math.floor(maxrp / r);
  }
  return { N: 2 ** log2, r, p, maxmem: 1024 * 1024 * 1024 };
}

/** Reads a minisign secret key file (unencrypted, or encrypted with an empty password). */
export function readSecretKey(text) {
  const bin = Buffer.from(dataLine(text) ?? '', 'base64');
  if (bin.length !== 158 || bin.subarray(0, 2).toString() !== 'Ed' || bin.subarray(4, 6).toString() !== 'B2') {
    fail("That isn't a minisign secret key");
  }
  const kdf = bin.subarray(2, 4).toString('latin1');
  const salt = bin.subarray(6, 38);
  const raw = Buffer.from(bin.subarray(54));
  const candidates = [raw];
  if (kdf === 'Sc') {
    const ops = Number(bin.readBigUInt64LE(38));
    const mem = Number(bin.readBigUInt64LE(46));
    const stream = crypto.scryptSync('', salt, 104, scryptParams(ops, mem));
    candidates.push(Buffer.from(raw.map((byte, i) => byte ^ stream[i])));
  } else if (kdf !== '  ') {
    fail('Unknown key derivation in the secret key');
  }
  // Some writers (the checked-in test key) keep the Sc fields but store the key in the clear
  // with a zero checksum; accept a candidate whose seed really yields its public key.
  for (const body of candidates) {
    const keyId = body.subarray(0, 8);
    const sk = body.subarray(8, 72);
    const sum = blake2b(Buffer.concat([bin.subarray(0, 2), keyId, sk]), 32);
    const sumOk = sum.equals(body.subarray(72, 104)) || body.subarray(72, 104).every((x) => x === 0);
    const seed = sk.subarray(0, 32);
    const pk = sk.subarray(32);
    if (sumOk && edPublicOf(seed).equals(pk)) return { keyId: Buffer.from(keyId), seed: Buffer.from(seed), pk: Buffer.from(pk) };
  }
  return fail("The secret key is damaged, or it has a password (pack can't use one)");
}

const edPublicOf = (seed) =>
  crypto.createPublicKey(edPrivate(seed)).export({ format: 'der', type: 'spki' }).subarray(-32);

/** Reads a `.pub` file or a raw `RW…` line. */
export function readPublicKey(text) {
  const line = text.includes('\n') ? dataLine(text) : text.trim();
  const bin = Buffer.from(line ?? '', 'base64');
  if (bin.length !== 42 || bin.subarray(0, 2).toString() !== 'Ed') fail("That isn't a minisign public key");
  return { keyId: bin.subarray(2, 10), pk: bin.subarray(10), line };
}

export const publicKeyLine = (key) => b64(Buffer.concat([Buffer.from('Ed'), key.keyId, key.pk]));

/** The SIGNATURE text for `digest` (32 bytes), prehashed, with the author's key in the untrusted comment. */
export function signDigest(key, digest, trustedComment = 'timestamp:0') {
  const priv = edPrivate(key.seed);
  const sig = crypto.sign(null, blake2b(digest), priv);
  const global = crypto.sign(null, Buffer.concat([sig, Buffer.from(trustedComment)]), priv);
  return [
    `untrusted comment: ${AUTHOR_KEY_COMMENT} ${publicKeyLine(key)}`,
    b64(Buffer.concat([Buffer.from('ED'), key.keyId, sig])),
    `trusted comment: ${trustedComment}`,
    b64(global),
    '',
  ].join('\n');
}

/** Checks SIGNATURE text over `digest` under `pub`. Returns true or false, never throws on a bad signature. */
export function verifyDigest(signatureText, digest, pub) {
  try {
    const lines = signatureText.trim().split(/\r?\n/);
    if (lines.length < 4 || !lines[0].startsWith('untrusted comment:') || !lines[2].startsWith('trusted comment:')) return false;
    const bin = Buffer.from(lines[1].trim(), 'base64');
    if (bin.length !== 74 || bin.subarray(0, 2).toString() !== 'ED') return false;
    if (!bin.subarray(2, 10).equals(pub.keyId)) return false;
    const key = edPublic(pub.pk);
    if (!crypto.verify(null, blake2b(digest), key, bin.subarray(10))) return false;
    const trusted = lines[2].slice('trusted comment: '.length);
    const global = Buffer.from(lines[3].trim(), 'base64');
    return crypto.verify(null, Buffer.concat([bin.subarray(10), Buffer.from(trusted)]), key, global);
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------- keygen

function insideGitRepo(dir) {
  for (let d = path.resolve(dir); ; d = path.dirname(d)) {
    if (fs.existsSync(path.join(d, '.git'))) return true;
    if (path.dirname(d) === d) return false;
  }
}

export function defaultKeyDir() {
  const home = os.homedir();
  const base =
    process.platform === 'win32'
      ? (process.env.APPDATA ?? path.join(home, 'AppData', 'Roaming'))
      : process.platform === 'darwin'
        ? path.join(home, 'Library', 'Application Support')
        : (process.env.XDG_CONFIG_HOME ?? path.join(home, '.config'));
  return path.join(base, 'JL-Studios', 'plugin-keys');
}

/** Writes `<name>.key` (minisign's unencrypted secret key) and `<name>.pub`. Returns both paths. */
export function keygen(name, dir = defaultKeyDir()) {
  if (!/^[\w.-]+$/.test(name)) fail('A key name is letters, digits, "-", "_" and "."');
  const dest = path.resolve(dir);
  if (insideGitRepo(dest)) fail(`Refusing to write a secret key inside a git repository: ${dest}`);
  const keyFile = path.join(dest, `${name}.key`);
  const pubFile = path.join(dest, `${name}.pub`);
  if (fs.existsSync(keyFile) || fs.existsSync(pubFile)) fail(`${name}.key or ${name}.pub already exists in ${dest}`);
  const { privateKey, publicKey } = crypto.generateKeyPairSync('ed25519');
  const seed = privateKey.export({ format: 'der', type: 'pkcs8' }).subarray(-32);
  const pk = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  const keyId = crypto.randomBytes(8);
  const sk = Buffer.concat([seed, pk]);
  const head = Buffer.concat([Buffer.from('Ed'), Buffer.alloc(2), Buffer.from('B2')]);
  const sum = blake2b(Buffer.concat([Buffer.from('Ed'), keyId, sk]), 32);
  const secret = Buffer.concat([head, Buffer.alloc(32 + 8 + 8), keyId, sk, sum]);
  fs.mkdirSync(dest, { recursive: true });
  fs.writeFileSync(keyFile, `untrusted comment: minisign secret key\n${b64(secret)}\n`, { mode: 0o600 });
  const id = hex(Buffer.from(keyId).reverse()).toUpperCase();
  fs.writeFileSync(pubFile, `untrusted comment: minisign public key: ${id}\n${publicKeyLine({ keyId, pk })}\n`);
  return { keyFile, pubFile };
}

// ---------------------------------------------------------------- commands

/** `plugins/store/dist/` in the Lumen repo (the dev store reads it); else the current folder. */
export function defaultOutDir() {
  const store = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'plugins', 'store');
  return fs.existsSync(store) ? path.join(store, 'dist') : process.cwd();
}

/** Packs `folder` into a signed (when `keyFile` is given) `.lumenplugin`. */
export function pack(folder, { keyFile, outFile } = {}) {
  const dir = path.resolve(folder);
  if (!fs.statSync(dir).isDirectory()) fail(`${folder} isn't a folder`);
  const files = collectFolder(dir);
  const wasm = path.join(dir, 'plugin.wasm');
  const notes = checkContent(files, { wasmFile: wasm });
  const entries = [...files];
  if (keyFile) {
    const sig = signDigest(readSecretKey(fs.readFileSync(keyFile, 'utf8')), digestOfFiles(files));
    if (Buffer.byteLength(sig) > MAX_SIGNATURE) fail('SIGNATURE is too large');
    entries.push(['SIGNATURE', Buffer.from(sig)]);
  } else notes.push('No --key: the package is unsigned');
  const out = path.resolve(outFile ?? path.join(defaultOutDir(), `${path.basename(dir)}.lumenplugin`));
  fs.mkdirSync(path.dirname(out), { recursive: true });
  fs.writeFileSync(out, zipFiles(entries));
  return { out, notes, files: files.length };
}

export function digestCommand(input, outFile) {
  const p = path.resolve(input);
  const files = fs.statSync(p).isDirectory() ? collectFolder(p) : unzipFiles(fs.readFileSync(p));
  const d = digestOfFiles(files);
  fs.writeFileSync(outFile, d);
  return d;
}

export function verifyCommand(file, pubFile) {
  const files = unzipFiles(fs.readFileSync(file));
  const sig = files.get('SIGNATURE');
  if (!sig) return { ok: false, why: 'The package has no SIGNATURE' };
  const pub = readPublicKey(fs.readFileSync(pubFile, 'utf8'));
  const ok = verifyDigest(sig.toString('utf8'), digestOfFiles(files), pub);
  return { ok, why: ok ? '' : "The signature doesn't match the files under that key" };
}

function flags(argv, names) {
  const out = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith('--')) {
      const k = argv[i].slice(2);
      if (!names.includes(k)) fail(`Unknown option ${argv[i]}`);
      out[k] = argv[++i];
      if (out[k] === undefined) fail(`${argv[i - 1]} needs a value`);
    } else out._.push(argv[i]);
  }
  return out;
}

export function main(argv) {
  const [cmd, ...rest] = argv;
  try {
    if (cmd === 'pack') {
      const a = flags(rest, ['key', 'out']);
      if (a._.length !== 1) fail('usage: pack <folder> [--key <file>] [--out <file>]');
      const r = pack(a._[0], { keyFile: a.key, outFile: a.out });
      r.notes.forEach((n) => console.log(`note: ${n}`));
      console.log(`${r.out} (${r.files} files)`);
    } else if (cmd === 'keygen') {
      const a = flags(rest, ['dir']);
      if (a._.length !== 1) fail('usage: keygen <name> [--dir <folder>]');
      const r = keygen(a._[0], a.dir);
      console.log(`${r.keyFile}\n${r.pubFile}\nKeep the .key file secret and outside every repository.`);
    } else if (cmd === 'digest') {
      const a = flags(rest, ['out']);
      if (a._.length !== 1 || !a.out) fail('usage: digest <folder|file> --out <file>');
      console.log(hex(digestCommand(a._[0], a.out)));
    } else if (cmd === 'verify') {
      const a = flags(rest, ['pub']);
      if (a._.length !== 1 || !a.pub) fail('usage: verify <file> --pub <file>');
      const r = verifyCommand(a._[0], a.pub);
      console.log(r.ok ? 'ok' : `FAILED: ${r.why}`);
      return r.ok ? 0 : 1;
    } else fail('usage: pack | keygen | digest | verify (see the header of pack.mjs)');
    return 0;
  } catch (e) {
    if (!(e instanceof PackError) && e?.code !== 'ENOENT') throw e;
    console.error(`error: ${e.message}`);
    return 1;
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1])) {
  process.exitCode = main(process.argv.slice(2));
}
