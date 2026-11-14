-- Distinct album covers (plan 2026-11-09): up to three prints that look different, picked
-- once and cached until the album's photos change. Derived columns only.
-- cover_picked: 1 when the person chose cover_asset_id by hand. Every cover set before this
-- migration counts as automatic (Julian, 2026-11-09), hence the default 0.
ALTER TABLE albums ADD COLUMN cover_picked INTEGER NOT NULL DEFAULT 0;
-- Changed by Shuffle cover; mixed with the album id so albums at 0 still differ.
ALTER TABLE albums ADD COLUMN cover_seed INTEGER NOT NULL DEFAULT 0;
-- The cached prints (JSON array of asset ids, front first) and what they were picked from.
ALTER TABLE albums ADD COLUMN preview_json TEXT;
ALTER TABLE albums ADD COLUMN preview_key TEXT;
