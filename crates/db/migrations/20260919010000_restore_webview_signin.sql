UPDATE servers
SET access_token = NULL,
    user_id = NULL,
    auth_state = 'unauthenticated',
    cred_updated_at = NULL,
    last_validated_at = NULL,
    updated_at = unixepoch()
WHERE access_token IN ('kopuz:soundcloud:oauth:v1', 'kopuz:youtube:oauth:v1')
   OR access_token GLOB 'kopuz:musickit:v1:*';

DROP TABLE browser_auth;
