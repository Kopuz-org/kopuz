-- When each track was last played; a play still in the recent history at the upgrade counts as just played, every older one stays undated.
ALTER TABLE listen_counts ADD COLUMN last_played_at INTEGER;
UPDATE listen_counts SET last_played_at = unixepoch()
 WHERE EXISTS (SELECT 1 FROM recently_played rp
                WHERE rp.source = listen_counts.source AND rp.track_key = listen_counts.track_key);
