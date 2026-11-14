-- Round 6: what a track sounds like, and the best part of each video.

-- A saved track's measured sound (`music_profile::TrackProfile` as JSON), keyed like
-- `track_analysis` by its file name in the library's music folder and invalidated when the
-- file's size or modification time changes, or the profile's version moves on.
CREATE TABLE track_profiles (
  file_name    TEXT PRIMARY KEY,
  byte_size    INTEGER NOT NULL,
  modified     INTEGER NOT NULL,
  version      INTEGER NOT NULL,
  profile_json TEXT NOT NULL
);

-- The part of a video a slideshow uses (`video_highlight::best_window`). `analyzer` names
-- what scored it, so a model arriving later re-scores the videos already seen.
CREATE TABLE video_highlights (
  asset_id    INTEGER PRIMARY KEY REFERENCES assets(id) ON DELETE CASCADE,
  analyzer    TEXT NOT NULL,
  in_ms       INTEGER NOT NULL,
  out_ms      INTEGER NOT NULL,
  score       REAL NOT NULL,
  computed_at INTEGER NOT NULL
);
