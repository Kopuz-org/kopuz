-- Bits per sample of a lossless track; 0 for lossy audio, or until a rescan or sync reads it.
ALTER TABLE tracks ADD COLUMN bit_depth INTEGER NOT NULL DEFAULT 0;
ALTER TABLE queue_tracks ADD COLUMN bit_depth INTEGER NOT NULL DEFAULT 0;
