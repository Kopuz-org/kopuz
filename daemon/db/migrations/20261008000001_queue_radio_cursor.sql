-- The cursor a radio queue asks its source with for the next page; NULL for a
-- queue that is not a radio, or one whose source has nothing more to give.
ALTER TABLE queue_state ADD COLUMN radio_cursor TEXT;
