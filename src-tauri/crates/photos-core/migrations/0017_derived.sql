-- Phase 8b (docs/superpowers/specs/2026-09-22-lumen-develop-design.md §5): an export added
-- back to the library remembers the photo it was made from, for the info panel's
-- "Exported from …". Deleting the source forgets the link, never the export.
ALTER TABLE assets ADD COLUMN derived_from INTEGER REFERENCES assets(id) ON DELETE SET NULL;
