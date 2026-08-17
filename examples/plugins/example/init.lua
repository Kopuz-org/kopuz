--- Kopuz example source plugin.
---
--- A source with no service behind it: a three-track catalog in
--- `lib/catalog.lua`, a sign-in that accepts any password, and stream URLs
--- pointing at public-domain recordings. It exists so the author-facing contract
--- can be read end to end, and so the host has something to be exercised against
--- that never fails for a network reason.
---
--- To try it: copy this directory into `<config dir>/plugins/`, press "Rescan
--- plugins" in Settings, then add Example as a source and sign in with any
--- username and password. `docs/plugins.md` is the reference.
---
--- What this deliberately does not show is an HTTP call, because a fake backend
--- has nothing to call. `kopuz.http` is documented with examples in
--- `docs/plugins.md`; the tables the functions below return are what a real
--- plugin builds out of a response body.

local catalog = require("lib.catalog")

--- Where the fake credential lands. `kopuz.store` is per-plugin, JSON-backed,
--- and lives outside the plugin directory, which may be read-only.
local SESSION_KEY = "session"

--- Favorites share that store, as a set rather than an array: an empty Lua table
--- has no unambiguous JSON form, while a map round-trips as an object whether or
--- not it has entries.
local FAVORITES_KEY = "favorites"

--- Which wizard step the next `auth_submit` answers. In memory, not in the
--- store: it means nothing once the popup closes.
local step = nil

local M = {}

local function trim(text)
  return (text or ""):match("^%s*(.-)%s*$")
end

local function session()
  return kopuz.store.get(SESSION_KEY)
end

--- Every read below needs a session. `kopuz.fail("auth", …)` is what sends the
--- user back to the wizard; a plain `error()` would surface as a backend failure
--- and leave them with nothing to do about it.
local function require_session()
  local current = session()
  if current == nil then
    kopuz.fail("auth", "not signed in")
  end
  return current
end

--- The handshake, called once when the host loads this chunk.
---
--- `name` and `version` are omitted on purpose: `plugin.toml` carries them
--- already, and a second copy is a copy that drifts. Capabilities cannot live in
--- the manifest for the mirror-image reason, since they have to describe the
--- functions in this file.
---
--- `ctx` repeats the host facts that are also on the `kopuz` global, so this
--- reads them off the global and ignores the argument.
function M.setup(_ctx)
  kopuz.log.info(string.format("example plugin loaded (api %d, locale %s)", kopuz.api, kopuz.locale))
  local current = session()
  return {
    auth_required = current == nil,
    account = current and current.user or nil,
    capabilities = {
      -- `fetch_library` is worth polling, so the sync task will call it.
      sync = true,
      -- Playlists are listed but read-only. Claiming "add_remove" here would
      -- put an add button in front of the user that this file cannot honour.
      playlists = "none",
      -- Artist pages are built from the synced library, so no `fetch_artist`.
      artist_view = "library",
      albums = "standard",
      -- `fetch_favorites` answers with the whole set at once, so the paged form
      -- is never called.
      favorites_sync = "instant",
      -- Everything else is left out. A missing capability is "not supported",
      -- which is why a plugin written against an older host keeps working.
    },
  }
end

--- Step one of the wizard. The host renders whichever prompt comes back, posts
--- what the user typed to `auth_submit`, and repeats until `done` or `failed`.
function M.auth_begin()
  step = "credentials"
  return {
    kind = "form",
    title = "Sign in to Example",
    fields = {
      { key = "user", label = "Username" },
      -- `secret` renders a password input and keeps the value out of the logs.
      { key = "password", label = "Password", secret = true },
    },
  }
end

--- Step two and three. `values` is the `key = value` table the form collected,
--- and it is empty for a `message` or `open_url` step, where submitting only
--- means the user pressed continue.
function M.auth_submit(values)
  if step == "credentials" then
    local user = trim(values.user)
    if user == "" then
      -- `failed` closes the wizard with this message shown verbatim. Returning
      -- a fresh `form` instead would ask again.
      return { kind = "failed", message = "Enter a username." }
    end
    -- Any password is accepted because there is nothing to check it against. A
    -- real plugin exchanges these for a token here and stores that instead.
    kopuz.store.set(SESSION_KEY, {
      user = user,
      token = kopuz.crypto.sha256(user .. ":" .. (values.password or "")),
      signed_in_at_ms = kopuz.time.now_ms(),
    })
    step = "confirm"
    return {
      kind = "message",
      text = string.format("Signed in as %s. Nothing was verified: there is no server here.", user),
    }
  end

  if step == "confirm" then
    step = nil
    -- Tells the host the auth state changed, so the settings row and the sync
    -- task pick the session up without waiting for the next `validate`.
    kopuz.notify.auth_changed(true)
    return { kind = "done" }
  end

  -- A submit with no wizard in flight. Refusing it would trap the user in a
  -- popup with no way forward.
  return { kind = "done" }
end

--- The user closed the popup. Whatever the wizard started (a poll, a pending
--- device code) is torn down here.
function M.auth_cancel()
  step = nil
end

--- Asked on startup and whenever the source is switched to. `expired` sends the
--- user to the wizard; `unreachable` marks the source offline without implying
--- the credentials are wrong, and is the answer when the backend cannot be
--- reached at all.
function M.validate()
  if session() == nil then
    return "expired"
  end
  return "valid"
end

--- The whole library in one snapshot. The sync task diffs this against the
--- database, so returning a partial catalog deletes the rest of it.
function M.fetch_library()
  require_session()
  return {
    albums = catalog.albums,
    tracks = catalog.tracks,
    artist_images = catalog.artist_images,
  }
end

--- Catalog search. `limit` is a cap and not a target: fewer results is normal,
--- and a plugin that has no search at all leaves this function out, which makes
--- the host search the synced library instead.
function M.search(query, limit)
  require_session()
  local needle = trim(query):lower()
  if needle == "" then
    -- `invalid_input` surfaces the message as it is written. The host already
    -- short-circuits an empty query, so this only fires if that ever changes.
    kopuz.fail("invalid_input", "search needs a query")
  end
  return {
    tracks = catalog.matching(needle, limit),
    albums = catalog.albums_matching(needle),
  }
end

--- Bare refs of everything the user favorited, in a stable order.
function M.fetch_favorites()
  require_session()
  local ids = {}
  for item_id in pairs(kopuz.store.get(FAVORITES_KEY) or {}) do
    ids[#ids + 1] = item_id
  end
  table.sort(ids)
  return ids
end

function M.push_favorite(item_id, on)
  require_session()
  if catalog.by_id[item_id] == nil then
    kopuz.fail("invalid_input", "no track " .. tostring(item_id))
  end
  local set = kopuz.store.get(FAVORITES_KEY) or {}
  -- Assigning nil removes the key, so an unfavorited track leaves no `false`
  -- behind for `fetch_favorites` to filter out.
  set[item_id] = on and true or nil
  kopuz.store.set(FAVORITES_KEY, set)
end

function M.fetch_playlists()
  require_session()
  return catalog.playlists
end

--- One page of a playlist. `cursor` is nil on the first call and then whatever
--- came back as `next`, which the host never looks inside. `next = nil` ends the
--- walk.
---
--- One track per page is absurd for a playlist of three and is the point: the
--- host's paging loop gets exercised by the example rather than only in
--- production.
function M.fetch_playlist_entries_page(playlist_id, cursor)
  require_session()
  local entries = catalog.playlist_entries(playlist_id)
  if entries == nil then
    kopuz.fail("invalid_input", "no playlist " .. tostring(playlist_id))
  end
  local index = tonumber(cursor) or 1
  local track = entries[index]
  if track == nil then
    return { tracks = {} }
  end
  return {
    tracks = { track },
    next = entries[index + 1] and tostring(index + 1) or nil,
  }
end

--- The URL the host plays. It fetches it with a plain buffered GET, so the URL
--- has to answer 2xx with audio bytes and an accurate `Content-Length`, and must
--- not look like a `.pls`/`.m3u` playlist or it is followed as one instead of
--- played.
---
--- A real plugin returns whatever URL its backend hands out, expiry and all,
--- asked for freshly on every play. The example points at fixed files because
--- Lua here has no socket library, so it cannot serve bytes of its own.
function M.resolve_stream(item_id)
  require_session()
  local stream = catalog.streams[item_id]
  if stream == nil then
    kopuz.fail("invalid_input", "no track " .. tostring(item_id))
  end
  return {
    url = stream.url,
    content_length = stream.content_length,
    duration_secs = stream.duration_secs,
    -- Wikimedia refuses a request that carries no User-Agent. The host sends
    -- `Kopuz/<version>` unless this field overrides it, which is what the field
    -- is for: backends that are particular about who is asking.
    user_agent = "kopuz-example-plugin/1.0 (+https://github.com/temidaradev/kopuz)",
  }
end

--- Called before the Lua state is dropped: app shutdown, or a reload. Anything
--- held only in memory is flushed here, because nothing runs afterwards.
function M.unload()
  step = nil
  kopuz.log.debug("example plugin unloaded")
end

-- Left out on purpose: `discover_home`, `discover_continuation`, `start_radio`,
-- `fetch_artist`, the `fetch_album*` family, `fetch_favorites_page` and the
-- playlist writes. Leaving a function off this table is how a plugin declines an
-- operation. The host reads the absence as unsupported and degrades to what a
-- source without that feature would have done, so a stub that fails on purpose
-- buys nothing.

return M
