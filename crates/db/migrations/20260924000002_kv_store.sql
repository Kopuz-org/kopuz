-- metadata_cache only ever held named values; its MusicBrainz, cover and duration columns were never written.
CREATE TABLE kv (
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL,
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (kind, name)
);
INSERT INTO kv (name, kind, value, updated_at)
SELECT cache_key, kind, payload, fetched_at FROM metadata_cache WHERE payload IS NOT NULL;
DROP TABLE metadata_cache;
