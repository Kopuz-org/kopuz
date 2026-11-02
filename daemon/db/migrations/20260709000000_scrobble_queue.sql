-- Offline scrobble backlog (issue #335). One row per (listen × service): a
-- scrobble that failed with a transient error is stored here and resubmitted
-- later with its original listen timestamp. "The same listen owed to two
-- services" is just two rows sharing (listened_at, artist, title); delivering
-- one service deletes only its row. `listen_info` holds the ListenBrainz
-- additional-info payload (that service's own JSON, not the queue's storage
-- format) and is NULL for Last.fm/Libre.fm. Replaces the earlier JSON queue
-- file, which never shipped, so there is nothing to migrate in.
CREATE TABLE scrobble_queue (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    listened_at INTEGER NOT NULL,
    artist      TEXT    NOT NULL,
    title       TEXT    NOT NULL,
    album       TEXT,
    service     TEXT    NOT NULL,
    listen_info TEXT,
    UNIQUE (listened_at, artist, title, service)
);
