-- Round 22 lane B: what Repair replaced, and how much of a recovered photo is real.
-- `recovered_pct` is set when a photo's original is the readable part of a damaged file (§4).
-- `replaced_originals` remembers the bytes a swap set aside, so a source that still holds them
-- is not offered them again as a new photo, and the swap can be undone.
ALTER TABLE assets ADD COLUMN recovered_pct INTEGER;
CREATE TABLE replaced_originals (
  content_hash       TEXT PRIMARY KEY,           -- the bytes that were replaced
  asset_id           INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  kept_rel           TEXT,                       -- under <library>/, NULL when there was no file
  old_relative_path  TEXT NOT NULL,
  old_extension      TEXT NOT NULL,
  old_byte_size      INTEGER NOT NULL,
  replaced_at        INTEGER NOT NULL
);
CREATE INDEX replaced_originals_asset ON replaced_originals(asset_id);
