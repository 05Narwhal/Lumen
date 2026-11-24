// node --test scripts/plugins/pack.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import * as pack from './pack.mjs';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const fixtures = path.join(root, 'src-tauri', 'crates', 'photos-plugins', 'tests', 'fixtures');
const testKey = path.join(fixtures, 'test-key', 'test.key');
const testPub = path.join(fixtures, 'test-key', 'test.pub');

const tmp = (name) => fs.mkdtempSync(path.join(os.tmpdir(), `lumen-pack-${name}-`));
const put = (dir, rel, data) => {
  const full = path.join(dir, ...rel.split('/'));
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, data);
};
const PLUGIN = `id = "com.test.warm"
name = "Warm"
version = "1.0.0"
author = "Test"
description = "A test presets plugin"
license = "MIT"
lumen = ">=26.2.0"
api = 1

[presets]
dir = "presets"
`;
function pluginFolder() {
  const dir = tmp('plugin');
  put(dir, 'plugin.toml', PLUGIN);
  put(dir, 'presets/warm.toml', 'name = "Warm"\n');
  return dir;
}

test('digest_matches_golden', () => {
  const g = JSON.parse(fs.readFileSync(path.join(fixtures, 'digest-golden.json'), 'utf8'));
  const files = g.files.map((f) => [f.path, Buffer.from(f.contentBase64, 'base64')]);
  assert.equal(pack.digestOfFiles(files).toString('hex'), g.digestHex);
  // order of input doesn't matter, SIGNATURE is ignored
  assert.equal(pack.digestOfFiles([...files].reverse().concat([['SIGNATURE', Buffer.from('x')]])).toString('hex'), g.digestHex);
});

test('deterministic_zip', () => {
  const dir = pluginFolder();
  const a = path.join(dir, '..', `${path.basename(dir)}-a.lumenplugin`);
  const b = path.join(dir, '..', `${path.basename(dir)}-b.lumenplugin`);
  pack.pack(dir, { keyFile: testKey, outFile: a });
  pack.pack(dir, { keyFile: testKey, outFile: b });
  assert.ok(fs.readFileSync(a).equals(fs.readFileSync(b)));
  // re-packing after the signature exists in the folder changes nothing either
  put(dir, 'SIGNATURE', 'old');
  pack.pack(dir, { keyFile: testKey, outFile: b });
  assert.ok(fs.readFileSync(a).equals(fs.readFileSync(b)));
});

test('ignores_build_dirs', () => {
  const dir = pluginFolder();
  put(dir, 'target/release/junk.bin', 'x');
  put(dir, 'sub/target/junk.bin', 'x');
  put(dir, '.git/config', 'x');
  put(dir, 'sub/.git/HEAD', 'x');
  const out = path.join(tmp('out'), 'p.lumenplugin');
  pack.pack(dir, { keyFile: testKey, outFile: out });
  const names = [...pack.unzipFiles(fs.readFileSync(out)).keys()].sort();
  assert.deepEqual(names, ['SIGNATURE', 'plugin.toml', 'presets/warm.toml']);
});

test('refuses_like_lumen', () => {
  assert.throws(() => pack.checkPaths(['a\\b']), /can't contain/);
  assert.throws(() => pack.checkPaths(['Themes/a.toml', 'themes/a.toml']), /differ only in case/);
  assert.throws(() => pack.checkPaths(['A/x', 'a/y']), /A\/x?.*differ only in case|a and A differ only in case|differ only in case/);
  assert.throws(() => pack.checkPaths(['a/../b']), /'\.\.'/);
  assert.throws(() => pack.checkPaths(['con.txt']), /reserved Windows name/);
  assert.throws(() => pack.checkPaths(['inner.zip']), /other archives/);
  assert.throws(() => pack.checkPaths(['a', 'a/b']), /both a file and a folder/);
  assert.throws(() => pack.checkPaths(Array.from({ length: 2001 }, (_, i) => `f${i}`)), /at most 2000 files/);
  // a LicenseRef licence needs a LICENSE file
  const dir = tmp('licref');
  put(dir, 'plugin.toml', PLUGIN.replace('"MIT"', '"LicenseRef-Mine"'));
  put(dir, 'presets/warm.toml', 'x');
  assert.throws(() => pack.pack(dir, { outFile: path.join(tmp('o'), 'p.lumenplugin') }), /needs a LICENSE/);
  put(dir, 'LICENSE', 'mine');
  pack.pack(dir, { outFile: path.join(tmp('o'), 'p.lumenplugin') });
  // no plugin.toml
  const empty = tmp('empty');
  put(empty, 'a.txt', 'x');
  assert.throws(() => pack.pack(empty), /no plugin.toml/);
});

test('manifest_read', () => {
  const m = pack.readManifest(`license = "MIT"\npermissions = [\n "storage",\n "photos.read"\n]\n[[assets]]\npath = "a"\nlicense = "OFL-1.1"\n`);
  assert.deepEqual(m, { license: 'MIT', permissions: ['storage', 'photos.read'], assetLicenses: ['OFL-1.1'] });
});

test('wasm_imports_checked', { skip: pack.wasmImports === undefined || !hasWasmTools() }, () => {
  const wasm = path.join(fixtures, '..', 'tests', 'wasm', 'importer_ok.wasm');
  const dir = tmp('wasm');
  put(dir, 'plugin.toml', PLUGIN.replace(/\[presets\][^]*$/,'[importer]\nlabel = "X"\npick = "folder"\n'));
  put(dir, 'plugin.wasm', fs.readFileSync(wasm));
  assert.throws(() => pack.pack(dir, { outFile: path.join(tmp('o'), 'p.lumenplugin') }), /needs the permission/);
});

function hasWasmTools() {
  try {
    return pack.wasmImports(path.join(fixtures, '..', 'tests', 'wasm', 'importer_ok.wasm')) !== null;
  } catch {
    return false;
  }
}

test('sign_and_verify', () => {
  const dir = pluginFolder();
  const out = path.join(tmp('sv'), 'p.lumenplugin');
  pack.pack(dir, { keyFile: testKey, outFile: out });
  assert.equal(pack.verifyCommand(out, testPub).ok, true);

  // one byte of a file changed -> fails
  const files = pack.unzipFiles(fs.readFileSync(out));
  const toml = Buffer.from(files.get('plugin.toml'));
  toml[0] ^= 1;
  files.set('plugin.toml', toml);
  const bad = path.join(path.dirname(out), 'bad.lumenplugin');
  fs.writeFileSync(bad, pack.zipFiles([...files]));
  assert.equal(pack.verifyCommand(bad, testPub).ok, false);

  // a different key fails
  const keys = tmp('keys');
  const k = pack.keygen('other', keys);
  assert.equal(pack.verifyCommand(out, k.pubFile).ok, false);
  // and an unencrypted key made by keygen signs and verifies
  const out2 = path.join(path.dirname(out), 'own.lumenplugin');
  pack.pack(dir, { keyFile: k.keyFile, outFile: out2 });
  assert.equal(pack.verifyCommand(out2, k.pubFile).ok, true);
  assert.equal(pack.verifyCommand(out2, testPub).ok, false);
});

test('keygen_refuses_repo_path', () => {
  assert.throws(() => pack.keygen('nope', path.join(root, 'scripts', 'plugins', 'keys-here')), /git repository/);
  assert.ok(!fs.existsSync(path.join(root, 'scripts', 'plugins', 'keys-here')));
  const k = pack.keygen('fine', tmp('kg'));
  assert.ok(fs.readFileSync(k.pubFile, 'utf8').startsWith('untrusted comment: minisign public key: '));
  assert.throws(() => pack.keygen('fine', path.dirname(k.keyFile)), /already exists/);
});

test('digest_command_writes_32_bytes', () => {
  const dir = pluginFolder();
  const out = path.join(tmp('dg'), 'digest.bin');
  const d = pack.digestCommand(dir, out);
  assert.equal(fs.readFileSync(out).length, 32);
  assert.ok(fs.readFileSync(out).equals(d));
  // the same digest from the packed file
  const p = path.join(path.dirname(out), 'p.lumenplugin');
  pack.pack(dir, { keyFile: testKey, outFile: p });
  const out2 = path.join(path.dirname(out), 'digest2.bin');
  pack.digestCommand(p, out2);
  assert.ok(fs.readFileSync(out2).equals(d));
});

// ---- Round 25 P1: the listing checks Lumen repeats (icon, README, screenshots, links)

const png = (w, h, extra = 0) => {
  const b = Buffer.alloc(24 + extra);
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]).copy(b);
  b.writeUInt32BE(w, 16);
  b.writeUInt32BE(h, 20);
  return b;
};
const withListing = (extra, shots = []) =>
  `${PLUGIN.replace('[presets]', `${extra}\n[listing]\nscreenshots = [${shots.map((s) => `"${s}"`).join(', ')}]\n\n[presets]`)}`;
const content = (toml, extras = {}) =>
  pack.checkContent([['plugin.toml', Buffer.from(toml)], ['presets/warm.toml', Buffer.from('x')], ...Object.entries(extras)]);

test('listing_read', () => {
  const l = pack.readListing(
    `author_url = "https://example.com/me"\nrepository = "https://github.com/a/b"\n[listing]\nscreenshots = [\n "screenshots/a.png",\n "screenshots/b.jpg"\n]\n[presets]\ndir = "presets"\n`,
  );
  assert.deepEqual(l, { authorUrl: 'https://example.com/me', repository: 'https://github.com/a/b', screenshots: ['screenshots/a.png', 'screenshots/b.jpg'] });
});

test('a good listing passes', () => {
  assert.deepEqual(
    content(withListing('author_url = "https://example.com"', ['screenshots/a.png']), {
      'icon.png': png(256, 256),
      'README.md': Buffer.from('# hi'),
      'screenshots/a.png': png(800, 600),
    }),
    [],
  );
});

test('icon must be a square PNG of 128 to 512 px under 256 KB', () => {
  for (const [file, why] of [
    [png(64, 64), /128/],
    [png(1024, 1024), /512/],
    [png(256, 200), /square/],
    [png(256, 256, 256 * 1024), /256 KB/],
    [Buffer.from('not a png at all, really not'), /PNG/],
  ]) {
    assert.throws(() => content(PLUGIN, { 'icon.png': file }), why);
  }
});

test('README is at most 64 KB of UTF-8', () => {
  assert.throws(() => content(PLUGIN, { 'README.md': Buffer.alloc(64 * 1024 + 1, 97) }), /64 KB/);
  assert.throws(() => content(PLUGIN, { 'README.md': Buffer.from([0xff, 0xfe, 0x41]) }), /UTF-8/);
});

test('screenshots are listed, shipped, images and at most six', () => {
  const shot = (n) => `screenshots/${n}.png`;
  assert.throws(() => content(withListing('', [shot('a')])), /screenshots\/a\.png.*isn't in the package/);
  assert.throws(() => content(withListing('', ['pics/a.png']), { 'pics/a.png': png(10, 10) }), /under screenshots\//);
  assert.throws(() => content(withListing('', [shot('a'), shot('a')]), { [shot('a')]: png(10, 10) }), /twice/);
  assert.throws(() => content(withListing('', [shot('a')]), { [shot('a')]: Buffer.from('GIF89a....') }), /PNG or JPEG/);
  assert.throws(() => content(withListing('', [shot('a')]), { [shot('a')]: png(10, 10, 2 * 1024 * 1024) }), /2 MB/);
  const seven = [...'abcdefg'];
  assert.throws(() => content(withListing('', seven.map(shot)), Object.fromEntries(seven.map((n) => [shot(n), png(10, 10)]))), /at most 6/);
});

test('author_url and repository must be https', () => {
  assert.throws(() => content(withListing('author_url = "http://example.com"')), /author_url.*https/);
  assert.throws(() => content(withListing('repository = "git://example.com/a"')), /repository.*https/);
});

// ---- Round 26: themes are .lumentheme files

test('a plugin with [[themes]] is refused', () => {
  const dir = pluginFolder();
  put(dir, 'plugin.toml', `${PLUGIN}\n[[themes]]\nfile = "themes/nord.toml"\n`);
  assert.throws(() => pack.pack(dir, { outFile: path.join(tmp('o'), 'p.lumenplugin') }), /\.lumentheme/);
});

test('pack-theme is deterministic and refuses what a theme may not carry', async () => {
  const { packTheme } = await import('./pack-theme.mjs');
  const dir = tmp('theme');
  put(dir, 'theme.toml', 'id = "studio.test.nord"\nname = "Nord"\n');
  put(dir, 'fonts/a.woff2', 'wOF2');
  const [a, b] = [path.join(tmp('o'), 'a.lumentheme'), path.join(tmp('o'), 'b.lumentheme')];
  packTheme(dir, { outFile: a });
  packTheme(dir, { outFile: b });
  assert.ok(fs.readFileSync(a).equals(fs.readFileSync(b)));
  put(dir, 'plugin.wasm', 'x');
  assert.throws(() => packTheme(dir, { outFile: a }), /plugin\.wasm: a theme carries only/);
});
