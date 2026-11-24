// node --test plugins/store/validate.test.mjs
import assert from 'node:assert/strict';
import test from 'node:test';

import { validateIndex } from './validate.mjs';

const entry = (over = {}, latest = {}) => ({
  id: 'jl.wayfinder',
  name: 'Wayfinder',
  author: 'JL Studios',
  jl: true,
  summary: 'Find where photos without GPS were taken.',
  kinds: ['tools'],
  permissions: ['library.read'],
  icon: 'https://example.com/icon.png',
  homepage: 'https://example.com',
  latest: {
    version: '1.0.0',
    api: '1.1.0',
    url: 'https://github.com/05Narwhal/lumen-plugins/releases/download/x/wayfinder-1.0.0.lumenplugin',
    blake3: 'a'.repeat(64),
    size: 123,
    released: '2026-11-21',
    ...latest,
  },
  ...over,
});
const index = (...plugins) => ({ version: 1, plugins });

test('a valid index passes', () => {
  assert.deepEqual(validateIndex(index(entry())), []);
});

test('a missing latest.blake3 fails', () => {
  const e = entry();
  delete e.latest.blake3;
  const errors = validateIndex(index(e));
  assert.equal(errors.length, 1);
  assert.match(errors[0], /blake3/);
});

test('a blake3 that is not 64 hex characters fails', () => {
  assert.match(validateIndex(index(entry({}, { blake3: 'abc' })))[0], /blake3/);
});

test('a duplicate id fails', () => {
  const errors = validateIndex(index(entry(), entry({ name: 'Other' })));
  assert.equal(errors.length, 1);
  assert.match(errors[0], /duplicate/i);
});

test('an api other than 1.0.0 or 1.1.0 fails', () => {
  assert.match(validateIndex(index(entry({}, { api: '2.0.0' })))[0], /api/);
});

test('a non-https package url fails', () => {
  assert.match(validateIndex(index(entry({}, { url: 'http://example.com/a.lumenplugin' })))[0], /https/);
});

test('a non-https icon fails', () => {
  assert.match(validateIndex(index(entry({ icon: 'http://example.com/i.png' })))[0], /https/);
});

test('an unknown index version fails', () => {
  assert.match(validateIndex({ version: 2, plugins: [] })[0], /version/);
});

test('an unknown field fails', () => {
  assert.match(validateIndex(index(entry({ extra: 1 })))[0], /extra/);
});

// ---- Round 25 P1: listing fields and `featured`

const listed = (over = {}) =>
  entry({
    readme: 'https://example.com/README.md',
    screenshots: ['https://example.com/1.png', 'https://example.com/2.png'],
    authorUrl: 'https://example.com/jl',
    repository: 'https://github.com/05Narwhal/wayfinder',
    ...over,
  });

test('an entry with a README, screenshots, author link and repository passes', () => {
  assert.deepEqual(validateIndex(index(listed())), []);
});

test('every listing link must be https', () => {
  for (const over of [
    { readme: 'http://example.com/R.md' },
    { screenshots: ['http://example.com/1.png'] },
    { authorUrl: 'http://example.com' },
    { repository: 'javascript:alert(1)' },
  ]) {
    assert.match(validateIndex(index(listed(over)))[0], /https|match/, JSON.stringify(over));
  }
});

test('more than six screenshots fails', () => {
  const seven = Array.from({ length: 7 }, (_, n) => `https://example.com/${n}.png`);
  assert.match(validateIndex(index(listed({ screenshots: seven })))[0], /at most 6/);
});

test('featured must name a listed plugin', () => {
  assert.deepEqual(validateIndex({ ...index(entry()), featured: 'jl.wayfinder' }), []);
  assert.match(validateIndex({ ...index(entry()), featured: 'jl.nothing' })[0], /featured/);
});
