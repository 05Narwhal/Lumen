-- Phase 7a (docs/superpowers/specs/2026-09-22-lumen-ml-design.md §4).
CREATE TABLE asset_embeddings (
    asset_id   INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    model      TEXT NOT NULL,
    vec        BLOB NOT NULL,
    aesthetic  REAL
);
CREATE INDEX asset_embeddings_model ON asset_embeddings(model);

CREATE TABLE track_analysis (
    file_name   TEXT PRIMARY KEY,
    byte_size   INTEGER NOT NULL,
    modified    INTEGER NOT NULL,
    analyzer    TEXT NOT NULL,
    bpm         REAL NOT NULL,
    confidence  REAL NOT NULL,
    beats       BLOB NOT NULL,
    downbeats   BLOB NOT NULL,
    energy      REAL NOT NULL
);
