-- Import batches remember where they came from; per-file failures are recorded, never fatal.

ALTER TABLE import_batches ADD COLUMN source_path TEXT;

CREATE TABLE import_failures (
  id        INTEGER PRIMARY KEY,
  batch_id  INTEGER NOT NULL REFERENCES import_batches(id) ON DELETE CASCADE,
  path      TEXT NOT NULL,
  reason    TEXT NOT NULL
);
CREATE INDEX idx_import_failures_batch ON import_failures(batch_id);
