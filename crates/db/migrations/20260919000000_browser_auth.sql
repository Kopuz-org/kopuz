CREATE TABLE browser_auth (
    server_id TEXT PRIMARY KEY REFERENCES servers(id) ON DELETE CASCADE,
    credentials TEXT NOT NULL
);
