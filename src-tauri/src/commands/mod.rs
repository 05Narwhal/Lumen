//! Thin adapters only: deserialize, call a crate, serialize. No business logic.

pub mod ai;
pub mod albums;
pub mod apple;
pub mod backup;
pub mod app;
pub mod catalog;
pub mod denoise;
pub mod develop;
pub mod discord;
pub mod device_browser;
pub mod devices;
pub mod duplicates;
pub mod edits;
pub mod external_apps; // Round 22 lane C
pub mod facet;
#[cfg(debug_assertions)]
pub mod dev;
pub mod import;
pub mod media;
pub mod memories;
pub mod ml;
pub mod people;
pub mod pets;
pub mod plugin_import; // Round 23 A3
pub mod plugin_tools; // Round 23 A4
pub mod plugins; // Round 23 C1
pub mod raw_formats;
pub mod search;
pub mod music;
pub mod ocr;
pub mod library;
pub mod library_check;
pub mod repair; // Round 21 lane B
pub mod lightroom;
pub mod map;
pub mod mcp;
pub mod settings;
pub mod slideshows;
pub mod storage;
pub mod updates;
pub mod vault;
pub mod plugin_themes; // Round 23 lane B1
