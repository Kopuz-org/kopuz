-- Spotify is no longer built in; a marketplace plugin declaring the `spotify` service serves these sources.
-- The stored token was minted for the built-in client, which a plugin cannot use, so the source signs in again.
INSERT OR REPLACE INTO server_settings (server_id, key, value)
SELECT id, 'plugin_service', 'spotify' FROM servers WHERE service = 'Spotify';
DELETE FROM server_credentials WHERE server_id IN (SELECT id FROM servers WHERE service = 'Spotify');
UPDATE tracks SET service = 'Plugin' WHERE service = 'Spotify';
UPDATE queue_tracks SET service = 'Plugin' WHERE service = 'Spotify';
UPDATE servers SET service = 'Plugin' WHERE service = 'Spotify';
