-- Credentials moved out of servers after the original WebView migration.
DELETE FROM server_credentials
WHERE access_token IN ('kopuz:soundcloud:oauth:v1', 'kopuz:youtube:oauth:v1')
   OR access_token GLOB 'kopuz:musickit:v1:*';

DROP TABLE IF EXISTS browser_auth;
