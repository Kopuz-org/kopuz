-- Credited artists with the id their source issued, so an artist opens by what
-- the source called it rather than by a name the daemon has to resolve back.
-- Parallel to artists_json, which stays the names alone: an older build reads
-- that column as a plain string array and must keep working.
ALTER TABLE tracks ADD COLUMN credits_json TEXT NOT NULL DEFAULT '[]';
