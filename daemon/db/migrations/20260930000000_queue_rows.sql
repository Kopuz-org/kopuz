-- One row per queue entry in play order; it carries the whole row, since a queue holds tracks the library never stored.
CREATE TABLE queue_tracks (
    position         INTEGER PRIMARY KEY,
    track_key        TEXT NOT NULL,
    service          TEXT,
    source_album_id  TEXT NOT NULL,
    title            TEXT NOT NULL,
    artist           TEXT NOT NULL,
    album            TEXT NOT NULL,
    duration         INTEGER, -- NULL for a stream with no end
    khz              INTEGER NOT NULL,
    bitrate          INTEGER NOT NULL,
    track_number     INTEGER,
    disc_number      INTEGER,
    cover_path       TEXT,
    mb_release_id    TEXT,
    mb_recording_id  TEXT,
    mb_track_id      TEXT,
    playlist_item_id TEXT
);

-- A queued track's credits; the artist row is not a foreign key, since a stale one only fails to open.
CREATE TABLE queue_credits (
    queue_position   INTEGER NOT NULL REFERENCES queue_tracks(position) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    name             TEXT NOT NULL,
    source_artist_id TEXT,
    source           TEXT,
    artist_pk        INTEGER,
    PRIMARY KEY (queue_position, position)
);

-- The shuffled play order: which queue position plays at each step.
CREATE TABLE queue_shuffle (
    step     INTEGER PRIMARY KEY,
    position INTEGER NOT NULL REFERENCES queue_tracks(position) ON DELETE CASCADE
);
