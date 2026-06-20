-- Per-sync epoch token for incremental playlist-entry streaming, mirroring the
-- favorites epoch. As a remote playlist pages in, each position is upserted with
-- the current walk's epoch (and its running position as the key); rows whose
-- epoch is stale after the walk completes were removed remotely (or are the
-- shrunken tail of a now-shorter playlist) and are swept. Lets the cached entry
-- list grow live during the walk while deletions apply correctly once the full
-- set is known. Default 0 — rows written before this migration are swept by the
-- first full reconcile.
ALTER TABLE playlist_tracks ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
