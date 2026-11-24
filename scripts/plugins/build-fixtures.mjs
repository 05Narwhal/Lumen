#!/usr/bin/env node
// Builds the wasm host's test fixtures (Round 23 A2) and checks them in.
//
//   node scripts/plugins/build-fixtures.mjs
//
// Needs the `wasm32-wasip2` target (`rustup target add wasm32-wasip2`). The fixtures in
// plugins/fixtures/ are built with plain wit-bindgen against
// src-tauri/crates/photos-plugins/wit/, each `.wasm` (already a component: wasip2 links with
// wasm-component-ld) is copied to src-tauri/crates/photos-plugins/tests/wasm/<name>.wasm, and
// WIT_HASH gets the blake3 of the WIT file with CRLF folded to LF.
//
// The hash comes from the tiny native helper `plugins/fixtures/wit_hash` (`cargo run`), so the
// script needs no npm package; `wit_hash_matches_fixtures` in photos-plugins computes the same.

import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const fixtures = join(root, 'plugins', 'fixtures');
const crate = join(root, 'src-tauri', 'crates', 'photos-plugins');
const out = join(crate, 'tests', 'wasm');
const names = ['importer_ok', 'tools_ok', 'socket', 'file', 'endless', 'memory_hog', 'undeclared', 'trap', 'review_ok', 'ui_ok', 'proposals_ok'];

const run = (args) =>
  execFileSync('cargo', args, { cwd: fixtures, stdio: ['ignore', 'pipe', 'inherit'] }).toString();

run(['build', '--release', '--target', 'wasm32-wasip2', ...names.flatMap((n) => ['-p', n])]);
mkdirSync(out, { recursive: true });
for (const name of names) {
  copyFileSync(join(fixtures, 'target', 'wasm32-wasip2', 'release', `${name}.wasm`), join(out, `${name}.wasm`));
}

const hash = run(['run', '--quiet', '--release', '-p', 'wit_hash', '--', join(crate, 'wit', 'lumen-plugin.wit')]).trim();
writeFileSync(join(out, 'WIT_HASH'), `${hash}\n`);
console.log(`${names.length} fixtures + WIT_HASH ${hash}`);
