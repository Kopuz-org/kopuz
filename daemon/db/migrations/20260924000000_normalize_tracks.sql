-- A playlist entry's id belongs to the entry: one per track let a second playlist overwrite the first's.
ALTER TABLE playlist_tracks ADD COLUMN item_id TEXT;

CREATE TABLE track_musicbrainz (
    track_pk     INTEGER PRIMARY KEY REFERENCES tracks(rowid_pk) ON DELETE CASCADE,
    release_id   TEXT,
    recording_id TEXT,
    track_id     TEXT
);
INSERT INTO track_musicbrainz (track_pk, release_id, recording_id, track_id)
SELECT rowid_pk, mb_release_id, mb_recording_id, mb_track_id
  FROM tracks
 WHERE COALESCE(mb_release_id, mb_recording_id, mb_track_id) IS NOT NULL;

ALTER TABLE tracks DROP COLUMN playlist_item_id;
ALTER TABLE tracks DROP COLUMN mb_release_id;
ALTER TABLE tracks DROP COLUMN mb_recording_id;
ALTER TABLE tracks DROP COLUMN mb_track_id;
