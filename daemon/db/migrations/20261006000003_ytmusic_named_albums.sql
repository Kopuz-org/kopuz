-- A YouTube album id minted from its row's album and artist text named no release, so a track keeps no album rather than that guess.
DELETE FROM albums
 WHERE source IN (SELECT id FROM servers WHERE service = 'YtMusic')
   AND source_album_id LIKE 'ytmusic:album:%' AND source_album_id NOT LIKE 'ytmusic:album:MPRE%';

UPDATE tracks SET source_album_id = ''
 WHERE source IN (SELECT id FROM servers WHERE service = 'YtMusic')
   AND source_album_id LIKE 'ytmusic:album:%' AND source_album_id NOT LIKE 'ytmusic:album:MPRE%';

UPDATE queue_tracks SET source_album_id = ''
 WHERE service = 'YtMusic'
   AND source_album_id LIKE 'ytmusic:album:%' AND source_album_id NOT LIKE 'ytmusic:album:MPRE%';
