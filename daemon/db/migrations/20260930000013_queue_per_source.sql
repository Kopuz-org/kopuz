-- Each source keeps its own queue, so switching back resumes where it left off; the one saved so far belongs to the active source.
CREATE TABLE queue_state_new (
    source              TEXT PRIMARY KEY NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    current_queue_index INTEGER NOT NULL DEFAULT 0,
    progress_secs       INTEGER NOT NULL DEFAULT 0,
    shuffle_enabled     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE queue_tracks_new (
    source           TEXT NOT NULL,
    position         INTEGER NOT NULL,
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
    playlist_item_id TEXT,
    PRIMARY KEY (source, position)
);

-- `source` is the listing a credit came from; `queue_source` is whose queue the row is in.
CREATE TABLE queue_credits_new (
    queue_source     TEXT NOT NULL,
    queue_position   INTEGER NOT NULL,
    position         INTEGER NOT NULL,
    name             TEXT NOT NULL,
    source_artist_id TEXT,
    source           TEXT,
    artist_key       TEXT,
    PRIMARY KEY (queue_source, queue_position, position),
    FOREIGN KEY (queue_source, queue_position) REFERENCES queue_tracks_new(source, position) ON DELETE CASCADE
);

CREATE TABLE queue_shuffle_new (
    source   TEXT NOT NULL,
    step     INTEGER NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (source, step),
    FOREIGN KEY (source, position) REFERENCES queue_tracks_new(source, position) ON DELETE CASCADE
);

INSERT INTO queue_state_new (source, version, current_queue_index, progress_secs, shuffle_enabled)
SELECT COALESCE((SELECT active_source FROM app_state WHERE id = 1), 'local'),
       version, current_queue_index, progress_secs, shuffle_enabled
  FROM queue_state;

INSERT INTO queue_tracks_new
SELECT COALESCE((SELECT active_source FROM app_state WHERE id = 1), 'local'), position, track_key, service,
       source_album_id, title, artist, album, duration, khz, bitrate, track_number, disc_number,
       cover_path, mb_release_id, mb_recording_id, mb_track_id, playlist_item_id
  FROM queue_tracks;

INSERT INTO queue_credits_new
SELECT COALESCE((SELECT active_source FROM app_state WHERE id = 1), 'local'), queue_position, position, name,
       source_artist_id, source, artist_key
  FROM queue_credits;

INSERT INTO queue_shuffle_new
SELECT COALESCE((SELECT active_source FROM app_state WHERE id = 1), 'local'), step, position
  FROM queue_shuffle;

DROP TABLE queue_shuffle;
DROP TABLE queue_credits;
DROP TABLE queue_tracks;
DROP TABLE queue_state;
ALTER TABLE queue_state_new RENAME TO queue_state;
ALTER TABLE queue_tracks_new RENAME TO queue_tracks;
ALTER TABLE queue_credits_new RENAME TO queue_credits;
ALTER TABLE queue_shuffle_new RENAME TO queue_shuffle;
