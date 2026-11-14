-- Round 4D: pairs the user said are not duplicates. Stored lower id first.
CREATE TABLE duplicate_dismissals (
    a INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    b INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (a, b)
);
