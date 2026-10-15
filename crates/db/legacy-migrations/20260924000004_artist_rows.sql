ALTER TABLE track_credits RENAME TO legacy_track_credits;
DROP INDEX idx_track_credits_artist;

CREATE TABLE artists (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    source           TEXT NOT NULL,
    source_artist_id TEXT CHECK (source_artist_id IS NULL
                                 OR (source_artist_id != '' AND source_artist_id = TRIM(source_artist_id))),
    name             TEXT NOT NULL CHECK (name != '' AND name = TRIM(name)),
    name_key         TEXT NOT NULL CHECK (name_key != ''),
    UNIQUE (source, source_artist_id)
);
CREATE UNIQUE INDEX idx_artists_unlinked ON artists(source, name_key) WHERE source_artist_id IS NULL;

CREATE TABLE track_credits (
    track_pk  INTEGER NOT NULL REFERENCES tracks(rowid_pk) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    artist_pk INTEGER NOT NULL REFERENCES artists(id),
    name      TEXT NOT NULL CHECK (name != '' AND name = TRIM(name)),
    PRIMARY KEY (track_pk, position)
);
CREATE INDEX idx_track_credits_artist ON track_credits(artist_pk);

ALTER TABLE albums ADD COLUMN artist_pk INTEGER REFERENCES artists(id);
CREATE INDEX idx_albums_artist_pk ON albums(artist_pk) WHERE artist_pk IS NOT NULL;
