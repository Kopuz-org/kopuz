-- Rows of a server that no longer exists: removing one never took its data with it.
DELETE FROM tracks WHERE source != 'local' AND source NOT LIKE 'local:%' AND source NOT IN (SELECT id FROM servers);
DELETE FROM albums WHERE source != 'local' AND source NOT LIKE 'local:%' AND source NOT IN (SELECT id FROM servers);
DELETE FROM playlists WHERE source != 'local' AND source NOT LIKE 'local:%' AND source NOT IN (SELECT id FROM servers);
DELETE FROM favorites WHERE server_id != 'local' AND server_id NOT LIKE 'local:%' AND server_id NOT IN (SELECT id FROM servers);
DELETE FROM recently_played WHERE source != 'local' AND source NOT LIKE 'local:%' AND source NOT IN (SELECT id FROM servers);

-- A remote row never had a date added; first seen is the closest thing a sync can know.
UPDATE tracks SET added_at = unixepoch() WHERE service IS NOT NULL AND added_at = 0;

-- An album row guessed from one of its tracks bills a guessed artist, so it credits no other track.
ALTER TABLE albums ADD COLUMN derived INTEGER NOT NULL DEFAULT 0;
-- These services list no albums of their own, so every album row they have was built from a track.
UPDATE albums SET derived = 1
 WHERE source IN (SELECT id FROM servers WHERE service IN ('YtMusic', 'SoundCloud'));

-- Every track's album gets a row, the way the favorites import already built them.
INSERT INTO albums (source, source_album_id, title, artist, cover_path, artist_id, derived)
SELECT t.source, t.source_album_id,
       CASE WHEN t.album = '' THEN 'Singles' ELSE t.album END,
       t.artist, t.cover_path,
       CASE WHEN EXISTS (SELECT 1 FROM track_credits c WHERE c.track_pk = t.rowid_pk AND c.name = TRIM(t.artist))
            THEN (SELECT c.artist_id FROM track_credits c WHERE c.track_pk = t.rowid_pk AND c.name = TRIM(t.artist) LIMIT 1)
            ELSE (SELECT c.artist_id FROM track_credits c WHERE c.track_pk = t.rowid_pk ORDER BY c.position LIMIT 1)
       END,
       1
  FROM tracks t
 WHERE t.rowid_pk IN (SELECT MIN(rowid_pk) FROM tracks WHERE source_album_id != '' GROUP BY source, source_album_id)
ON CONFLICT(source, source_album_id) DO NOTHING;

-- Play counts keyed like every other table, not by a string rebuilt from the service name.
CREATE TABLE play_counts (
    source    TEXT NOT NULL,
    track_key TEXT NOT NULL,
    count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (source, track_key)
);
INSERT INTO play_counts (source, track_key, count)
SELECT t.source, t.track_key, lc.count
  FROM tracks t JOIN listen_counts lc ON lc.track_key = CASE
       WHEN t.service IS NOT NULL THEN lower(t.service) || ':' || t.track_key
       WHEN t.source = 'local' THEN t.track_key
       ELSE t.source || '|' || t.track_key END
ON CONFLICT DO NOTHING;
-- A count whose track is gone was shared by every server of its service, so each keeps it.
INSERT INTO play_counts (source, track_key, count)
SELECT s.id, substr(lc.track_key, instr(lc.track_key, ':') + 1), lc.count
  FROM listen_counts lc JOIN servers s ON lower(s.service) = substr(lc.track_key, 1, instr(lc.track_key, ':') - 1)
ON CONFLICT DO NOTHING;
INSERT INTO play_counts (source, track_key, count)
SELECT substr(track_key, 1, instr(track_key, '|') - 1), substr(track_key, instr(track_key, '|') + 1), count
  FROM listen_counts WHERE track_key LIKE 'local:%|%'
ON CONFLICT DO NOTHING;
INSERT INTO play_counts (source, track_key, count)
SELECT 'local', track_key, count FROM listen_counts WHERE track_key LIKE '/%'
ON CONFLICT DO NOTHING;
DROP TABLE listen_counts;
ALTER TABLE play_counts RENAME TO listen_counts;
