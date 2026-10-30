-- Every (artist, track) credit of a source: a credit row, or a track on a non-derived album billed to the artist.
CREATE VIEW artist_credit_rows AS
SELECT ca.source AS source, c.artist_pk AS artist_pk, c.track_pk AS track_pk
  FROM track_credits c JOIN artists ca ON ca.id = c.artist_pk
UNION
SELECT al.source, al.artist_pk, bt.rowid_pk
  FROM albums al
  JOIN tracks bt ON bt.source = al.source AND bt.source_album_id = al.source_album_id
 WHERE al.artist_pk IS NOT NULL AND al.derived = 0;

-- The earliest covered album of each credited artist; a bare column beside MIN() is read from the minimum's row.
CREATE VIEW artist_cover_albums AS
SELECT cr.source AS source, cr.artist_pk AS artist_pk, al.cover_path AS cover_path, MIN(al.rowid_pk) AS album_pk
  FROM artist_credit_rows cr
  JOIN tracks t ON t.rowid_pk = cr.track_pk
  JOIN albums al ON al.source = t.source AND al.source_album_id = t.source_album_id
 WHERE al.cover_path IS NOT NULL
 GROUP BY cr.artist_pk;
