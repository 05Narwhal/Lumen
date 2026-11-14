-- High dynamic range, as `photos_media::hdr::HdrKind::code`:
--   0 ordinary, 1 gain map (Apple HEIC, Ultra HDR JPEG), 2 PQ, 3 HLG.
-- Nullable would only mean "not looked at yet"; 0 is the honest default and a backfill
-- pass would need to read every file, so existing libraries report ordinary until an
-- asset is re-imported.
ALTER TABLE assets ADD COLUMN hdr INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_assets_hdr ON assets(hdr) WHERE hdr > 0;
