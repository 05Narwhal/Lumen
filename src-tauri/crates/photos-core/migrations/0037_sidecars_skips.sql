-- Round 19 C3: nothing is lost on import (spec 13).
--
-- assets.source_dir: the folder the file sat in on its source ('' at the root), so Live Photo
-- pairing joins a still and a movie only from the same folder. NULL on rows made before this
-- migration: they are never paired with a new row. Derived data.
ALTER TABLE assets ADD COLUMN source_dir TEXT;

-- Files that travel with a photo: `.xmp`, `.aae`, `.thm` beside a media file with the same
-- stem, copied next to its original (`originals/<relative_path>`) and recorded here.
-- kind: 'xmp' | 'aae' | 'thm'. relative_path is under originals/.
CREATE TABLE asset_sidecars (
  id            INTEGER PRIMARY KEY,
  asset_id      INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  kind          TEXT NOT NULL,
  relative_path TEXT NOT NULL,
  source_name   TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  content_hash  TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  UNIQUE (asset_id, kind)
);
CREATE INDEX asset_sidecars_asset ON asset_sidecars(asset_id);

-- Every file a source held that was not imported, and why. reason: 'unsupported' | 'duplicate'
-- | 'unreadable' | 'burst_frame' | 'not_downloaded' | 'failed_verify'. asset_id names the
-- library copy for 'duplicate'.
CREATE TABLE import_skips (
  id         INTEGER PRIMARY KEY,
  batch_id   INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
  path       TEXT NOT NULL,
  reason     TEXT NOT NULL,
  asset_id   INTEGER,
  created_at INTEGER NOT NULL
);
CREATE INDEX import_skips_batch ON import_skips(batch_id);
