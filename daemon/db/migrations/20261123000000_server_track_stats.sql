-- What a source's server reports per track for the signed-in account: its plays, which already include kopuz's own scrobbles, and its 1-5 star rating.
CREATE TABLE server_track_stats (
    source         TEXT NOT NULL,
    track_key      TEXT NOT NULL,
    play_count     INTEGER,
    last_played_at INTEGER,
    rating         INTEGER,
    PRIMARY KEY (source, track_key)
);

