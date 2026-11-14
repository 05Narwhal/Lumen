-- Round 3 (docs/superpowers/plans/2026-09-23-round-3-fixes-import-hiding-films.md): hiding,
-- the Hidden folder, rotation hints, the import journal and memory films. One migration for
-- the round, so a library opened by a build part-way through never misses a later change.

-- An album that hides its photos from the rest of the library (school work, say).
ALTER TABLE albums ADD COLUMN hides_items INTEGER NOT NULL DEFAULT 0;

-- The Hidden folder, behind a password.
ALTER TABLE assets ADD COLUMN in_vault INTEGER NOT NULL DEFAULT 0;

-- 1 when the photo is in the Hidden folder or in any album that hides its items. Every
-- query that shows or curates photos filters on it, so none has to join albums. The
-- triggers below keep it true whatever changes membership, including cascading deletes.
ALTER TABLE assets ADD COLUMN concealed INTEGER NOT NULL DEFAULT 0;
CREATE INDEX assets_concealed ON assets(concealed) WHERE concealed = 1;

CREATE TRIGGER concealed_on_album_add AFTER INSERT ON album_items BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id = NEW.asset_id;
END;

CREATE TRIGGER concealed_on_album_remove AFTER DELETE ON album_items BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id = OLD.asset_id;
END;

CREATE TRIGGER concealed_on_album_hides AFTER UPDATE OF hides_items ON albums BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id IN (SELECT asset_id FROM album_items WHERE album_id = NEW.id);
END;

-- Before the album goes, so its photos are judged without it (the cascade to album_items
-- then finds nothing left to change).
CREATE TRIGGER concealed_on_album_delete BEFORE DELETE ON albums BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1 AND al.id != OLD.id))
  WHERE id IN (SELECT asset_id FROM album_items WHERE album_id = OLD.id);
END;

CREATE TRIGGER concealed_on_vault AFTER UPDATE OF in_vault ON assets BEGIN
  UPDATE assets SET concealed = (NEW.in_vault = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = NEW.id AND al.hides_items = 1))
  WHERE id = NEW.id;
END;

-- The Hidden folder's password: an Argon2id PHC string, never the password.
CREATE TABLE vault (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  phc         TEXT NOT NULL,
  created_at  INTEGER NOT NULL,
  reset_at    INTEGER
);

-- Rotation suggestions: NULL not looked at, 0 upright, 1–3 quarter turns clockwise needed.
ALTER TABLE assets ADD COLUMN rotation_hint INTEGER;
ALTER TABLE assets ADD COLUMN rotation_hint_conf REAL;

-- A file placed in originals/ whose row is not inserted yet. A crash between the two is
-- reconciled when the library next opens (photos-core::import_journal).
CREATE TABLE import_journal (
  id             INTEGER PRIMARY KEY,
  batch_id       INTEGER NOT NULL,
  content_hash   TEXT NOT NULL,
  relative_path  TEXT NOT NULL,
  placed_at      INTEGER NOT NULL
);

-- A memory played as a film: the show Lumen made for it, kept so a replay starts at once.
CREATE TABLE memory_films (
  memory_id  INTEGER PRIMARY KEY REFERENCES memories(id) ON DELETE CASCADE,
  -- What the film was made from (the curated ids), so a changed memory makes a new film.
  source_key TEXT NOT NULL,
  doc_json   TEXT NOT NULL,
  bpm        INTEGER,
  built_at   INTEGER NOT NULL
);
