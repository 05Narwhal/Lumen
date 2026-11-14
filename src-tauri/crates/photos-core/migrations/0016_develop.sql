-- Phase 8 (docs/superpowers/specs/2026-09-22-lumen-develop-design.md §2).
-- Colour labels (0 none, 1 red, 2 yellow, 3 green, 4 blue, 5 purple) and flags
-- (-1 reject, 0 none, 1 pick), for culling a shoot before it is developed.
ALTER TABLE assets ADD COLUMN color_label INTEGER NOT NULL DEFAULT 0;
ALTER TABLE assets ADD COLUMN flag INTEGER NOT NULL DEFAULT 0;
CREATE INDEX assets_label_flag ON assets(color_label, flag) WHERE state = 0;

CREATE TABLE develop_presets (
    id             INTEGER PRIMARY KEY,
    name           TEXT NOT NULL,
    settings_json  TEXT NOT NULL,
    -- Which panels the preset sets (["basic", "curve", ...]); the others are left alone.
    panels_json    TEXT NOT NULL,
    builtin        INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);

CREATE TABLE edit_history (
    id             INTEGER PRIMARY KEY,
    asset_id       INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    recipe_json    TEXT NOT NULL,
    -- What the step did, in words ("Exposure +0.3").
    label          TEXT NOT NULL,
    -- A named snapshot; NULL for an ordinary step.
    snapshot_name  TEXT,
    created_at     INTEGER NOT NULL
);
CREATE INDEX edit_history_asset ON edit_history(asset_id, id);
