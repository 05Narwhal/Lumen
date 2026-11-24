//! Plugins in the app (Round 23): the loaded store in app state and each lane's glue. The
//! packages, checks and store themselves live in the `photos-plugins` crate.

pub mod data;
pub mod detect;
pub mod hosts;
#[cfg(test)]
mod first_party_check;
pub mod importer;
pub mod mcp_tools;
pub mod library;
pub mod models;
pub mod places;
pub mod providers;
pub mod state;
pub mod store; // Round 24 lane C
pub mod themes;
pub mod tool_selection; // Round 25 P4b-2b

pub use state::Plugins;
