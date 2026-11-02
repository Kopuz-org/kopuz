-- What the queue was built from, in the daemon's own encoding; NULL for a raw track list.
ALTER TABLE queue_state ADD COLUMN origin TEXT;
