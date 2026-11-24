//! Lumen plugin SDK (plugin API 1.3). Thin, safe helpers over the host's WIT interfaces.
//!
//! Features: `importer` (world `importer-plugin`), `tools` (world `tools-plugin`), both
//! (world `full-plugin`); API 1.1's `places` (world `places-plugin`: tools and the Place review
//! export, with `models`, `library` and `places`), used alone; API 1.2's `ui` (world `ui-plugin`:
//! tools plus the settings page's `ui.action` / `ui.value`), used alone; and API 1.3's `proposals`
//! (world `proposals-plugin`: tools, with `library` and `proposals`), used alone. The world decides which interfaces the component can import, so a
//! plugin built with only `importer` can never ask for `photos` or `drafts`.
//!
//! The host calls only exist when compiling for `wasm32`; natively the crate keeps just the
//! pure helpers so they can be unit-tested with `cargo test`.

use serde::de::DeserializeOwned;

pub mod chosen;
pub mod drafts;
pub mod host;
pub mod library;
pub mod models;
pub mod photos;
pub mod places;
pub mod proposals;
pub mod storage;

#[cfg(all(feature = "places", any(feature = "importer", feature = "tools")))]
compile_error!("lumen-plugin: the `places` feature is used alone (it already exports tools)");
#[cfg(all(feature = "ui", any(feature = "importer", feature = "tools", feature = "places")))]
compile_error!("lumen-plugin: the `ui` feature is used alone (it already exports tools)");
#[cfg(all(feature = "proposals", any(feature = "importer", feature = "tools", feature = "places", feature = "ui")))]
compile_error!("lumen-plugin: the `proposals` feature is used alone (it already exports tools)");

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
    #[cfg(all(feature = "places", not(any(feature = "importer", feature = "tools"))))]
    wit_bindgen::generate!({
        world: "places-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
    #[cfg(all(feature = "ui", not(any(feature = "importer", feature = "tools", feature = "places"))))]
    wit_bindgen::generate!({
        world: "ui-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
    #[cfg(all(feature = "proposals", not(any(feature = "importer", feature = "tools", feature = "places", feature = "ui"))))]
    wit_bindgen::generate!({
        world: "proposals-plugin",
        path: "../../../src-tauri/crates/photos-plugins/wit",
        pub_export_macro: true,
        export_macro_name: "__export",
        default_bindings_module: "lumen_plugin::bindings",
    });
}

#[cfg(all(target_arch = "wasm32", any(feature = "importer", feature = "tools", feature = "places", feature = "ui", feature = "proposals")))]
pub use bindings::lumen::plugin::types::{Entry, Extras, Item, PhotoMeta, Tool};
#[cfg(all(target_arch = "wasm32", feature = "proposals"))]
pub use bindings::lumen::plugin::types::{Asset, AssetDetails, AssetFilter, AssetPage, FaceName, Gps, GpsSource, LatLon, MediaKind, Proposal, Proposed};
#[cfg(all(target_arch = "wasm32", feature = "places"))]
pub use bindings::lumen::plugin::types::{
    Asset, AssetDetails, AssetFilter, AssetPage, Candidate, Gps, GpsSource, MediaKind, MoveTo, Neighbour, PlaceFlag, PlaceHit,
    PlaceState, ReviewEvent, SplitAt, Suggestion, Tensor, VectorSource,
};

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
#[cfg(all(target_arch = "wasm32", any(feature = "tools", feature = "places", feature = "ui", feature = "proposals")))]
pub trait Tools {
    fn tools() -> Vec<Tool>;
    fn call(name: String, args_json: String) -> Result<String, String>;
}

/// What a places plugin implements for the Place review screen (API 1.1, spec §8.6): each of
/// the person's actions, answered with only the suggestions it changed.
#[cfg(all(target_arch = "wasm32", feature = "places"))]
pub trait Review {
    fn on_review(event: ReviewEvent) -> Vec<Suggestion>;
}

/// What a plugin implements for its settings page (API 1.2, feature `ui`): a `button` control
/// runs `action`; a `stat` or `list` control reads `value`. A stat's value is plain text; a list's
/// is a JSON array of strings or of `{"text": ..., "detail": ...}`. Both run under the plugin's
/// usual 30 s / 512 MB limits, and only while the plugin is on.
#[cfg(all(target_arch = "wasm32", feature = "ui"))]
pub trait Ui {
    fn action(name: String) -> Result<String, String>;
    fn value(key: String) -> Result<String, String>;
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

/// Exports a [`Tools`] and a [`Review`] (feature `places`).
#[cfg(all(target_arch = "wasm32", feature = "places"))]
#[macro_export]
macro_rules! export_places {
    ($t:ty, $r:ty) => {
        struct __LumenPlaces;
        impl $crate::bindings::exports::lumen::plugin::tools::Guest for __LumenPlaces {
            fn tools() -> Vec<$crate::Tool> { <$t as $crate::Tools>::tools() }
            fn call(name: String, args_json: String) -> Result<String, String> { <$t as $crate::Tools>::call(name, args_json) }
        }
        impl $crate::bindings::exports::lumen::plugin::review::Guest for __LumenPlaces {
            fn on_review(event: $crate::ReviewEvent) -> Vec<$crate::Suggestion> { <$r as $crate::Review>::on_review(event) }
        }
        $crate::bindings::__export!(__LumenPlaces with_types_in $crate::bindings);
    };
}

/// Exports a [`Tools`] (feature `proposals`); its tools call [`proposals::submit_all`].
#[cfg(all(target_arch = "wasm32", feature = "proposals"))]
#[macro_export]
macro_rules! export_proposals {
    ($t:ty) => {
        struct __LumenProposals;
        impl $crate::bindings::exports::lumen::plugin::tools::Guest for __LumenProposals {
            fn tools() -> Vec<$crate::Tool> { <$t as $crate::Tools>::tools() }
            fn call(name: String, args_json: String) -> Result<String, String> { <$t as $crate::Tools>::call(name, args_json) }
        }
        $crate::bindings::__export!(__LumenProposals with_types_in $crate::bindings);
    };
}

/// Exports a [`Tools`] and a [`Ui`] (feature `ui`).
#[cfg(all(target_arch = "wasm32", feature = "ui"))]
#[macro_export]
macro_rules! export_ui {
    ($t:ty, $u:ty) => {
        struct __LumenUi;
        impl $crate::bindings::exports::lumen::plugin::tools::Guest for __LumenUi {
            fn tools() -> Vec<$crate::Tool> { <$t as $crate::Tools>::tools() }
            fn call(name: String, args_json: String) -> Result<String, String> { <$t as $crate::Tools>::call(name, args_json) }
        }
        impl $crate::bindings::exports::lumen::plugin::ui::Guest for __LumenUi {
            fn action(name: String) -> Result<String, String> { <$u as $crate::Ui>::action(name) }
            fn value(key: String) -> Result<String, String> { <$u as $crate::Ui>::value(key) }
        }
        $crate::bindings::__export!(__LumenUi with_types_in $crate::bindings);
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
