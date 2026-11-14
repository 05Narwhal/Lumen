-- Phase 8c (docs/superpowers/plans/2026-09-23-phase-8c-develop.md, Task 6): the History
-- panel lists a photo's steps and snapshots together; capping steps at 200 deletes by
-- (asset_id, snapshot_name IS NULL, id). The built-in presets live in code, not here.
CREATE INDEX edit_history_snapshots ON edit_history(asset_id, snapshot_name, id);
