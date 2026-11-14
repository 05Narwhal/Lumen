-- Structured filters (rating, kind, location) were costing ~200 ms over 250k items:
-- `idx_assets_timeline` narrows to state = 0 but every candidate row then had to be
-- fetched to test the other columns. This index carries all of them, so the scan stays
-- inside the index and never touches the table. It supersedes idx_assets_rating.
DROP INDEX IF EXISTS idx_assets_rating;
CREATE INDEX IF NOT EXISTS idx_assets_filter
  ON assets(state, media_type, rating, gps_lat, live_photo_group, captured_at DESC, id);
-- No ANALYZE here: a migration runs against an empty or stale table and would teach the
-- planner nothing. `photos_core::db::analyze` runs it on the maintenance lane instead.
