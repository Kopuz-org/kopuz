-- A remote source's cached rows predate artist credits, and its next sync rebuilds
-- them with ids from the pages it already fetches; local rows have no ids to gain.
DELETE FROM tracks WHERE source != 'local' AND source NOT LIKE 'local:%';
DELETE FROM albums WHERE source != 'local' AND source NOT LIKE 'local:%';
