-- A saved queue origin naming an album 20261006000003 dropped would point at nothing.
UPDATE queue_state SET origin = NULL
 WHERE source IN (SELECT id FROM servers WHERE service = 'YtMusic')
   AND json_valid(origin)
   AND ((json_extract(origin, '$.kind') = 'album'
         AND json_extract(origin, '$.id') LIKE 'ytmusic:album:%' AND json_extract(origin, '$.id') NOT LIKE 'ytmusic:album:MPRE%')
     OR (json_extract(origin, '$.kind') = 'filter'
         AND json_extract(origin, '$.album') LIKE 'ytmusic:album:%' AND json_extract(origin, '$.album') NOT LIKE 'ytmusic:album:MPRE%'));
