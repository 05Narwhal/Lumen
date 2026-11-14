-- The justified layout needs an edited photo's shape, and a crop or a quarter turn
-- changes it. Rust computes it from the recipe when the recipe is saved (SQL cannot run
-- the geometry), so a layout load stays one indexed LEFT JOIN.
ALTER TABLE edits ADD COLUMN aspect REAL;

-- The highest edit version ever issued. Versions end up in `immutable` image URLs, so one
-- must never be reused — not even after every edit has been reverted and its row deleted.
CREATE TABLE edit_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
