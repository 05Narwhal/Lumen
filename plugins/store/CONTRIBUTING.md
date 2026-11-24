# Adding a plugin to the store

Anyone can propose a plugin; Julian reviews and merges every pull request.

1. Build your plugin with the `lumen-plugin` SDK (see `docs/PLUGINS.md` in the Lumen repo). Pick the
   narrowest permissions it needs; the install sheet shows them to every person.
2. Pack and sign it with your own key:
   `node pack.mjs keygen <name>` once (keep the `.key` file secret and out of every repository),
   then `node pack.mjs pack <folder> --key <file> --out <name>-<version>.lumenplugin`.
   Updates must be signed by the same key; Lumen refuses an update from a different key.
3. Attach the `.lumenplugin` to a release (in your own repository, or ask in the pull request for
   one here). The URL must be `https`.
4. Fork this repository and run
   `node add-plugin.mjs <file> --url <asset url> --summary "<one sentence>"`
   (add `--icon`, `--homepage`, `--api 1.1.0` as needed). Do not set `--jl`; it is for plugins
   signed by the JL Studios key.
5. Run `node validate.mjs index.json --download` (needs `npm i @noble/hashes fflate`).
6. Open a pull request with the index change only. CI repeats step 5. Sign off your commit
   (`git commit -s`).

## What gets a plugin declined

Permissions it does not need, a package that does not match its entry, a licence that is not
stated, code or models that are not yours to distribute, or anything that sends a person's photos
or data off their computer without saying so in its description.

To update a plugin, run `add-plugin.mjs` again with the new package: the entry with the same `id`
is replaced.
