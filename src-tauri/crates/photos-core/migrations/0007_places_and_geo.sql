-- Where a photo was taken, in words, filled by the offline atlas on the maintenance lane.
ALTER TABLE assets ADD COLUMN place_name TEXT;

-- Map panning grouped 83k geotagged rows out of 250k in ~150 ms, because the partial
-- index on gps_lat alone forced a table fetch per candidate row to read gps_lon, the
-- state and the id. This index carries everything the cluster query touches, so the
-- aggregation never leaves the index.
DROP INDEX IF EXISTS idx_assets_gps;
CREATE INDEX IF NOT EXISTS idx_assets_geo
  ON assets(gps_lat, gps_lon, state, media_type, live_photo_group, id)
  WHERE gps_lat IS NOT NULL;
