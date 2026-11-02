-- A radio row's whole station record, so a restored queue can play a station no registry lists any more; NULL for every other row.
ALTER TABLE queue_tracks ADD COLUMN station TEXT;
