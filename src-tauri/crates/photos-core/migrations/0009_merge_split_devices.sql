-- Merge device sections that were split only because some files omit the body serial.
--
-- The identity was (make, model, body_serial), and a NULL serial compared as its own
-- value, so one camera appeared once per file format that records the serial differently
-- (a Nikon D7100 writes it into DNG but not into NEF). Only groups holding at most one
-- distinct serial are merged: two recorded serials mean two real bodies.
CREATE TEMPORARY TABLE device_merge AS
SELECT MIN(id) AS keep_id, make, model, MAX(body_serial) AS serial
FROM devices
GROUP BY make, model
HAVING COUNT(DISTINCT body_serial) <= 1 AND COUNT(*) > 1;

-- The photos move to the surviving section first.
UPDATE assets
SET device_id = (
  SELECT m.keep_id FROM device_merge m
  JOIN devices d ON d.make IS m.make AND d.model IS m.model
  WHERE d.id = assets.device_id
)
WHERE device_id IN (
  SELECT d.id FROM devices d JOIN device_merge m ON d.make IS m.make AND d.model IS m.model
);

-- Then the now-empty duplicates go, before the survivor adopts the serial: the table has
-- a UNIQUE index on (make, model, body_serial), so adopting it first would collide with
-- the very row being removed.
DELETE FROM devices
WHERE id IN (
  SELECT d.id FROM devices d
  JOIN device_merge m ON d.make IS m.make AND d.model IS m.model
  WHERE d.id <> m.keep_id
);

UPDATE devices
SET body_serial = (SELECT m.serial FROM device_merge m WHERE m.keep_id = devices.id)
WHERE id IN (SELECT keep_id FROM device_merge) AND body_serial IS NULL;

DROP TABLE device_merge;
