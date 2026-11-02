-- The album's own artist id, as the source bills it; a sync fills it on the next upsert.
ALTER TABLE albums ADD COLUMN artist_id TEXT;
