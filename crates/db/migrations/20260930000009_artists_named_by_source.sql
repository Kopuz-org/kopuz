-- Whether a linked artist's name came from the source's own record rather than the text some credit carried.
ALTER TABLE artists ADD COLUMN named_by_source INTEGER NOT NULL DEFAULT 0;
