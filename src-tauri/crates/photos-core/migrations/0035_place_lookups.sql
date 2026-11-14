-- Exact place names (plan 2026-11-08-exact-place-names): names from OpenStreetMap Nominatim,
-- looked up by the exact position (rounded to 5 decimals, ~1 m) so a border is never crossed.
-- `place_source`: 0 = the built-in atlas or nothing yet, 1 = named online. Derived data only.
ALTER TABLE assets ADD COLUMN place_source INTEGER NOT NULL DEFAULT 0;
CREATE TABLE place_lookups (
  key          TEXT PRIMARY KEY,   -- "46.00060,14.02640"
  place_name   TEXT NOT NULL,      -- '' = nothing to say
  locality     TEXT NOT NULL,
  country_code TEXT,
  address_json TEXT NOT NULL,      -- Nominatim's `address`, so new name rules need no network
  fetched_at   INTEGER NOT NULL
);
CREATE INDEX idx_assets_place_source ON assets(place_source) WHERE gps_lat IS NOT NULL;
