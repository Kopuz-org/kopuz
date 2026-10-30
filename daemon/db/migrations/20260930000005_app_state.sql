-- What using the app changes, as it stood when it last ran; the settings file holds only settings.
CREATE TABLE app_state (
    id                        INTEGER PRIMARY KEY CHECK (id = 1),
    device_id                 TEXT NOT NULL,
    active_source             TEXT NOT NULL,
    source_explicitly_set     INTEGER NOT NULL,
    volume                    REAL NOT NULL,
    discord_presence_paused   INTEGER,
    fullscreen_tabs_collapsed INTEGER NOT NULL,
    sort_order                TEXT NOT NULL,
    album_view_mode           TEXT NOT NULL,
    artist_album_view_mode    TEXT NOT NULL,
    artists_view_mode         TEXT NOT NULL,
    artist_view_order         TEXT NOT NULL,
    listen_now_style          TEXT NOT NULL,
    hero_height               INTEGER NOT NULL
);

-- The sort a view was left in, one criterion per row in precedence order.
CREATE TABLE view_sorts (
    view      TEXT NOT NULL CHECK (view IN ('albums', 'library', 'artist_albums', 'artists')),
    position  INTEGER NOT NULL,
    field     TEXT NOT NULL,
    direction TEXT NOT NULL CHECK (direction IN ('Asc', 'Desc')),
    PRIMARY KEY (view, position)
);

CREATE TABLE sidebar_items (
    position INTEGER PRIMARY KEY,
    item     TEXT NOT NULL
);

CREATE TABLE home_sections (
    position INTEGER PRIMARY KEY,
    key      TEXT NOT NULL,
    enabled  INTEGER NOT NULL
);

-- Integration credentials by the config field they fill; a settings file never holds them.
CREATE TABLE integration_credentials (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL CHECK (value != '')
);

CREATE TABLE ytdlp_history (
    position INTEGER PRIMARY KEY,
    url      TEXT NOT NULL,
    title    TEXT NOT NULL,
    format   TEXT NOT NULL,
    status   TEXT NOT NULL,
    error    TEXT
);

-- A folder-tree server's library roots; none means auto-detect.
CREATE TABLE server_folders (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    position  INTEGER NOT NULL,
    path      TEXT NOT NULL,
    PRIMARY KEY (server_id, position)
);
