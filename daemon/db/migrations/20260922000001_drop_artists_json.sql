-- The names are credit rows now; the step between this and the last migration filled them from here.
ALTER TABLE tracks DROP COLUMN artists_json;
