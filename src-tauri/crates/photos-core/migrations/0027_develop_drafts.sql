-- Round 6: Develop's edits stay in Develop until a photo is exported. The library, the
-- viewer and every thumbnail show `edits` (the viewer's crop, turn and trim) only.
CREATE TABLE develop_drafts (
  asset_id    INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  recipe_json TEXT NOT NULL,
  -- From the same never-reused counter as `edits.version` (`edit_meta.max_version`), so an
  -- export's renders of a draft never share a cache key with a library edit's.
  version     INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);

-- Recipes made in Develop move whole (copied first, then removed): the library goes back to
-- the original, and Develop opens them as they were. Nothing is lost.
INSERT OR IGNORE INTO develop_drafts (asset_id, recipe_json, version, updated_at)
  SELECT asset_id, recipe_json, version, updated_at FROM edits
  WHERE json_extract(recipe_json, '$.develop') IS NOT NULL;
DELETE FROM edits
  WHERE json_extract(recipe_json, '$.develop') IS NOT NULL
    AND asset_id IN (SELECT asset_id FROM develop_drafts);
