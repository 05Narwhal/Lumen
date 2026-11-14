-- Lumen library schema v1. See docs/superpowers/specs/2026-09-17-photos-app-design.md §5.2.

CREATE TABLE IF NOT EXISTS schema_version (
  version    INTEGER NOT NULL,
  applied_at INTEGER NOT NULL
);

CREATE TABLE devices (
  id            INTEGER PRIMARY KEY,
  make          TEXT,
  model         TEXT,
  body_serial   TEXT,
  display_name  TEXT NOT NULL,
  kind          INTEGER NOT NULL,
  icon          TEXT,
  first_seen_at INTEGER NOT NULL,
  UNIQUE(make, model, body_serial)
);

CREATE TABLE import_batches (
  id            INTEGER PRIMARY KEY,
  source_kind   INTEGER NOT NULL,
  source_label  TEXT NOT NULL,
  started_at    INTEGER NOT NULL,
  finished_at   INTEGER,
  item_count    INTEGER NOT NULL DEFAULT 0,
  skipped_count INTEGER NOT NULL DEFAULT 0,
  state         INTEGER NOT NULL
);

CREATE TABLE assets (
  id                INTEGER PRIMARY KEY,
  content_hash      TEXT NOT NULL UNIQUE,
  relative_path     TEXT NOT NULL,
  filename          TEXT NOT NULL,
  extension         TEXT NOT NULL,
  media_type        INTEGER NOT NULL,
  byte_size         INTEGER NOT NULL,
  width             INTEGER,
  height            INTEGER,
  orientation       INTEGER NOT NULL DEFAULT 1,
  duration_ms       INTEGER,
  captured_at       INTEGER,
  captured_at_utc   INTEGER,
  tz_offset_minutes INTEGER,
  imported_at       INTEGER NOT NULL,
  file_modified_at  INTEGER,
  device_id         INTEGER REFERENCES devices(id),
  import_batch_id   INTEGER REFERENCES import_batches(id),
  gps_lat           REAL,
  gps_lon           REAL,
  gps_altitude      REAL,
  thumbhash         BLOB,
  dominant_color    TEXT,
  rating            INTEGER NOT NULL DEFAULT 0,
  is_favorite       INTEGER NOT NULL DEFAULT 0,
  state             INTEGER NOT NULL DEFAULT 0,
  trashed_at        INTEGER,
  live_photo_group  TEXT,
  metadata_json     TEXT
);

CREATE INDEX idx_assets_timeline ON assets(state, captured_at DESC, id DESC);
CREATE INDEX idx_assets_device   ON assets(device_id, state, captured_at DESC, id DESC);
CREATE INDEX idx_assets_batch    ON assets(import_batch_id);
CREATE INDEX idx_assets_favorite ON assets(is_favorite, captured_at DESC) WHERE is_favorite = 1;
CREATE INDEX idx_assets_trash    ON assets(trashed_at) WHERE state = 1;

CREATE VIRTUAL TABLE assets_geo USING rtree(id, min_lat, max_lat, min_lon, max_lon);

CREATE TABLE albums (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  kind           INTEGER NOT NULL,
  query_json     TEXT,
  cover_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
  parent_id      INTEGER REFERENCES albums(id) ON DELETE CASCADE,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL
);

CREATE TABLE album_items (
  album_id  INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
  asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  position  REAL NOT NULL,
  added_at  INTEGER NOT NULL,
  PRIMARY KEY (album_id, asset_id)
);
CREATE INDEX idx_album_items_order ON album_items(album_id, position);

CREATE TABLE edits (
  asset_id    INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  recipe_json TEXT NOT NULL,
  version     INTEGER NOT NULL DEFAULT 1,
  updated_at  INTEGER NOT NULL
);

CREATE TABLE derivatives (
  content_hash  TEXT NOT NULL,
  kind          INTEGER NOT NULL,
  size          INTEGER NOT NULL,
  edit_version  INTEGER NOT NULL DEFAULT 0,
  relative_path TEXT NOT NULL,
  byte_size     INTEGER NOT NULL,
  generated_at  INTEGER NOT NULL,
  PRIMARY KEY (content_hash, kind, size, edit_version)
);

CREATE TABLE memories (
  id             INTEGER PRIMARY KEY,
  kind           INTEGER NOT NULL,
  title          TEXT NOT NULL,
  subtitle       TEXT,
  start_at       INTEGER NOT NULL,
  end_at         INTEGER NOT NULL,
  centroid_lat   REAL,
  centroid_lon   REAL,
  cover_asset_id INTEGER REFERENCES assets(id) ON DELETE SET NULL,
  generated_at   INTEGER NOT NULL,
  is_hidden      INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE memory_items (
  memory_id INTEGER NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
  asset_id  INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  score     REAL NOT NULL,
  PRIMARY KEY (memory_id, asset_id)
);

CREATE TABLE slideshows (
  id            INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  document_json TEXT NOT NULL,
  created_at    INTEGER NOT NULL,
  updated_at    INTEGER NOT NULL
);

CREATE VIRTUAL TABLE assets_fts USING fts5(
  filename, device_name, place_name, keywords,
  content='', tokenize='unicode61 remove_diacritics 2'
);
