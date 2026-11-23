-- One row per counted play, so listening can be told over time and not only as an all-time count.
CREATE TABLE listen_events (
    id        INTEGER PRIMARY KEY,
    source    TEXT NOT NULL,
    track_key TEXT NOT NULL,
    played_at INTEGER NOT NULL
);
CREATE INDEX idx_listen_events_time ON listen_events(source, played_at);
