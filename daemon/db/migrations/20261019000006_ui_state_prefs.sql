-- UI state is each frontend's own preference now, seeded identically for both from what the app kept; a pref already stored wins.
INSERT OR IGNORE INTO frontend_prefs (frontend, key, value)
SELECT frontend.name, pref.key, pref.value
  FROM (SELECT 'kopuz' AS name UNION ALL SELECT 'gpui') AS frontend
 CROSS JOIN (
    SELECT 'source_explicitly_set' AS key, iif(source_explicitly_set, 'true', 'false') AS value FROM app_state WHERE id = 1
    UNION ALL SELECT 'fullscreen_tabs_collapsed', iif(fullscreen_tabs_collapsed, 'true', 'false') FROM app_state WHERE id = 1
    UNION ALL SELECT 'sort_order', json_quote(sort_order) FROM app_state WHERE id = 1
    UNION ALL SELECT 'album_view_mode', json_quote(album_view_mode) FROM app_state WHERE id = 1
    UNION ALL SELECT 'artist_album_view_mode', json_quote(artist_album_view_mode) FROM app_state WHERE id = 1
    UNION ALL SELECT 'artists_view_mode', json_quote(artists_view_mode) FROM app_state WHERE id = 1
    UNION ALL SELECT 'artist_view_order', json_quote(artist_view_order) FROM app_state WHERE id = 1
    UNION ALL SELECT 'listen_now_style', json_quote(listen_now_style) FROM app_state WHERE id = 1
    UNION ALL SELECT 'hero_height', CAST(hero_height AS TEXT) FROM app_state WHERE id = 1
    UNION ALL SELECT v.key, (
        SELECT json_group_array(json_object('field', s.field, 'direction', s.direction) ORDER BY s.position)
          FROM view_sorts s WHERE s.view = v.view)
      FROM (SELECT 'albums' AS view, 'album_sort' AS key
            UNION ALL SELECT 'library', 'library_sort'
            UNION ALL SELECT 'artist_albums', 'artist_album_sort'
            UNION ALL SELECT 'artists', 'artist_sort') AS v
     WHERE EXISTS (SELECT 1 FROM app_state WHERE id = 1)
    UNION ALL SELECT 'sidebar_order', json_group_array(item ORDER BY position)
      FROM sidebar_items HAVING count(*) > 0
    UNION ALL SELECT 'home_sections',
        json_group_array(json_object('key', key, 'enabled', json(iif(enabled, 'true', 'false'))) ORDER BY position)
      FROM home_sections HAVING count(*) > 0
 ) AS pref;

ALTER TABLE app_state DROP COLUMN source_explicitly_set;
ALTER TABLE app_state DROP COLUMN fullscreen_tabs_collapsed;
ALTER TABLE app_state DROP COLUMN sort_order;
ALTER TABLE app_state DROP COLUMN album_view_mode;
ALTER TABLE app_state DROP COLUMN artist_album_view_mode;
ALTER TABLE app_state DROP COLUMN artists_view_mode;
ALTER TABLE app_state DROP COLUMN artist_view_order;
ALTER TABLE app_state DROP COLUMN listen_now_style;
ALTER TABLE app_state DROP COLUMN hero_height;

DROP TABLE view_sorts;
DROP TABLE sidebar_items;
DROP TABLE home_sections;
