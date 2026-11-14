-- Manual groups of people (Family, Friends, ...). A person may be in several.
CREATE TABLE person_groups (
    id         INTEGER PRIMARY KEY,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    position   INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE person_group_members (
    group_id  INTEGER NOT NULL REFERENCES person_groups(id) ON DELETE CASCADE,
    person_id INTEGER NOT NULL,
    PRIMARY KEY (group_id, person_id)
) WITHOUT ROWID;
CREATE INDEX person_group_members_person ON person_group_members(person_id);
