# lumen-plugin

Safe Rust helpers for writing Lumen plugins (plugin API 1, WIT `lumen:plugin@1.0.0`). MIT.

```toml
[dependencies]
lumen-plugin = { path = "../sdk/lumen-plugin", features = ["importer"] }   # and/or "tools"
```

Build with `cargo build --target wasm32-wasip2 --release`. The WIT is read from
`src-tauri/crates/photos-plugins/wit`; it is the single source of truth and is not copied here.

| Feature | World bound | Exports macro |
| --- | --- | --- |
| `importer` | `importer-plugin` | `export_importer!(T)` (T: `Importer`) |
| `tools` | `tools-plugin` | `export_tools!(T)` (T: `Tools`) |
| both | `full-plugin` | `export_full!(I, T)` (one component cannot export twice) |

API: `log`, `setting::<T>(key)` (settings are JSON text), `chosen::{list, stat, read, read_all,
progress}`, `storage::{read, write, remove, names}`, `photos::{scope, metadata, preview, recipe}`
(feature `tools`), `drafts::patch` (feature `tools`). Host calls exist only on `wasm32`; natively
`cargo test` runs the pure helpers (`chosen::read_all_with`, setting decoding).

## Verified: unused interfaces are not imported (`unused_interfaces_not_imported`)

A scratch guest (outside the repo) with both features, calling `log`, `setting`, `chosen::list`,
`chosen::progress`, `chosen::read` and `photos::scope` but **never `storage` or `drafts`**, built
with `cargo build --target wasm32-wasip2 --release` against wit-bindgen 0.63. Its component's
imports (read with a small `wasmparser` program) hold `lumen:plugin/{host,types,chosen,photos}`
and the WASI interfaces, and neither `storage` nor `drafts`. So the linker drops what the code
never calls, a plugin without the `storage` permission passes when it does not use it, and no
per-permission world split was needed (worlds are split only by plugin kind). `types` is always
imported (it only carries type definitions, no functions).

Tooling gap: `wasm-tools` was not installed on the machine that ran this, so the check used
`wasmparser` instead of `wasm-tools component wit`. Re-run with `wasm-tools component wit
plugin.wasm` when it is available.
