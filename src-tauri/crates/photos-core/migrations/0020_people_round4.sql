-- Round 4B (plan 2026-09-25-round-4b-people.md).
-- Why a detection is not a person's face: NULL = kept; 'screen', 'drawing', 'print',
-- 'statue' (the crop check), 'user' (Not a person). A rejected face has no person and is
-- never grouped, counted or suggested, but stays, so a rescan does not find it again as new.
ALTER TABLE faces ADD COLUMN rejected TEXT;
-- 0..1 from the detector's score and the face's size: ranks exemplars and covers.
ALTER TABLE faces ADD COLUMN quality REAL;
-- 'listed' or 'removed' (Remove from People: a real person not wanted). Replaces hidden,
-- which stays as a dead column (dropping it would rebuild the table).
ALTER TABLE people ADD COLUMN status TEXT NOT NULL DEFAULT 'listed';
UPDATE people SET status = 'removed' WHERE hidden = 1;
-- "This face is not that person": a No to a suggestion, or a face moved out of a person.
CREATE TABLE face_rejections (
    face_id    INTEGER NOT NULL REFERENCES faces(id) ON DELETE CASCADE,
    person_id  INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (face_id, person_id)
);
CREATE INDEX faces_rejected ON faces(rejected) WHERE rejected IS NOT NULL;
