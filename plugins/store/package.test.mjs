// node --test plugins/store/package.test.mjs
// Packs the Disk Atlas plugin with the test key, adds it to an index and checks it as --download would.
// Skipped outside the Lumen repo (needs scripts/plugins/pack.mjs and the test key).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { entryFromPackage, upsert } from './add-plugin.mjs';
import { checkPackage, loadPackTools, validateIndex } from './validate.mjs';

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const key = path.join(root, 'src-tauri', 'crates', 'photos-plugins', 'tests', 'fixtures', 'test-key');
const example = path.join(root, 'plugins', 'first-party', 'disk-atlas');
const ready = fs.existsSync(path.join(key, 'test.key')) && fs.existsSync(example);

const setup = async () => {
  const pack = await loadPackTools();
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'store-')), 'disk-atlas.lumenplugin');
  pack.pack(example, { keyFile: path.join(key, 'test.key'), outFile: out });
  const bytes = fs.readFileSync(out);
  const url = 'https://github.com/05Narwhal/lumen-plugins/releases/download/packages-1/disk-atlas.lumenplugin';
  const entry = await entryFromPackage(bytes, { url }, pack);
  const pub = pack.readPublicKey(fs.readFileSync(path.join(key, 'test.pub'), 'utf8'));
  return { pack, bytes, entry, pub };
};

test('add-plugin fills a valid entry from the package', { skip: !ready }, async () => {
  const { entry, bytes } = await setup();
  assert.equal(entry.latest.size, bytes.length);
  assert.deepEqual(entry.kinds, []);
  assert.deepEqual(validateIndex(upsert({ version: 1, plugins: [] }, entry)), []);
});

test('a package that matches its entry passes the download check', { skip: !ready }, async () => {
  const { pack, bytes, entry } = await setup();
  assert.deepEqual(await checkPackage(entry, bytes, { pack, jlPub: null }), []);
});

test('a changed package fails the blake3 check', { skip: !ready }, async () => {
  const { pack, bytes, entry } = await setup();
  const errors = await checkPackage({ ...entry, latest: { ...entry.latest, blake3: 'b'.repeat(64) } }, bytes, { pack, jlPub: null });
  assert.match(errors[0], /blake3/);
});

test('jl: true needs the JL key', { skip: !ready }, async () => {
  const { pack, bytes, entry, pub } = await setup();
  const claim = { ...entry, jl: true };
  assert.match((await checkPackage(claim, bytes, { pack, jlPub: null }))[0], /jl/);
  const other = { ...pub, line: 'RWQ' + 'x'.repeat(50) };
  assert.match((await checkPackage(claim, bytes, { pack, jlPub: other }))[0], /not signed by the JL/);
  assert.deepEqual(await checkPackage(claim, bytes, { pack, jlPub: pub }), []);
});

test('add-plugin writes the listing URLs it is given and the manifest links', { skip: !ready }, async () => {
  const { pack, bytes } = await setup();
  const entry = await entryFromPackage(
    bytes,
    {
      url: 'https://example.com/a.lumenplugin',
      readme: 'https://example.com/README.md',
      icon: 'https://example.com/icon.png',
      screenshots: ['https://example.com/1.png'],
    },
    pack,
  );
  assert.equal(entry.readme, 'https://example.com/README.md');
  assert.deepEqual(entry.screenshots, ['https://example.com/1.png']);
  assert.deepEqual(validateIndex({ ...upsert({ version: 1, plugins: [] }, entry), featured: entry.id }), []);
});
