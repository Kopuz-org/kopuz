-- An artist id is present or absent, never blank, so no reader has to check.
UPDATE track_credits SET artist_id = NULL WHERE TRIM(artist_id) = '';
UPDATE albums SET artist_id = NULL WHERE TRIM(artist_id) = '';
