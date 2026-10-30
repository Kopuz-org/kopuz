-- Kopuz SQLite schema (issue #347). Timestamp-versioned; never edit a merged
-- migration — add a new timestamped file (sqlx migrate add <name>).

-- App config: a single-row JSON blob (loaded/saved wholesale; never field-queried).
CREATE TABLE app_config (
    id   INTEGER PRIMARY KEY CHECK (id = 1),
    json TEXT NOT NULL
);

-- Per-track play counts: carved out of config so a play is a 1-row UPSERT, not a
-- whole-blob rewrite.
CREATE TABLE listen_counts (
    track_key TEXT PRIMARY KEY NOT NULL,
    count     INTEGER NOT NULL DEFAULT 0
);

-- Media servers own their own creds (no longer in config). Active server is
-- config['active_server_id'] (NULL => local).
CREATE TABLE servers (
    id                TEXT PRIMARY KEY NOT NULL,
    name              TEXT NOT NULL,
    url               TEXT NOT NULL,
    service           TEXT NOT NULL,            -- 'Jellyfin' | 'Subsonic' | 'Custom' | 'YtMusic'
    access_token      TEXT,                      -- creds live HERE (Jellyfin/Subsonic token, YT cookie jar)
    user_id           TEXT,
    yt_browser        TEXT,
    yt_anonymous      INTEGER NOT NULL DEFAULT 0,
    extra             TEXT,                      -- per-service spare JSON
    auth_state        TEXT NOT NULL DEFAULT 'unauthenticated', -- 'active'|'expired'|'unauthenticated'
    last_validated_at INTEGER,
    cred_updated_at   INTEGER,
    tier              TEXT NOT NULL DEFAULT 'unknown',         -- YT Premium: 'premium'|'free'|'unknown'
    created_at        INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at        INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Albums. source = 'local' | servers.id
CREATE TABLE albums (
    rowid_pk        INTEGER PRIMARY KEY,
    source          TEXT NOT NULL,
    source_album_id TEXT NOT NULL,
    title           TEXT NOT NULL,
    artist          TEXT NOT NULL,
    genre           TEXT NOT NULL DEFAULT '',
    year            INTEGER NOT NULL DEFAULT 0,
    cover_path      TEXT,
    manual_cover    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (source, source_album_id)
);
CREATE INDEX idx_albums_source ON albums(source);
CREATE INDEX idx_albums_artist ON albums(source, artist);

-- Tracks. Identity = (source, track_key): local => filesystem path, server => item/video id.
CREATE TABLE tracks (
    rowid_pk         INTEGER PRIMARY KEY,
    source           TEXT NOT NULL,             -- 'local' | servers.id
    track_key        TEXT NOT NULL,             -- path (local) | item/video id (server)
    path             TEXT,                       -- filesystem path (local only)
    service          TEXT,                       -- protocol for server tracks (NULL for local)
    source_album_id  TEXT NOT NULL DEFAULT '',
    title            TEXT NOT NULL DEFAULT '',
    artist           TEXT NOT NULL DEFAULT '',
    album            TEXT NOT NULL DEFAULT '',
    duration         INTEGER NOT NULL DEFAULT 0,
    khz              INTEGER NOT NULL DEFAULT 0,
    bitrate          INTEGER NOT NULL DEFAULT 0,
    track_number     INTEGER,
    disc_number      INTEGER,
    mb_release_id    TEXT,
    mb_recording_id  TEXT,
    mb_track_id      TEXT,
    playlist_item_id TEXT,
    artists_json     TEXT NOT NULL DEFAULT '[]',
    cover_path       TEXT,
    UNIQUE (source, track_key)
);
CREATE INDEX idx_tracks_source ON tracks(source);
CREATE INDEX idx_tracks_album  ON tracks(source, source_album_id);
CREATE INDEX idx_tracks_artist ON tracks(source, artist);
CREATE INDEX idx_tracks_path   ON tracks(path);
CREATE INDEX idx_tracks_title  ON tracks(source, title);

-- Playlists + ordered membership.
CREATE TABLE playlists (
    rowid_pk     INTEGER PRIMARY KEY,
    source       TEXT NOT NULL,
    source_pl_id TEXT NOT NULL,
    name         TEXT NOT NULL,
    cover_path   TEXT,
    image_tag    TEXT,
    position     INTEGER NOT NULL DEFAULT 0,
    UNIQUE (source, source_pl_id)
);
CREATE INDEX idx_playlists_source ON playlists(source);

CREATE TABLE playlist_tracks (
    playlist_pk INTEGER NOT NULL REFERENCES playlists(rowid_pk) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    track_ref   TEXT NOT NULL,
    PRIMARY KEY (playlist_pk, position)
);
CREATE INDEX idx_pltracks_ref ON playlist_tracks(track_ref);

CREATE TABLE folders (
    id     TEXT PRIMARY KEY NOT NULL,
    source TEXT NOT NULL,
    name   TEXT NOT NULL
);
CREATE TABLE folder_playlists (
    folder_id    TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    playlist_ref TEXT NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (folder_id, playlist_ref)
);

-- Favorites, per server (server_id = 'local' for filesystem). dirty = optimistic
-- local toggle not yet pushed to remote.
CREATE TABLE favorites (
    server_id  TEXT NOT NULL,
    ref        TEXT NOT NULL,
    dirty      INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (server_id, ref)
);
CREATE INDEX idx_favorites_server ON favorites(server_id);

-- Read-through enrichment cache (MusicBrainz/Last.fm/iTunes covers + artist images).
CREATE TABLE metadata_cache (
    cache_key       TEXT NOT NULL,
    kind            TEXT NOT NULL,
    mb_release_id   TEXT,
    mb_recording_id TEXT,
    mb_track_id     TEXT,
    cover_ref       TEXT,
    artist_image    TEXT,
    duration        INTEGER,
    payload         TEXT,
    fetched_at      INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (cache_key, kind)
);
CREATE INDEX idx_metacache_mbrel ON metadata_cache(mb_release_id);

CREATE TABLE artist_images (
    artist_norm TEXT NOT NULL,
    kind        TEXT NOT NULL,    -- 'server' | 'local' | 'custom'
    image_ref   TEXT NOT NULL,
    PRIMARY KEY (artist_norm, kind)
);

-- Persistent lyrics cache (replaces the in-memory LRU). kind: 'synced_word'|'plain'|'none'.
CREATE TABLE lyrics_cache (
    key_hash   TEXT PRIMARY KEY NOT NULL,
    kind       TEXT NOT NULL,
    content    TEXT NOT NULL DEFAULT '',
    source     TEXT NOT NULL DEFAULT '',
    fetched_at INTEGER NOT NULL DEFAULT (unixepoch())
);

-- Queue/progress snapshot (single row).
CREATE TABLE queue_state (
    id                  INTEGER PRIMARY KEY CHECK (id = 1),
    version             INTEGER NOT NULL DEFAULT 1,
    queue_json          TEXT NOT NULL DEFAULT '[]',
    current_queue_index INTEGER NOT NULL DEFAULT 0,
    progress_secs       INTEGER NOT NULL DEFAULT 0,
    shuffle_order_json  TEXT NOT NULL DEFAULT '[]',
    shuffle_enabled     INTEGER NOT NULL DEFAULT 0
);
