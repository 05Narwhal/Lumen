-- Round 18 C1: a file Lumen has tried every reader on and still cannot open is taken off every
-- grid and out of every count, and listed under Settings -> Library -> "Files Lumen can't
-- open". Derived data only: the flag is set from the unreadable marks under derivatives/ by a
-- backfill and by ingest, cleared by a later successful decode or "Try again", and never
-- touches originals/.
--
-- It rides on `concealed` (migration 0019), which every list already filters on, so grids,
-- counts, summaries, search and memories agree without touching a query. The five triggers
-- that keep `concealed` true are recreated to include the new flag.

ALTER TABLE assets ADD COLUMN unreadable INTEGER NOT NULL DEFAULT 0;
CREATE INDEX assets_unreadable ON assets(id) WHERE unreadable = 1;

DROP TRIGGER concealed_on_album_add;
DROP TRIGGER concealed_on_album_remove;
DROP TRIGGER concealed_on_album_hides;
DROP TRIGGER concealed_on_album_delete;
DROP TRIGGER concealed_on_vault;

CREATE TRIGGER concealed_on_album_add AFTER INSERT ON album_items BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id = NEW.asset_id;
END;

CREATE TRIGGER concealed_on_album_remove AFTER DELETE ON album_items BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id = OLD.asset_id;
END;

CREATE TRIGGER concealed_on_album_hides AFTER UPDATE OF hides_items ON albums BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1))
  WHERE id IN (SELECT asset_id FROM album_items WHERE album_id = NEW.id);
END;

CREATE TRIGGER concealed_on_album_delete BEFORE DELETE ON albums BEGIN
  UPDATE assets SET concealed = (in_vault = 1 OR unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = assets.id AND al.hides_items = 1 AND al.id != OLD.id))
  WHERE id IN (SELECT asset_id FROM album_items WHERE album_id = OLD.id);
END;

CREATE TRIGGER concealed_on_vault AFTER UPDATE OF in_vault ON assets BEGIN
  UPDATE assets SET concealed = (NEW.in_vault = 1 OR NEW.unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = NEW.id AND al.hides_items = 1))
  WHERE id = NEW.id;
END;

CREATE TRIGGER concealed_on_unreadable AFTER UPDATE OF unreadable ON assets BEGIN
  UPDATE assets SET concealed = (NEW.in_vault = 1 OR NEW.unreadable = 1 OR EXISTS (
    SELECT 1 FROM album_items ai JOIN albums al ON al.id = ai.album_id
    WHERE ai.asset_id = NEW.id AND al.hides_items = 1))
  WHERE id = NEW.id;
END;

-- Assets are inserted unreadable = 0, so no row needs `concealed` recomputed here.
