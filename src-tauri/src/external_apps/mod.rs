//! External apps (Round 22 lane C, spec §7). The types, the built-in table, detection and the
//! custom source are pure and live in `photos_core::external_apps`; this module adds what needs the
//! machine: the real lookups (`system`), the cached list and the per-session icon numbers
//! (`state`). `icons` makes the 64 px PNGs; `launch` starts an app with files.

mod icons;
mod launch;
mod state;
mod system;

pub use photos_core::external_apps::*;
pub use icons::icon_png;
pub use launch::SystemLauncher;
pub use state::{apps_for, detected, forget_detected, icon_target, icon_url, to_external, use_plugins};
pub use system::{is_store, store_progid, SystemLookup};
