-- "Date added" for a track, as unix seconds. The scan stamps local rows once
-- with the file's birth time (its mtime where the filesystem records no birth
-- time). 0 means unstamped: a server row, or a library not rescanned since this
-- migration. Both orderings fall back to rowid_pk for those.
ALTER TABLE tracks ADD COLUMN added_at INTEGER NOT NULL DEFAULT 0;
CREATE INDEX idx_tracks_added ON tracks(source, added_at);
