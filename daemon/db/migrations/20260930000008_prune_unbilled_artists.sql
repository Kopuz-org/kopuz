-- Artist rows nothing credits, bills or queues any more, such as the ones derived albums moved off.
DELETE FROM artists
 WHERE NOT EXISTS (SELECT 1 FROM track_credits c WHERE c.artist_pk = artists.id)
   AND NOT EXISTS (SELECT 1 FROM albums a WHERE a.artist_pk = artists.id)
   AND NOT EXISTS (SELECT 1 FROM queue_credits q WHERE q.artist_pk = artists.id);
