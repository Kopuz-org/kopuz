-- Which tracks have a downloaded copy and where, written a row at a time instead of patched into the config blob.
CREATE TABLE offline_tracks (
    item_id TEXT NOT NULL PRIMARY KEY,
    path    TEXT NOT NULL
);
INSERT INTO offline_tracks (item_id, path)
SELECT o.key, o.value FROM app_config c, json_each(c.json, '$.offline_tracks') o
 WHERE c.id = 1 AND o.type = 'text';
UPDATE app_config SET json = json_remove(json, '$.offline_tracks') WHERE id = 1;

-- A pinned station; its manifest is the registry's own published document, kept as it came.
CREATE TABLE pinned_stations (
    id       TEXT NOT NULL PRIMARY KEY,
    position INTEGER NOT NULL UNIQUE,
    manifest TEXT NOT NULL CHECK (json_valid(manifest))
);
INSERT OR IGNORE INTO pinned_stations (id, position, manifest)
SELECT json_extract(p.value, '$.id'), p.key, p.value FROM app_config c, json_each(c.json, '$.pinned_stations') p
 WHERE c.id = 1 AND p.type = 'text' AND json_valid(p.value) AND json_type(p.value, '$.id') = 'text';
UPDATE app_config SET json = json_remove(json, '$.pinned_stations') WHERE id = 1;
