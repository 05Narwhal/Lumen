-- Round 13: the text read in photos (spec 2026-10-07-round-13-ocr-design.md §1).

-- Which photos have been read, by which pair of models ("<det id>+<rec id>"); a new pair
-- rescans. A photo with no text, or one that would not decode, still gets its row so the
-- background job never loops on it.
CREATE TABLE IF NOT EXISTS ocr_scan (
  asset_id INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  model    TEXT NOT NULL
);

-- One row per line read.
CREATE TABLE IF NOT EXISTS asset_text (
  id       INTEGER PRIMARY KEY,
  asset_id INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  ord      INTEGER NOT NULL,            -- reading order
  text     TEXT NOT NULL,
  conf     REAL NOT NULL,
  quad     BLOB NOT NULL,               -- 4 corners x (x, y) f32 LE, 0..1 of the unedited photo
  script   TEXT NOT NULL,               -- ISO 15924: 'Latn' for now
  lang     TEXT,                        -- NULL until language identification
  alt      TEXT                         -- a close second reading; search only, never shown
);
CREATE INDEX IF NOT EXISTS idx_asset_text_asset ON asset_text(asset_id, ord);

-- assets_fts gains a photo_text column. FTS5 cannot add a column, so the table is dropped
-- and refilled from the catalogue as 0004 did; it is derived data only. The refill is
-- fts::SELECT_ROWS verbatim (photo_text starts empty: nothing has been read yet), with đ/Đ
-- written as d/D as fts::fold_dj does: the tokenizer folds č ć š ž but not đ.
-- No ANALYZE here: that belongs on the maintenance lane.
DROP TABLE IF EXISTS assets_fts;
CREATE VIRTUAL TABLE assets_fts USING fts5(
  filename, device_name, place_name, keywords, photo_text,
  tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO assets_fts(rowid, filename, device_name, place_name, keywords, photo_text)
  SELECT a.id, replace(replace(a.filename, 'đ', 'd'), 'Đ', 'D'), replace(replace(d.display_name, 'đ', 'd'), 'Đ', 'D'),
         replace(replace(NULLIF(TRIM(COALESCE(a.place_name, '') || ' ' || COALESCE(a.locality, '')), ''), 'đ', 'd'), 'Đ', 'D'),
         replace(replace(NULLIF(TRIM(COALESCE(a.title, '') || ' ' || COALESCE(a.caption, '') || ' ' || COALESCE(a.keywords, '')
           || ' ' || COALESCE(a.scene_tags, '')
           || ' ' || COALESCE((SELECT group_concat(DISTINCT p.name) FROM faces f JOIN people p ON p.id = f.person_id
                               WHERE f.asset_id = a.id AND p.name IS NOT NULL AND p.status = 'listed'), '')
           || ' ' || COALESCE((SELECT group_concat(DISTINCT pt.name) FROM pet_detections pd JOIN pets pt ON pt.id = pd.pet_id
                               WHERE pd.asset_id = a.id AND pd.rejected IS NULL AND pt.name IS NOT NULL AND pt.status = 'listed'), '')), ''), 'đ', 'd'), 'Đ', 'D'),
         NULL
  FROM assets a LEFT JOIN devices d ON d.id = a.device_id;
