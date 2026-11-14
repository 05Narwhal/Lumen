-- Phase 8f: AI masks (spec 2026-09-22-lumen-develop-design.md, Amendment (Phase 8f)).

-- Which photos have been looked at, by which model. A photo with nothing to mask still gets
-- its row, so asking again answers from here instead of running the model again.
CREATE TABLE IF NOT EXISTS ai_mask_scan (
  asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  model    TEXT NOT NULL
);

-- One row per raster. Stored here, not in the recipe (recipes go through invoke and into every
-- history step) and not in a cache (an export must not change when a cache or a model goes away).
-- A recipe names its raster by hash, so a detection that replaces this one leaves the old rows
-- with current = 0: recipes and drafts that use them still render as they did.
CREATE TABLE IF NOT EXISTS ai_masks (
  asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  hash     TEXT NOT NULL,               -- of kind, instance, size and weights
  kind     TEXT NOT NULL,               -- sky | subject | background | person
  instance INTEGER NOT NULL,            -- which person, from 0; 0 for the others
  model    TEXT NOT NULL,
  score    REAL NOT NULL,
  width    INTEGER NOT NULL,
  height   INTEGER NOT NULL,
  weights  BLOB NOT NULL,               -- width x height 8-bit weights, deflated
  current  INTEGER NOT NULL DEFAULT 1,  -- from the photo's latest detection
  PRIMARY KEY (asset_id, hash)
);
