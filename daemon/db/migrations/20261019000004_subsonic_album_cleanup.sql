-- A saved queue origin still names an album by its signed id; keep the server's id before the signature, as 20261006000002 did for the rows.
UPDATE queue_state
   SET origin = json_set(origin, '$.id',
         substr(json_extract(origin, '$.id'), 1,
                instr(json_extract(origin, '$.id'), ':')
                + instr(substr(json_extract(origin, '$.id'), instr(json_extract(origin, '$.id'), ':') + 1), ':') - 1))
 WHERE source IN (SELECT id FROM servers WHERE service IN ('Subsonic', 'Custom'))
   AND json_valid(origin) AND json_extract(origin, '$.kind') = 'album'
   AND (json_extract(origin, '$.id') LIKE 'custom:%:%' OR json_extract(origin, '$.id') LIKE 'subsonic:%:%');

UPDATE queue_state
   SET origin = json_set(origin, '$.album',
         substr(json_extract(origin, '$.album'), 1,
                instr(json_extract(origin, '$.album'), ':')
                + instr(substr(json_extract(origin, '$.album'), instr(json_extract(origin, '$.album'), ':') + 1), ':') - 1))
 WHERE source IN (SELECT id FROM servers WHERE service IN ('Subsonic', 'Custom'))
   AND json_valid(origin) AND json_extract(origin, '$.kind') = 'filter'
   AND (json_extract(origin, '$.album') LIKE 'custom:%:%' OR json_extract(origin, '$.album') LIKE 'subsonic:%:%');

-- A song with no albumId was filed under a derived album keyed by its own id; a synced album may share that id on integer-id servers.
CREATE TEMP TABLE song_albums AS
SELECT a.source, a.source_album_id
  FROM albums a
 WHERE a.derived = 1
   AND a.source IN (SELECT id FROM servers WHERE service IN ('Subsonic', 'Custom'))
   AND EXISTS (SELECT 1 FROM tracks t
                WHERE t.source = a.source AND t.source_album_id = a.source_album_id
                  AND a.source_album_id IN ('subsonic:' || t.track_key, 'custom:' || t.track_key));

UPDATE tracks SET source_album_id = ''
 WHERE EXISTS (SELECT 1 FROM song_albums s
                WHERE s.source = tracks.source AND s.source_album_id = tracks.source_album_id);

UPDATE queue_tracks SET source_album_id = ''
 WHERE EXISTS (SELECT 1 FROM song_albums s
                WHERE s.source = queue_tracks.source AND s.source_album_id = queue_tracks.source_album_id);

DELETE FROM albums
 WHERE EXISTS (SELECT 1 FROM song_albums s
                WHERE s.source = albums.source AND s.source_album_id = albums.source_album_id);

DROP TABLE song_albums;

-- A signed getCoverArt URL stored as a cover, bare or inside an item ref, holds a replayable token; keep only the cover id it names.
CREATE TEMP TABLE signed_covers AS
SELECT source, old, prefix || ':' || CASE WHEN instr(rest, '&') > 0 THEN substr(rest, 1, instr(rest, '&') - 1) ELSE rest END AS new
  FROM (SELECT source, old, prefix, substr(url, instr(url, 'id=') + 3) AS rest
          FROM (SELECT c.source, c.old, CAST(unhex(substr(c.old, instr(c.old, 'urlhex_') + 7)) AS TEXT) AS url,
                       CASE s.service WHEN 'Subsonic' THEN 'subsonic' ELSE 'custom' END AS prefix
                  FROM (SELECT source, cover_path AS old FROM albums
                        UNION SELECT source, cover_path FROM tracks
                        UNION SELECT source, cover_path FROM queue_tracks) c
                  JOIN servers s ON s.id = c.source AND s.service IN ('Subsonic', 'Custom')
                 WHERE c.old LIKE 'urlhex_%' OR c.old LIKE '%:urlhex_%')
         WHERE instr(url, 'getCoverArt') > 0 AND instr(url, 'id=') > 0)
 WHERE rest != '';

UPDATE albums SET cover_path = (SELECT c.new FROM signed_covers c WHERE c.source = albums.source AND c.old = albums.cover_path)
 WHERE EXISTS (SELECT 1 FROM signed_covers c WHERE c.source = albums.source AND c.old = albums.cover_path);
UPDATE tracks SET cover_path = (SELECT c.new FROM signed_covers c WHERE c.source = tracks.source AND c.old = tracks.cover_path)
 WHERE EXISTS (SELECT 1 FROM signed_covers c WHERE c.source = tracks.source AND c.old = tracks.cover_path);
UPDATE queue_tracks SET cover_path = (SELECT c.new FROM signed_covers c WHERE c.source = queue_tracks.source AND c.old = queue_tracks.cover_path)
 WHERE EXISTS (SELECT 1 FROM signed_covers c WHERE c.source = queue_tracks.source AND c.old = queue_tracks.cover_path);

DROP TABLE signed_covers;

-- Artist photos were stored as the signed URL itself.
UPDATE artist_images
   SET image_ref = (SELECT CASE s.service WHEN 'Subsonic' THEN 'subsonic' ELSE 'custom' END FROM servers s WHERE s.id = artist_images.source)
                   || ':' || CASE WHEN instr(substr(image_ref, instr(image_ref, 'id=') + 3), '&') > 0
                                  THEN substr(substr(image_ref, instr(image_ref, 'id=') + 3), 1, instr(substr(image_ref, instr(image_ref, 'id=') + 3), '&') - 1)
                                  ELSE substr(image_ref, instr(image_ref, 'id=') + 3) END
 WHERE source IN (SELECT id FROM servers WHERE service IN ('Subsonic', 'Custom'))
   AND kind = 'server' AND image_ref LIKE 'http%' AND instr(image_ref, 'getCoverArt') > 0 AND instr(image_ref, 'id=') > 0;
