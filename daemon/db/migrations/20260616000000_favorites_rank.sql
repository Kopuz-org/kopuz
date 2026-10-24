-- Favorites gain an explicit display order (issue #347 follow-up). Lower rank =
-- higher in the list. A remote pull stores the service's order (rank = index,
-- newest first); a fresh local like gets a rank below the current minimum so it
-- surfaces at the top, matching how the source (e.g. YT Music) orders likes.
-- Existing rows default to 0 — ties fall back to rowid (insertion) order, the
-- prior behavior, until the next pull assigns real ranks.
ALTER TABLE favorites ADD COLUMN rank INTEGER NOT NULL DEFAULT 0;
