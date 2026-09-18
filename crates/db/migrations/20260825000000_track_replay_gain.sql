-- ReplayGain values as the media server reported them: gains in dB relative to
-- the reference loudness, peaks as a linear sample amplitude where 1.0 is full
-- scale. NULL where the source publishes none (every local track, and any
-- server that predates the field). The player then reads the tags off the
-- stream it is decoding instead.
ALTER TABLE tracks ADD COLUMN rg_track_gain REAL;
ALTER TABLE tracks ADD COLUMN rg_track_peak REAL;
ALTER TABLE tracks ADD COLUMN rg_album_gain REAL;
ALTER TABLE tracks ADD COLUMN rg_album_peak REAL;
