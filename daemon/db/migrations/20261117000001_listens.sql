-- One row per counted listen, for the yearly recap. The track's names are kept
-- as played, since a streamed track may never be stored in `tracks`, and
-- `utc_offset` is the local offset at the time, so a listen keeps the calendar
-- day it was heard on.
CREATE TABLE listens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    source      TEXT    NOT NULL,
    track_key   TEXT    NOT NULL,
    played_at   INTEGER NOT NULL,
    utc_offset  INTEGER NOT NULL DEFAULT 0,
    duration_ms INTEGER NOT NULL DEFAULT 0,
    title       TEXT    NOT NULL DEFAULT '',
    artist      TEXT    NOT NULL DEFAULT '',
    artist_key  TEXT,
    album       TEXT    NOT NULL DEFAULT '',
    album_id    TEXT    NOT NULL DEFAULT ''
);

CREATE INDEX idx_listens_source_played ON listens (source, played_at);
