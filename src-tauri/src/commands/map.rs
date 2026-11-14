//! The map: clusters for a viewport, the library's extent, what is inside a box, and
//! offline place search.

use photos_core::geo::{self, Bounds, Cluster};
use photos_core::places::{self, Place};
use photos_core::scope::LayoutScope;
use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::error::AppResult;
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaceHit {
    pub label: String,
    pub lat: f64,
    pub lon: f64,
}

/// Markers for the current viewport. Called on `moveend`, so it may do real work.
#[tauri::command(async)]
pub fn map_clusters(
    state: State<'_, AppState>,
    bounds: Bounds,
    zoom: f64,
    scope: Option<LayoutScope>,
) -> AppResult<Vec<Cluster>> {
    let lib = state.library()?;
    let scope = scope.unwrap_or_default();
    Ok(lib.db.with_conn(|c| geo::map_clusters(c, bounds, zoom, &scope))?)
}

/// The extent of everything geotagged, for "fit to library". `None` when nothing has GPS.
#[tauri::command(async)]
pub fn map_bounds(state: State<'_, AppState>, scope: Option<LayoutScope>) -> AppResult<Option<Bounds>> {
    let lib = state.library()?;
    let scope = scope.unwrap_or_default();
    Ok(lib.db.with_conn(|c| geo::map_bounds(c, &scope))?)
}

/// Newest assets inside a box, for the strip under the map.
#[tauri::command(async)]
pub fn assets_in_bounds(
    state: State<'_, AppState>,
    bounds: Bounds,
    scope: Option<LayoutScope>,
    limit: Option<i64>,
) -> AppResult<Vec<i64>> {
    let lib = state.library()?;
    let scope = scope.unwrap_or_default();
    let limit = limit.unwrap_or(200).clamp(1, 500);
    Ok(lib.db.with_conn(|c| geo::assets_in_bounds(c, bounds, &scope, limit))?)
}

/// Place search over the bundled atlas. No network, so it works on a plane.
#[tauri::command(async)]
pub fn search_places(query: String, limit: Option<usize>) -> AppResult<Vec<PlaceHit>> {
    Ok(places::search(&query, limit.unwrap_or(8).clamp(1, 25))
        .into_iter()
        .map(|(label, lat, lon)| PlaceHit { label, lat, lon })
        .collect())
}

/// The place a single point is in, for the viewer's info panel.
#[tauri::command(async)]
pub fn place_for(lat: f64, lon: f64) -> AppResult<Option<Place>> {
    Ok(places::reverse_geocode(lat, lon))
}
