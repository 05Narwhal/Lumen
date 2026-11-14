//! Lumen plugin SDK (plugin API 1). Thin, safe helpers over the host's WIT interfaces.
//!
//! Features: `importer` (world `importer-plugin`), `tools` (world `tools-plugin`), both
//! (world `full-plugin`). The world decides which interfaces the component can import, so a
//! plugin built with only `importer` can never ask for `photos` or `drafts`.
//!
//! The host calls only exist when compiling for `wasm32`; natively the crate keeps just the
//! pure helpers so they can be unit-tested with `cargo test`.

use serde::de::DeserializeOwned;

pub mod chosen;
pub mod drafts;
pub mod host;
pub mod photos;
pub mod storage;

#[cfg(target_arch = "wasm32")]
pub mod bindings {
    #[cfg(all(feature = "importer", feature = "tools"))]
    wit_bindgen::generate!({
        world: "full-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
    #[cfg(all(feature = "importer", not(feature = "tools")))]
    wit_bindgen::generate!({
        world: "importer-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
    #[cfg(all(feature = "tools", not(feature = "importer")))]
    wit_bindgen::generate!({
        world: "tools-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
}

#[cfg(all(target_arch = "wasm32", any(feature = "importer", feature = "tools")))]
pub use bindings::lumen::plugin::types::{Entry, Extras, Item, PhotoMeta, Tool};

/// Writes one line to the plugin's log (shown in Settings -> Plugins).
pub fn log(line: &str) {
    host::log(line);
}

/// A setting's value (the host keeps settings as JSON text), decoded as `T`.
/// `None` when the setting is unset or does not decode as `T`.
pub fn setting<T: DeserializeOwned>(key: &str) -> Option<T> {
    decode_setting(host::setting(key))
}

pub(crate) fn decode_setting<T: DeserializeOwned>(raw: Option<String>) -> Option<T> {
    serde_json::from_str(&raw?).ok()
}

/// What an importer plugin implements. `enumerate` lists what the chosen source holds, `read`
/// returns a range of one item's bytes, `extras` its albums and metadata, `sidecars` its
/// companion files.
#[cfg(all(target_arch = "wasm32", feature = "importer"))]
pub trait Importer {
    fn enumerate() -> Result<Vec<Item>, String>;
    fn read(key: String, offset: u64, len: u32) -> Result<Vec<u8>, String>;
    fn extras(key: String) -> Option<Extras>;
    fn sidecars(key: String) -> Vec<Item>;
}

/// What a tools plugin implements: the tool list and the dispatcher.
#[cfg(all(target_arch = "wasm32", feature = "tools"))]
pub trait Tools {
    fn tools() -> Vec<Tool>;
    fn call(name: String, args_json: String) -> Result<String, String>;
}

/// Exports an [`Importer`] (feature `importer` alone).
#[cfg(all(target_arch = "wasm32", feature = "importer", not(feature = "tools")))]
#[macro_export]
macro_rules! export_importer {
    ($t:ty) => {
        struct __LumenImporter;
        impl $crate::bindings::exports::lumen::plugin::importer::Guest for __LumenImporter {
            fn enumerate() -> Result<Vec<$crate::Item>, String> { <$t as $crate::Importer>::enumerate() }
            fn read(key: String, offset: u64, len: u32) -> Result<Vec<u8>, String> { <$t as $crate::Importer>::read(key, offset, len) }
            fn extras(key: String) -> Option<$crate::Extras> { <$t as $crate::Importer>::extras(key) }
            fn sidecars(key: String) -> Vec<$crate::Item> { <$t as $crate::Importer>::sidecars(key) }
        }
        $crate::bindings::__export!(__LumenImporter with_types_in $crate::bindings);
    };
}

/// Exports a [`Tools`] (feature `tools` alone).
#[cfg(all(target_arch = "wasm32", feature = "tools", not(feature = "importer")))]
#[macro_export]
macro_rules! export_tools {
    ($t:ty) => {
        struct __LumenTools;
        impl $crate::bindings::exports::lumen::plugin::tools::Guest for __LumenTools {
            fn tools() -> Vec<$crate::Tool> { <$t as $crate::Tools>::tools() }
            fn call(name: String, args_json: String) -> Result<String, String> { <$t as $crate::Tools>::call(name, args_json) }
        }
        $crate::bindings::__export!(__LumenTools with_types_in $crate::bindings);
    };
}

/// Exports both (features `importer` and `tools`): one component exports both interfaces, so
/// one macro takes both types.
#[cfg(all(target_arch = "wasm32", feature = "importer", feature = "tools"))]
#[macro_export]
macro_rules! export_full {
    ($i:ty, $t:ty) => {
        struct __LumenFull;
        impl $crate::bindings::exports::lumen::plugin::importer::Guest for __LumenFull {
            fn enumerate() -> Result<Vec<$crate::Item>, String> { <$i as $crate::Importer>::enumerate() }
            fn read(key: String, offset: u64, len: u32) -> Result<Vec<u8>, String> { <$i as $crate::Importer>::read(key, offset, len) }
            fn extras(key: String) -> Option<$crate::Extras> { <$i as $crate::Importer>::extras(key) }
            fn sidecars(key: String) -> Vec<$crate::Item> { <$i as $crate::Importer>::sidecars(key) }
        }
        impl $crate::bindings::exports::lumen::plugin::tools::Guest for __LumenFull {
            fn tools() -> Vec<$crate::Tool> { <$t as $crate::Tools>::tools() }
            fn call(name: String, args_json: String) -> Result<String, String> { <$t as $crate::Tools>::call(name, args_json) }
        }
        $crate::bindings::__export!(__LumenFull with_types_in $crate::bindings);
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_decodes_json() {
        assert_eq!(decode_setting::<u32>(Some("42".into())), Some(42));
        assert_eq!(decode_setting::<String>(Some("\"abc\"".into())), Some("abc".to_string()));
        assert_eq!(decode_setting::<Vec<bool>>(Some("[true,false]".into())), Some(vec![true, false]));
    }

    #[test]
    fn setting_unset_or_wrong_type_is_none() {
        assert_eq!(decode_setting::<u32>(None), None);
        assert_eq!(decode_setting::<u32>(Some("\"x\"".into())), None);
        assert_eq!(decode_setting::<u32>(Some("not json".into())), None);
    }
}
