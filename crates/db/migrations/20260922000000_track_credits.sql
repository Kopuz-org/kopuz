-- Parallel to artists_json, which stays a plain array of names: an older build
-- parses that column as one and must keep working.
ALTER TABLE tracks ADD COLUMN credits_json TEXT NOT NULL DEFAULT '[]';
