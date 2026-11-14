-- Round 4K: the village or district a photo was taken in ("Kropa, Slovenia"), from the
-- bundled GeoNames cities500, beside place_name's town of 15,000. '' means looked up and
-- nothing within LOCALITY_RADIUS_KM; NULL means not looked up yet (the backfill's worklist).
ALTER TABLE assets ADD COLUMN locality TEXT;
CREATE INDEX idx_assets_locality ON assets(locality) WHERE locality IS NOT NULL AND locality != '';
