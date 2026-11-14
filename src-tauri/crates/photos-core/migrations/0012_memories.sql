-- What curation needs per photo, computed at ingest from the 256 px image: a sharpness
-- figure (variance of the Laplacian) and a 64-bit perceptual hash for near-duplicates.
ALTER TABLE assets ADD COLUMN sharpness REAL;
ALTER TABLE assets ADD COLUMN phash INTEGER;
-- The curated subset of a memory, shown on its detail page and played.
ALTER TABLE memory_items ADD COLUMN curated INTEGER NOT NULL DEFAULT 0;
-- A memory's identity across regenerations, so hiding one survives them.
CREATE UNIQUE INDEX idx_memories_identity ON memories(kind, start_at, end_at);
