-- One row per credited artist, carrying the id its source issued; it replaces the names-only JSON list.
CREATE TABLE track_credits (
    track_pk  INTEGER NOT NULL REFERENCES tracks(rowid_pk) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    name      TEXT NOT NULL,
    artist_id TEXT,
    PRIMARY KEY (track_pk, position)
);
CREATE INDEX idx_track_credits_artist ON track_credits(artist_id) WHERE artist_id IS NOT NULL;

INSERT INTO track_credits (track_pk, position, name, artist_id)
SELECT t.rowid_pk, a.key, TRIM(a.value), NULL
  FROM tracks t, json_each(t.artists_json) AS a
 WHERE TRIM(a.value) != '';
INSERT INTO track_credits (track_pk, position, name, artist_id)
SELECT t.rowid_pk, 0, TRIM(t.artist), NULL
  FROM tracks t
 WHERE t.artists_json = '[]' AND TRIM(t.artist) != '';

ALTER TABLE tracks DROP COLUMN artists_json;
