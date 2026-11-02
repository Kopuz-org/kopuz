-- The one key an artist is opened by: the id its source issued, else one minted when the row is filed.
ALTER TABLE artists ADD COLUMN key TEXT NOT NULL DEFAULT '';
UPDATE artists SET key = COALESCE(source_artist_id, lower(hex(randomblob(16))));
CREATE UNIQUE INDEX idx_artists_key ON artists(source, key);
-- ADD COLUMN can't take a NOT NULL without a default, so the trigger is what refuses a row filed with none.
CREATE TRIGGER artists_key_required BEFORE INSERT ON artists WHEN NEW.key = ''
BEGIN SELECT RAISE(ABORT, 'an artist row needs a key'); END;

-- A queued credit holds that key rather than a row id, as everything a frontend opens an artist by does.
ALTER TABLE queue_credits ADD COLUMN artist_key TEXT;
UPDATE queue_credits
   SET artist_key = (SELECT key FROM artists WHERE artists.id = queue_credits.artist_pk)
 WHERE artist_pk IS NOT NULL;
ALTER TABLE queue_credits DROP COLUMN artist_pk;
