-- Round 24 lane W (§8.6, §8.7): GPS written into originals, and the writes that can be undone.
-- `content_hash` stays the import hash (derivatives, duplicates and import skip keep working);
-- `file_hash` is blake3 of the file's current bytes once a metadata write changed them, and is
-- what *Check the library* and Repair compare a library file with when it is set.
ALTER TABLE assets ADD COLUMN file_hash TEXT;
ALTER TABLE assets ADD COLUMN metadata_written_at INTEGER;
-- One *Write N locations* of a plugin's place review: one undo covers all of its items.
CREATE TABLE place_writes (
  id          INTEGER PRIMARY KEY,
  plugin_id   TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE TABLE place_write_items (
  write_id           INTEGER NOT NULL REFERENCES place_writes(id) ON DELETE CASCADE,
  asset_id           INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  lat                REAL NOT NULL,
  lon                REAL NOT NULL,
  wrote_file         INTEGER NOT NULL DEFAULT 0,  -- 1 when the GPS went into the original too
  prev_place_source  TEXT,                        -- what undo puts back
  PRIMARY KEY (write_id, asset_id)
);
CREATE INDEX place_write_items_asset ON place_write_items(asset_id);
