-- Phase 7b (docs/superpowers/plans/2026-09-22-phase-7b-people-and-search.md, Task 3).
CREATE TABLE people (
    id             INTEGER PRIMARY KEY,
    name           TEXT,
    cover_face_id  INTEGER,
    hidden         INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL
);

CREATE TABLE faces (
    id         INTEGER PRIMARY KEY,
    asset_id   INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    -- 0..1 of the displayed photo, so a face survives a re-rendered derivative.
    x          REAL NOT NULL,
    y          REAL NOT NULL,
    w          REAL NOT NULL,
    h          REAL NOT NULL,
    score      REAL NOT NULL,
    vec        BLOB NOT NULL,
    model      TEXT NOT NULL,
    person_id  INTEGER REFERENCES people(id) ON DELETE SET NULL,
    -- The user put it there; clustering never moves it.
    pinned     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX faces_asset ON faces(asset_id);
CREATE INDEX faces_person ON faces(person_id);

-- Which photos have been looked at for faces, by which detector (most have none).
CREATE TABLE face_scan (
    asset_id  INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    model     TEXT NOT NULL
);

ALTER TABLE assets ADD COLUMN scene_tags TEXT;
