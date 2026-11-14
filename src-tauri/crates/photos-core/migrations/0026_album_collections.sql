-- Round 5A: collections, folders of albums. An album is in at most one; collections nest.
CREATE TABLE album_collections (
  id         INTEGER PRIMARY KEY,
  name       TEXT NOT NULL,
  parent_id  INTEGER REFERENCES album_collections(id) ON DELETE SET NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX idx_album_collections_parent ON album_collections(parent_id);

ALTER TABLE albums ADD COLUMN collection_id INTEGER REFERENCES album_collections(id) ON DELETE SET NULL;
CREATE INDEX idx_albums_collection ON albums(collection_id);

-- Deleting a collection never deletes what is in it: its albums and collections move up.
CREATE TRIGGER album_collection_delete BEFORE DELETE ON album_collections BEGIN
  UPDATE albums SET collection_id = OLD.parent_id WHERE collection_id = OLD.id;
  UPDATE album_collections SET parent_id = OLD.parent_id WHERE parent_id = OLD.id;
END;
