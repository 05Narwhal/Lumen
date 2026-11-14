-- Round 4C: pets, one group per animal, mirroring people / faces / face_scan / face_rejections.
CREATE TABLE pets (
    id                  INTEGER PRIMARY KEY,
    name                TEXT,
    -- 'cat' or 'dog': a group never mixes them.
    species             TEXT NOT NULL,
    -- 'listed', or 'removed' (Remove from Pets: kept, so its look-alikes join it quietly).
    status              TEXT NOT NULL DEFAULT 'listed',
    cover_detection_id  INTEGER,
    created_at          INTEGER NOT NULL
);

CREATE TABLE pet_detections (
    id        INTEGER PRIMARY KEY,
    asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    species   TEXT NOT NULL,
    -- 0..1 of the unedited, display-oriented photo, like faces.
    x         REAL NOT NULL,
    y         REAL NOT NULL,
    w         REAL NOT NULL,
    h         REAL NOT NULL,
    score     REAL NOT NULL,
    vec       BLOB NOT NULL,
    model     TEXT NOT NULL,
    pet_id    INTEGER REFERENCES pets(id) ON DELETE SET NULL,
    -- The user put it there; grouping never moves it.
    pinned    INTEGER NOT NULL DEFAULT 0,
    -- Why it is not a pet: 'drawing', 'screen', 'toy', 'print', or 'user' (Not a pet).
    rejected  TEXT,
    quality   REAL,
    -- 'scan' or 'manual' (a box the user drew).
    source    TEXT NOT NULL DEFAULT 'scan'
);
CREATE INDEX pet_detections_asset ON pet_detections(asset_id);
CREATE INDEX pet_detections_pet ON pet_detections(pet_id);
CREATE INDEX pet_detections_unplaced ON pet_detections(asset_id) WHERE pet_id IS NULL AND rejected IS NULL;

CREATE TABLE pet_scan (
    asset_id  INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
    model     TEXT NOT NULL
);

CREATE TABLE pet_rejections (
    detection_id  INTEGER NOT NULL REFERENCES pet_detections(id) ON DELETE CASCADE,
    pet_id        INTEGER NOT NULL REFERENCES pets(id) ON DELETE CASCADE,
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (detection_id, pet_id)
);
