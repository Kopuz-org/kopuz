-- What a source's listing says of a track beyond its tags: whether it is marked
-- explicit, and its play count as the source words it ("6.9M plays"). A source
-- that reports neither leaves the defaults.
ALTER TABLE tracks ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tracks ADD COLUMN plays TEXT;

-- A restored queue replays rows the library may never have stored, so it keeps
-- them too.
ALTER TABLE queue_tracks ADD COLUMN explicit INTEGER NOT NULL DEFAULT 0;
ALTER TABLE queue_tracks ADD COLUMN plays TEXT;
