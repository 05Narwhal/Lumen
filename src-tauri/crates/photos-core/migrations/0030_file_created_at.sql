-- Round 8A: when the file was created where it was imported from, if that system says
-- (a folder on Windows or macOS; a phone or card often does not). The copy in originals/
-- was created at import, so older rows stay NULL: their source's time is gone.
ALTER TABLE assets ADD COLUMN file_created_at INTEGER;
