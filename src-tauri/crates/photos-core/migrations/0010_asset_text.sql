-- Text a source brings with a photo (Apple Photos' title, caption and keywords). Searched
-- through assets_fts's keywords column, which has existed, always NULL, since 0004.
ALTER TABLE assets ADD COLUMN title TEXT;
ALTER TABLE assets ADD COLUMN caption TEXT;
ALTER TABLE assets ADD COLUMN keywords TEXT;
