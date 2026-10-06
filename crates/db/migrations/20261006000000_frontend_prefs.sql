-- A frontend's own UI preferences, one opaque string per key; the daemon never reads a value.
CREATE TABLE IF NOT EXISTS frontend_prefs (
    frontend TEXT NOT NULL CHECK (frontend <> ''),
    key      TEXT NOT NULL CHECK (key <> ''),
    value    TEXT NOT NULL,
    PRIMARY KEY (frontend, key)
) WITHOUT ROWID;
