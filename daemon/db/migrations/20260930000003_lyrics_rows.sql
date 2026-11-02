-- A track's lyrics as last fetched, or a miss that expires, keyed by the lookup's cache key.
CREATE TABLE lyrics (
    cache_key  TEXT NOT NULL PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('synced', 'plain', 'none')),
    plain_text TEXT CHECK ((kind = 'plain') = (plain_text IS NOT NULL)),
    fetched_at INTEGER NOT NULL
);

CREATE TABLE lyric_lines (
    cache_key     TEXT NOT NULL REFERENCES lyrics(cache_key) ON DELETE CASCADE,
    position      INTEGER NOT NULL,
    start_time    REAL NOT NULL,
    end_time      REAL,
    text          TEXT NOT NULL,
    parent_line   INTEGER,
    background    INTEGER NOT NULL,
    opposite_turn INTEGER NOT NULL,
    PRIMARY KEY (cache_key, position)
);

-- Word or syllable timing within a line, where the provider has it.
CREATE TABLE lyric_chunks (
    cache_key  TEXT NOT NULL,
    line       INTEGER NOT NULL,
    position   INTEGER NOT NULL,
    start_time REAL NOT NULL,
    text       TEXT NOT NULL,
    PRIMARY KEY (cache_key, line, position),
    FOREIGN KEY (cache_key, line) REFERENCES lyric_lines(cache_key, position) ON DELETE CASCADE
);

INSERT INTO lyrics (cache_key, kind, plain_text, fetched_at)
SELECT name,
       CASE json_extract(value, '$.kind') WHEN 'synced2' THEN 'synced' ELSE json_extract(value, '$.kind') END,
       CASE json_extract(value, '$.kind') WHEN 'plain' THEN json_extract(value, '$.text') END,
       COALESCE(json_extract(value, '$.ts'), updated_at)
  FROM kv
 WHERE kind = 'lyrics' AND json_valid(value)
   AND (json_extract(value, '$.kind') IN ('synced2', 'none')
        OR (json_extract(value, '$.kind') = 'plain' AND json_type(value, '$.text') = 'text'));

INSERT INTO lyric_lines (cache_key, position, start_time, end_time, text, parent_line, background, opposite_turn)
SELECT k.name, l.key, json_extract(l.value, '$.start_time'), json_extract(l.value, '$.end_time'),
       json_extract(l.value, '$.text'), json_extract(l.value, '$.parent_line_index'),
       COALESCE(json_extract(l.value, '$.background'), 0), COALESCE(json_extract(l.value, '$.opposite_turn'), 0)
  FROM kv k, json_each(k.value, '$.lines') l
 WHERE k.kind = 'lyrics' AND k.name IN (SELECT cache_key FROM lyrics WHERE kind = 'synced')
   AND json_type(l.value, '$.start_time') IN ('integer', 'real') AND json_type(l.value, '$.text') = 'text';

INSERT INTO lyric_chunks (cache_key, line, position, start_time, text)
SELECT k.name, l.key, c.key, json_extract(c.value, '$.start_time'), json_extract(c.value, '$.text')
  FROM kv k, json_each(k.value, '$.lines') l, json_each(l.value, '$.chunks') c
 WHERE k.kind = 'lyrics'
   AND EXISTS (SELECT 1 FROM lyric_lines ll WHERE ll.cache_key = k.name AND ll.position = l.key)
   AND json_type(c.value, '$.start_time') IN ('integer', 'real') AND json_type(c.value, '$.text') = 'text';

DELETE FROM kv WHERE kind = 'lyrics';
