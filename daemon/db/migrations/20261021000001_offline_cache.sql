-- Tracks kept from their own stream for offline listening, evicted least recently played first; downloads stay in offline_tracks.
CREATE TABLE offline_cache (
    track_key TEXT PRIMARY KEY NOT NULL,
    source TEXT NOT NULL,
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    content_type TEXT NOT NULL,
    cached_at INTEGER NOT NULL,
    last_played_at INTEGER NOT NULL
);
CREATE INDEX offline_cache_last_played ON offline_cache (last_played_at);
