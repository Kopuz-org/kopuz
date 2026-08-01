-- Which installed plugin backs a `Plugin` server row. NULL for every built-in
-- service. Identity for plugin sources lives here rather than in `url`, which
-- is empty for them.
ALTER TABLE servers ADD COLUMN plugin_id TEXT;
