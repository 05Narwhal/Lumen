-- Incremental import memory per source volume or device (architecture spec §6.3).
CREATE TABLE import_seen (
  source_key  TEXT NOT NULL,      -- volume: "vol:<label>:<total_space>", iphone: "ios:<udid>"
  rel_path    TEXT NOT NULL,
  size        INTEGER NOT NULL,
  mtime       INTEGER,
  seen_at     INTEGER NOT NULL,
  PRIMARY KEY (source_key, rel_path)
);

ALTER TABLE devices ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_albums_parent ON albums(parent_id);
