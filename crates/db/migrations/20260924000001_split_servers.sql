-- A server row names the server; its secrets and per-service options are rows beside it that exist only when set.
CREATE TABLE server_credentials (
    server_id    TEXT PRIMARY KEY NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    access_token TEXT NOT NULL,
    user_id      TEXT,
    updated_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
INSERT INTO server_credentials (server_id, access_token, user_id, updated_at)
SELECT id, access_token, user_id, COALESCE(cred_updated_at, updated_at)
  FROM servers
 WHERE access_token IS NOT NULL;

CREATE TABLE server_settings (
    server_id TEXT NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    value     TEXT NOT NULL,
    PRIMARY KEY (server_id, key)
);
INSERT INTO server_settings (server_id, key, value)
SELECT id, 'yt_browser', yt_browser FROM servers WHERE yt_browser IS NOT NULL;
INSERT INTO server_settings (server_id, key, value)
SELECT id, 'yt_anonymous', '1' FROM servers WHERE yt_anonymous != 0;
INSERT INTO server_settings (server_id, key, value)
SELECT id, 'apple_music_storefront', apple_music_storefront FROM servers WHERE apple_music_storefront != 'us';
INSERT INTO server_settings (server_id, key, value)
SELECT id, 'apple_music_language', apple_music_language FROM servers WHERE apple_music_language != 'en';

ALTER TABLE servers DROP COLUMN access_token;
ALTER TABLE servers DROP COLUMN user_id;
ALTER TABLE servers DROP COLUMN yt_browser;
ALTER TABLE servers DROP COLUMN yt_anonymous;
ALTER TABLE servers DROP COLUMN extra;
ALTER TABLE servers DROP COLUMN auth_state;
ALTER TABLE servers DROP COLUMN last_validated_at;
ALTER TABLE servers DROP COLUMN cred_updated_at;
ALTER TABLE servers DROP COLUMN tier;
ALTER TABLE servers DROP COLUMN apple_music_storefront;
ALTER TABLE servers DROP COLUMN apple_music_language;
