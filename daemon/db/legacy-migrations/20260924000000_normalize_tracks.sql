-- A playlist entry's id belongs to the entry: one per track let a second playlist overwrite the first's.
ALTER TABLE playlist_tracks ADD COLUMN item_id TEXT;

CREATE TABLE track_credits (
    track_pk  INTEGER NOT NULL REFERENCES tracks(rowid_pk) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    name      TEXT NOT NULL,
    artist_id TEXT,
    PRIMARY KEY (track_pk, position)
);
CREATE INDEX idx_track_credits_artist ON track_credits(artist_id) WHERE artist_id IS NOT NULL;

-- A row without credits keeps its names as unlinked credits, the same fallback the wire applies.
INSERT INTO track_credits (track_pk, position, name, artist_id)
SELECT t.rowid_pk, c.key, TRIM(json_extract(c.value, '$.name')), json_extract(c.value, '$.id')
  FROM tracks t, json_each(t.credits_json) AS c
 WHERE t.credits_json != '[]' AND TRIM(json_extract(c.value, '$.name')) != '';
INSERT INTO track_credits (track_pk, position, name, artist_id)
SELECT t.rowid_pk, a.key, TRIM(a.value), NULL
  FROM tracks t, json_each(t.artists_json) AS a
 WHERE t.credits_json = '[]' AND TRIM(a.value) != '';
INSERT INTO track_credits (track_pk, position, name, artist_id)
SELECT t.rowid_pk, 0, TRIM(t.artist), NULL
  FROM tracks t
 WHERE t.credits_json = '[]' AND t.artists_json = '[]' AND TRIM(t.artist) != '';

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
ALTER TABLE tracks DROP COLUMN artists_json;
ALTER TABLE tracks DROP COLUMN credits_json;
ALTER TABLE tracks DROP COLUMN mb_release_id;
ALTER TABLE tracks DROP COLUMN mb_recording_id;
ALTER TABLE tracks DROP COLUMN mb_track_id;
