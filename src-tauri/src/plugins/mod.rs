//! Plugins in the app (Round 23): the loaded store in app state and each lane's glue. The
//! packages, checks and store themselves live in the `photos-plugins` crate.

pub mod data;
#[cfg(test)]
mod examples_check;
pub mod importer;
pub mod mcp_tools;
pub mod state;
pub mod themes;

pub use state::Plugins;
