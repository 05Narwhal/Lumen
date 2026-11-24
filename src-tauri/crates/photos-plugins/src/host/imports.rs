//! The install-time check of a `plugin.wasm` (spec §2): every import must be granted by the
//! manifest's permissions, nothing may reach the network, and the exports must match the code
//! kinds the manifest declares.

use wasmtime::component::Component;

use crate::package::{KindCheck, Report, Unpacked};
use crate::{PluginKind, PluginPermission};

/// The `lumen:plugin` interface an import names, if any, and the permission it needs.
pub(crate) enum Need {
    Free,
    Permission(PluginPermission),
    /// Linked when any of them is granted (API 1.3 `proposals`); each call checks its own.
    AnyOf(&'static [PluginPermission]),
    Network,
    Unknown,
}

/// What an import (`ns:pkg/iface@ver`) needs from the manifest.
pub(crate) fn need(import: &str) -> Need {
    let name = import.split('@').next().unwrap_or(import);
    let (pkg, iface) = name.split_once('/').unwrap_or((name, ""));
    match (pkg, iface) {
        ("lumen:plugin", "host" | "types") => Need::Free,
        ("lumen:plugin", "chosen") => Need::Permission(PluginPermission::ImportReadChosen),
        ("lumen:plugin", "photos") => Need::Permission(PluginPermission::PhotosRead),
        ("lumen:plugin", "drafts") => Need::Permission(PluginPermission::DevelopWriteDrafts),
        ("lumen:plugin", "storage") => Need::Permission(PluginPermission::Storage),
        ("lumen:plugin", "models") => Need::Permission(PluginPermission::Models),
        ("lumen:plugin", "library") => Need::Permission(PluginPermission::LibraryRead),
        ("lumen:plugin", "places") => Need::Permission(PluginPermission::PlacesWrite),
        ("lumen:plugin", "proposals") => Need::AnyOf(&PluginPermission::PROPOSING),
        ("wasi:sockets" | "wasi:http", _) => Need::Network,
        // Inert in Lumen's WASI: no preopens, no env, no args; clocks and random are allowed.
        ("wasi:cli" | "wasi:io" | "wasi:clocks" | "wasi:random" | "wasi:filesystem", _) => Need::Free,
        _ => Need::Unknown,
    }
}

fn shown(import: &str) -> &str {
    import.split('@').next().unwrap_or(import)
}

/// Names of the component's imports and exports, or why it isn't a component.
pub(crate) fn surface(bytes: &[u8]) -> Result<(Vec<String>, Vec<String>), String> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    let engine = wasmtime::Engine::new(&config).map_err(|e| e.to_string())?;
    let component = Component::new(&engine, bytes).map_err(|e| e.to_string())?;
    let ty = component.component_type();
    let imports = ty.imports(&engine).map(|(n, _)| n.to_string()).collect();
    let exports = ty.exports(&engine).map(|(n, _)| n.to_string()).collect();
    Ok((imports, exports))
}

fn exports_interface(exports: &[String], iface: &str) -> bool {
    exports
        .iter()
        .any(|e| shown(e) == iface && e.split('@').nth(1).is_some_and(|v| v.starts_with("1.")))
}

/// Checks a `plugin.wasm`'s imports against its permissions and its exports against the
/// manifest. Claims the two code kinds.
pub struct ImportsCheck;

impl KindCheck for ImportsCheck {
    fn kinds(&self) -> &'static [PluginKind] {
        &[PluginKind::Importer, PluginKind::Tools]
    }

    fn check(&self, pkg: &Unpacked, _dev: bool, report: &mut Report) {
        let m = &pkg.manifest;
        if !m.has_code() {
            return;
        }
        let bytes = match pkg.read("plugin.wasm") {
            Ok(b) => b,
            Err(e) => return report.problems.push(e.message),
        };
        let (imports, exports) = match surface(&bytes) {
            Ok(s) => s,
            Err(e) => {
                return report
                    .problems
                    .push(format!("plugin.wasm isn't a valid component: {e}"))
            }
        };

        let mut network = false;
        for import in &imports {
            match need(import) {
                Need::Free => {}
                Need::Permission(p) if m.permissions.contains(&p) => {}
                Need::Permission(_) => report.problems.push(format!(
                    "plugin.wasm uses {} without asking for it",
                    shown(import)
                )),
                Need::AnyOf(ps) if ps.iter().any(|p| m.permissions.contains(p)) => {}
                Need::AnyOf(_) => report.problems.push(format!(
                    "plugin.wasm uses {} without asking for a permission to propose anything",
                    shown(import)
                )),
                Need::Network => network = true,
                Need::Unknown => report.problems.push(format!(
                    "plugin.wasm uses {}, which Lumen doesn't offer to plugins",
                    shown(import)
                )),
            }
        }
        if network {
            report.problems.push("plugin.wasm uses the network".into());
        }

        if let Some(importer) = &m.importer {
            if exports_interface(&exports, "lumen:plugin/importer") {
                report.adds.push(format!("An importer: \"{}\"", importer.label));
            } else {
                report
                    .problems
                    .push("The manifest declares an importer but plugin.wasm doesn't export one".into());
            }
        }
        if !m.tools.is_empty() {
            if exports_interface(&exports, "lumen:plugin/tools") {
                let n = m.tools.len();
                report.adds.push(if n == 1 {
                    "1 tool for AI assistants".into()
                } else {
                    format!("{n} tools for AI assistants")
                });
            } else {
                report
                    .problems
                    .push("The manifest declares tools but plugin.wasm doesn't export them".into());
            }
        }
        let has_ui = exports_interface(&exports, "lumen:plugin/ui") && exports_interface(&exports, "lumen:plugin/tools");
        let live = m.sections.iter().flat_map(|s| &s.controls).any(|c| c.kind.needs_code());
        if live && !has_ui {
            report.problems.push(
                "The settings page has buttons, stats or lists but plugin.wasm doesn't export `ui` (with `tools`)".into(),
            );
        }
        if crate::manifest::needs_ui_export(m) && !has_ui {
            report.problems.push(
                "A contribution runs a plugin action but plugin.wasm doesn't export `ui` (with `tools`)".into(),
            );
        }
        if exports_interface(&exports, "lumen:plugin/review") && !m.permissions.contains(&PluginPermission::PlacesWrite) {
            report
                .problems
                .push("plugin.wasm answers the Place review but doesn't ask for places.write".into());
        }
    }
}
