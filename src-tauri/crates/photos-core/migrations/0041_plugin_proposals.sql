-- Round 25 P4b: what a plugin proposes for a photo (a place, a date, tags, a label, a person's
-- name, a title or a caption), shown on the review screen. Derived data: nothing here changes a
-- photo until the person accepts it, and every row can be dropped and proposed again.
CREATE TABLE plugin_proposals (
  id          INTEGER PRIMARY KEY,
  plugin_id   TEXT NOT NULL,
  asset_id    INTEGER NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,      -- location | date | tags | label | person | title | caption
  value       TEXT NOT NULL,      -- JSON of the kind's value
  confidence  REAL NOT NULL,      -- 0..1
  reason      TEXT,
  status      TEXT NOT NULL DEFAULT 'pending',  -- pending | accepted | rejected
  created_at  INTEGER NOT NULL,
  decided_at  INTEGER,
  UNIQUE (plugin_id, asset_id, kind)
);
CREATE INDEX plugin_proposals_review ON plugin_proposals(kind, status, id);
