-- Per-sync epoch token for incremental favorites streaming. As the remote
-- liked-songs walk pages in, each ref is upserted with the current sync's epoch
-- (and a running rank); rows whose epoch is stale after the walk completes were
-- unliked remotely and are swept. Lets the list grow live during the walk while
-- deletions apply correctly once the full set is known. Default 0 — backfilled
-- rows are swept by the first full sync.
ALTER TABLE favorites ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
