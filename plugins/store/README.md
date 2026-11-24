# Lumen plugin store

The list of plugins that Lumen's **Plugins** page offers. Lumen is a local-first photo library
(<https://github.com/05Narwhal/lumen>); this repository is only the index and the rules for
getting into it. The packages themselves are `.lumenplugin` files attached to this repository's
releases.

## How Lumen reads it

- Lumen fetches `index.json` from
  `https://raw.githubusercontent.com/05Narwhal/lumen-plugins/main/index.json` when the page opens,
  at launch and every six hours, and keeps the last copy on disk so the page works offline.
- An entry says which package is the latest (`url`, `size`, `blake3`). Lumen checks the size and
  blake3 on download, then runs its normal install: the package signature, key pinning and the
  permission sheet are unchanged. A plugin that fails any of them is not installed.
- `"jl": true` shows *By JL Studios*, and only if the package is signed by the JL Studios key.
- `index.schema.json` is the format; `index.json` is version 1.

## Files

| File | What it is |
|---|---|
| `index.json` | The store index. |
| `index.schema.json` | Its JSON Schema. |
| `validate.mjs` | `node validate.mjs index.json [--download] [--base old.json]`: schema, unique ids and, with `--download`, each new package's size, blake3, manifest and signature. |
| `add-plugin.mjs` | `node add-plugin.mjs file.lumenplugin --url <release asset url>`: writes an entry from a package. |
| `pack.mjs` | A copy of Lumen's `scripts/plugins/pack.mjs` (packs, signs, verifies). Needs `@noble/hashes` and `fflate`. |
| `jl.pub` | The JL Studios plugin public key, used to check `"jl": true`. |
| `.github/workflows/validate.yml` | Runs the above on every pull request. |

## Setting this repository up (Julian)

1. Create the public repo `05Narwhal/lumen-plugins`.
2. Copy this folder's contents to its root, plus `scripts/plugins/pack.mjs` from the Lumen repo, plus
   `jl.pub` (the public half of the JL Studios plugin key; never the secret).
3. Push to `main`. Make a release for each package and attach the `.lumenplugin`.
4. Add each entry with `add-plugin.mjs`, run `node validate.mjs index.json --download`, commit, push.
