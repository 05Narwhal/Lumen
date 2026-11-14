-- Round 7: indexes that answer the busiest screens without reading the wide rows.
-- (0028 is Round 6's media shows, on its own branch.)

-- People: a person's faces and their photos straight from the index. With person_id
-- alone, counting each person's photos read every face's row (vector included):
-- list_people 206 → 26 ms on Julian's library.
DROP INDEX IF EXISTS faces_person;
CREATE INDEX faces_person ON faces(person_id, asset_id);

-- Devices: each device's visible count and last import from the index (list_devices
-- 41 → 2 ms).
CREATE INDEX IF NOT EXISTS idx_assets_device_shown
  ON assets(device_id, state, concealed, media_type, live_photo_group, imported_at);
