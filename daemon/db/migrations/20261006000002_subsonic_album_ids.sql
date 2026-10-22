-- A Subsonic album id carried a cover URL signed with a fresh salt on every listing; the server's id before it is the album.
CREATE TEMP TABLE subsonic_album_ids AS
SELECT DISTINCT source, source_album_id AS old_id,
       CASE WHEN instr(substr(source_album_id, instr(source_album_id, ':') + 1), ':') = 0 THEN source_album_id
            ELSE substr(source_album_id, 1, instr(source_album_id, ':') + instr(substr(source_album_id, instr(source_album_id, ':') + 1), ':') - 1)
       END AS core
  FROM (SELECT source, source_album_id FROM albums
        UNION SELECT source, source_album_id FROM tracks
        UNION SELECT source, source_album_id FROM queue_tracks)
 WHERE source IN (SELECT id FROM servers WHERE service IN ('Subsonic', 'Custom'))
   AND (source_album_id LIKE 'subsonic:%' OR source_album_id LIKE 'custom:%');

-- Rows that meet on one id merge into the synced one, else the oldest; their tracks follow by id below.
DELETE FROM albums
 WHERE rowid_pk IN (
   SELECT a.rowid_pk
     FROM albums a JOIN subsonic_album_ids m ON m.source = a.source AND m.old_id = a.source_album_id
    WHERE EXISTS (
      SELECT 1
        FROM albums b JOIN subsonic_album_ids n ON n.source = b.source AND n.old_id = b.source_album_id
       WHERE b.source = a.source AND n.core = m.core AND b.rowid_pk != a.rowid_pk
         AND (b.derived < a.derived OR (b.derived = a.derived AND b.rowid_pk < a.rowid_pk))));

UPDATE albums
   SET source_album_id = (SELECT m.core FROM subsonic_album_ids m
                           WHERE m.source = albums.source AND m.old_id = albums.source_album_id)
 WHERE EXISTS (SELECT 1 FROM subsonic_album_ids m
                WHERE m.source = albums.source AND m.old_id = albums.source_album_id AND m.core != m.old_id);

UPDATE tracks
   SET source_album_id = (SELECT m.core FROM subsonic_album_ids m
                           WHERE m.source = tracks.source AND m.old_id = tracks.source_album_id)
 WHERE EXISTS (SELECT 1 FROM subsonic_album_ids m
                WHERE m.source = tracks.source AND m.old_id = tracks.source_album_id AND m.core != m.old_id);

UPDATE queue_tracks
   SET source_album_id = (SELECT m.core FROM subsonic_album_ids m
                           WHERE m.source = queue_tracks.source AND m.old_id = queue_tracks.source_album_id)
 WHERE EXISTS (SELECT 1 FROM subsonic_album_ids m
                WHERE m.source = queue_tracks.source AND m.old_id = queue_tracks.source_album_id AND m.core != m.old_id);

DROP TABLE subsonic_album_ids;
