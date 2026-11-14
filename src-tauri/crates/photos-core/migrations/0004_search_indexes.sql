-- Indexes the query builder leans on (architecture spec §11).
CREATE INDEX IF NOT EXISTS idx_assets_rating ON assets(rating) WHERE state = 0;
CREATE INDEX IF NOT EXISTS idx_assets_device_time ON assets(device_id, captured_at DESC);
CREATE INDEX IF NOT EXISTS idx_assets_gps ON assets(gps_lat) WHERE gps_lat IS NOT NULL;

-- 0001 created assets_fts as contentless (`content=''`). A contentless row can only be
-- deleted by handing SQLite the exact text that was indexed, which we no longer have once
-- a device is renamed — the stale name stayed searchable forever. The index is derived
-- data, so it is cheaper to let FTS5 keep its own copy of the text (a few MB at 250k rows)
-- than to shadow every indexed field. Recreate it and fill it from the catalogue.
DROP TABLE IF EXISTS assets_fts;
CREATE VIRTUAL TABLE assets_fts USING fts5(
  filename, device_name, place_name, keywords,
  tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO assets_fts(rowid, filename, device_name, place_name, keywords)
  SELECT a.id, a.filename, d.display_name, NULL, NULL
  FROM assets a LEFT JOIN devices d ON d.id = a.device_id;
