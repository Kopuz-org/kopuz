-- The global YT sync stamps become the per-source stamps every other source keeps.
INSERT INTO kv (name, kind, value)
SELECT 'synced:favorites', s.id, CAST(json_extract(k.value, '$.last_yt_sync_at') AS TEXT)
  FROM kv k, servers s
 WHERE k.name = 'yt_sync' AND k.kind = 'timestamps' AND s.service = 'YtMusic'
   AND json_valid(k.value) AND json_type(k.value, '$.last_yt_sync_at') = 'integer'
ON CONFLICT(kind, name) DO NOTHING;
INSERT INTO kv (name, kind, value)
SELECT 'synced:playlists', s.id, CAST(json_extract(k.value, '$.last_yt_playlists_sync_at') AS TEXT)
  FROM kv k, servers s
 WHERE k.name = 'yt_sync' AND k.kind = 'timestamps' AND s.service = 'YtMusic'
   AND json_valid(k.value) AND json_type(k.value, '$.last_yt_playlists_sync_at') = 'integer'
ON CONFLICT(kind, name) DO NOTHING;
DELETE FROM kv WHERE name = 'yt_sync' AND kind = 'timestamps';

-- Each file the legacy importer consumed is its own row rather than one entry of a list.
INSERT INTO kv (name, kind, value)
SELECT f.value, 'legacy_import', ''
  FROM kv k, json_each(k.value) f
 WHERE k.name = 'legacy_import' AND k.kind = 'files' AND json_valid(k.value) AND f.type = 'text'
ON CONFLICT(kind, name) DO NOTHING;
DELETE FROM kv WHERE name = 'legacy_import' AND kind = 'files';
