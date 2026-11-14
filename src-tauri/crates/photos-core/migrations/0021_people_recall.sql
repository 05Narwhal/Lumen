-- Round 4I (plan 2026-09-26-round-4i-people-recall.md).
-- Where a face came from: 'scan' (the detector), 'manual' (a box the user drew) or
-- 'restored' (the filter rejected it and the user said it is a real face, so a rescan never
-- judges it again). The orphan repair (faces pinned to nobody) runs in Rust, because it
-- needs the embeddings: `people::repair_orphans`.
ALTER TABLE faces ADD COLUMN source TEXT NOT NULL DEFAULT 'scan';
-- Unplaced faces, which recluster, suggestions and the viewer's Unknown faces read.
CREATE INDEX faces_unplaced ON faces(asset_id) WHERE person_id IS NULL AND rejected IS NULL;
