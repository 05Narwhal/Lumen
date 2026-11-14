-- Where `captured_at` came from (photos_media::CaptureSource): the shutter, the editor's
-- CreateDate, XMP, GPS, an edit time, or the file itself. NULL means "imported before
-- Lumen read dates properly", which is the backfill's worklist.
ALTER TABLE assets ADD COLUMN capture_source TEXT;
