-- A photo belongs to the artist row it was filed for, by the same (source, key) every other read opens it by.
CREATE TABLE artist_images_by_key (
    source      TEXT NOT NULL,
    artist_key  TEXT NOT NULL,
    kind        TEXT NOT NULL,
    image_ref   TEXT NOT NULL,
    PRIMARY KEY (source, artist_key, kind)
);

-- A linked artist's legacy key spelled its source and id; matching it whole needs no parsing of either.
INSERT OR IGNORE INTO artist_images_by_key (source, artist_key, kind, image_ref)
SELECT a.source, a.key, i.kind, i.image_ref
  FROM artist_images i
  JOIN artists a ON a.source_artist_id IS NOT NULL
                AND i.artist_norm = 'id:' || a.source || ':' || a.source_artist_id;

-- A bare folded name was shared by every unlinked artist called that, so each of them keeps a copy.
-- A linked artist keeps only a custom one, which the old read fell back to and nothing can fetch again.
INSERT OR IGNORE INTO artist_images_by_key (source, artist_key, kind, image_ref)
SELECT a.source, a.key, i.kind, i.image_ref
  FROM artist_images i
  JOIN artists a ON a.name_key = i.artist_norm
                AND (a.source_artist_id IS NULL OR i.kind = 'custom');

DROP TABLE artist_images;
ALTER TABLE artist_images_by_key RENAME TO artist_images;

-- Remembered misses were filed under the old keys.
DELETE FROM kv WHERE kind = 'artist_photo_miss';
