-- Julian's 2026-11-08 list, item 3: an album or collection can show a shape with an icon on it
-- instead of its photos. JSON `{ "kind": "icon", "shape": …, "icon": …, "color": … }`
-- (`photos_core::albums::CoverStyle`); NULL shows its photos, as before.
ALTER TABLE albums ADD COLUMN cover_style TEXT;
ALTER TABLE album_collections ADD COLUMN cover_style TEXT;
