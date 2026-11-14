#!/usr/bin/env node
// Builds the two code examples (plugins/examples/importer-folder-notes and tools-warmth) for
// wasm32-wasip2 and copies each .wasm next to its plugin.toml as plugin.wasm (git-ignored).
//
//   node scripts/plugins/build-examples.mjs
//
// Needs `rustup target add wasm32-wasip2`. Then pack with scripts/plugins/pack.mjs.

import { execFileSync } from 'node:child_process';
import { copyFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const examples = join(root, 'plugins', 'examples');
const names = ['importer-folder-notes', 'tools-warmth'];

// One build per plugin: in one workspace build cargo would unify the SDK's features (importer + tools),
// which selects the full world and hides export_importer! / export_tools!.
for (const name of names) {
  execFileSync('cargo', ['build', '--release', '--target', 'wasm32-wasip2', '-p', name], { cwd: examples, stdio: 'inherit' });
  const built = join(examples, 'target', 'wasm32-wasip2', 'release', `${name.replaceAll('-', '_')}.wasm`);
  copyFileSync(built, join(examples, name, 'plugin.wasm'));
  console.log(`${name}/plugin.wasm`);
}
