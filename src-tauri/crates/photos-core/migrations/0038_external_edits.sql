-- Round 22 lane C (spec §8): copies made for an external editor, beside their source in
-- originals/, and the app each copy was made for (read by the grid without a join).
ALTER TABLE assets ADD COLUMN edited_in TEXT;          -- app id of the editor a copy was made for
CREATE TABLE external_edits (
  id               INTEGER PRIMARY KEY,
  source_asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  asset_id         INTEGER UNIQUE REFERENCES assets(id) ON DELETE CASCADE,  -- NULL while the copy still has the source's bytes
  app_id           TEXT NOT NULL,
  relative_path    TEXT NOT NULL UNIQUE,               -- under originals/
  made_key         TEXT NOT NULL,
  made_hash        TEXT NOT NULL,
  seen_size        INTEGER NOT NULL,
  seen_mtime       INTEGER NOT NULL,
  created_at       INTEGER NOT NULL
);
CREATE INDEX external_edits_source ON external_edits(source_asset_id, app_id);
