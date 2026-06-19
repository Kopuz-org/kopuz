-- Recently-played, per source. Carved out of the config blob (which split it
-- into local vs a single shared `recently_played_server` Vec); keyed by source
-- so local and EACH server keep their own history. Newest first by played_at.
CREATE TABLE recently_played (
    source    TEXT NOT NULL,
    track_key TEXT NOT NULL,
    played_at INTEGER NOT NULL,
    PRIMARY KEY (source, track_key)
);

CREATE INDEX idx_recently_played_source ON recently_played (source, played_at DESC);
