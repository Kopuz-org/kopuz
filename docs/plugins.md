# Writing a Kopuz plugin

A plugin is a Lua file that Kopuz loads into a sandboxed interpreter inside its
own process. It provides a **music source**: a library, search, playlists, covers
and playable audio. Kopuz knows nothing about what sits behind it. The display
name, icon, capabilities, sign-in steps and stream URLs are all runtime data the
plugin supplies.

Nothing in Kopuz is specific to any provider. If your service can answer the
functions below, it can be a source.

The host hands you the parts that would otherwise be most of the work: an HTTP
client, JSON, hashes and base64, URL handling and a key-value store. What is left
is the mapping between your API's responses and the tables described here.

- [Installing](#installing)
- [The manifest](#the-manifest)
- [The entry chunk](#the-entry-chunk)
- [Capabilities](#capabilities)
- [The `kopuz` global](#the-kopuz-global)
- [Exported functions](#exported-functions)
- [Table shapes](#table-shapes)
- [Errors](#errors)
- [Item ids](#item-ids)
- [Cover art](#cover-art)
- [The sandbox](#the-sandbox)
- [Developing and debugging](#developing-and-debugging)
- [Checklist](#checklist)

`examples/plugins/example/` is a complete working plugin: a fake three-track
source with a sign-in wizard, favorites, a paginated playlist and playable
audio. It needs no account and no API key. Copy it into your plugins directory
and read it alongside this document.

## Installing

Kopuz scans `<config dir>/plugins/*/plugin.toml` at startup:

| Platform | Directory |
| --- | --- |
| Linux | `~/.config/kopuz/plugins/` |
| macOS | `~/Library/Application Support/com.temidaradev.kopuz/plugins/` |
| Windows | `%APPDATA%\temidaradev\kopuz\config\plugins\` |

One directory per plugin, holding at minimum `plugin.toml` and the entry chunk:

```
plugins/
  example/
    plugin.toml
    init.lua
    lib/
      catalog.lua
```

`KOPUZ_PLUGIN_PATH` adds further roots, separated the way `PATH` is, scanned
after the config directory. Packaged builds want this: under Nix, Flatpak or any
other store-based install the plugin sits on a read-only path with nowhere to
copy it from, so the package sets the variable instead.

```
KOPUZ_PLUGIN_PATH=/nix/store/…-kopuz-plugin-example/share/kopuz/plugins
```

Nothing is ever written next to the manifest, so that path may be read-only. A
plugin's own state lives in `<config dir>/plugin-data/<id>/`, which Kopuz creates
and exposes as `kopuz.data_dir`. Kopuz never reads or deletes it, not even when
the user removes the source.

An id found in more than one root resolves to the first, so a plugin dropped into
the config directory shadows a packaged build of the same id, which is what you
want while testing a local build.

A malformed manifest, a missing entry file, or an `api` this build does not speak
is logged and skipped, and never hides the other plugins. **Settings → Plugins**
lists what was found and has a **Rescan plugins** button.

## The manifest

```toml
id      = "example"                   # [a-z0-9_-]+, stable forever
name    = "Example"                   # sidebar badge and settings row
version = "1.0.0"
api     = 1                           # must equal the host's API_VERSION
entry   = "init.lua"                  # optional, defaults to init.lua
icon    = "fa-solid fa-puzzle-piece"  # optional Font Awesome class
accent  = "#8a8a8a"                   # optional CSS colour
```

`id` is load-bearing: it namespaces every track this plugin contributes to the
database. Changing it orphans the user's synced library. The charset is narrow
(`a-z`, `0-9`, `_`, `-`) because the id becomes a prefix on persisted refs.

`entry` must be a relative path that stays inside the plugin directory. An
absolute path or a `..` is rejected, so a manifest can only ever run code shipped
beside it.

`api` is refused rather than negotiated. A plugin targeting a generation this
build does not speak is skipped with a log line naming both numbers, because
half-speaking an older dialect is how plugin systems rot.

Capabilities are deliberately **not** in the manifest. They come from the
handshake, so they cannot claim something the loaded script does not implement.

## The entry chunk

The chunk returns one table. Kopuz calls `setup` on it once, then calls whatever
else it needs by name. Nothing else about the file matters: keep your state in
locals, and export only what the host calls.

```lua
local M = {}

function M.setup(ctx)
  return {
    auth_required = false,
    capabilities = { sync = true, favorites_sync = "instant" },
  }
end

function M.resolve_stream(item_id)
  return { url = "https://cdn.example.test/track/" .. item_id }
end

return M
```

`ctx` is a table carrying `api`, `host_version`, `locale`, `data_dir` and
`plugin_id`, all of which are on the `kopuz` global as well. Read them from
whichever is closer to hand.

`setup` runs before anything else, and its return value is the handshake:

| Field | Type | Meaning |
| --- | --- | --- |
| `capabilities` | table | See below. Omitted means a source that can only stream. |
| `auth_required` | bool | The sign-in wizard must run before this source can serve. |
| `name` | string | Overrides the manifest name in the UI. Omit unless it is computed. |
| `version` | string | Same, for the version. |
| `account` | string | Account label for the settings row, when signed in. |
| `web_url_template` | string | `https://…/{id}` for "open on the web". `{id}` is replaced with the bare item id. Omit when the source has no public pages. |

Raising from `setup`, or a chunk that will not load at all, marks the plugin
unusable until it is fixed and rescanned. It does not affect the other plugins.

## Capabilities

One table, every field optional. An omitted field means "not supported", which is
what lets a plugin written against an older Kopuz keep working.

| Field | Values | What it gates |
| --- | --- | --- |
| `sync` | bool | The sync task polls `fetch_library`. |
| `discover` | bool | The Discover page calls `discover_home`. |
| `radio` | bool | The radio action calls `start_radio`. |
| `playlists` | `"none"`, `"add_remove"`, `"reorder"` | Which playlist writes the UI offers. |
| `artist_view` | `"library"`, `"remote"` | Artist pages from the synced library, or from `fetch_artist`. |
| `albums` | `"standard"`, `"yt_music"` | Album page layout. |
| `favorites_sync` | `"instant"`, `"paginated"` | Whether favorites come from `fetch_favorites` or `fetch_favorites_page`. |
| `edit_tags`, `delete_from_disk`, `scan_folders`, `folders` | bool | Local-library features. Leave them off. |
| `downloads` | bool | Ignored, see below. |

Only claim what you implement. Declaring `playlists = "add_remove"` without
`add_to_playlist` puts a button in front of the user that fails when pressed,
which is worse than not offering it.

**Downloads are not supported for plugin sources.** The offline download path
does not go through `resolve_stream`, so there is no way to fetch a plugin's
bytes for it. The host forces the flag off rather than trusting it.

## The `kopuz` global

Always present, read-only, and the only global a plugin needs.

### Host facts

| Name | Value |
| --- | --- |
| `kopuz.version` | The app version, e.g. `"0.13.0"`. |
| `kopuz.api` | The API generation, matching the manifest's `api`. |
| `kopuz.plugin_id` | Your own id. Handy for log lines; you never need it for item ids. |
| `kopuz.data_dir` | Your private state directory, as a path string. |
| `kopuz.locale` | The UI's language tag, e.g. `"en"` or `"pt-PT"`, for strings you author. |

### `kopuz.fail(code, message)`

Raises a classified error. See [Errors](#errors) for what the host does with
each code. It does not return.

```lua
if response.status == 401 then
  kopuz.fail("auth", "the token expired")
end
```

### `kopuz.log`

`kopuz.log.trace|debug|info|warn|error(message)`. Re-emitted through the host's
`tracing` under target `plugin`, with your plugin id as a field.

### `kopuz.http`

`kopuz.http.request(opts)` plus the shorthands `get(url [, opts])`,
`post(url [, opts])` and `head(url [, opts])`. Calls suspend and resume; write
them straight-line, with no callbacks.

| Option | Type | Notes |
| --- | --- | --- |
| `url` | string | Required, unless it came from the shorthand's first argument. |
| `method` | string | Defaults to `GET`, or `POST` for `post`. |
| `headers` | table | `name = value`. |
| `query` | table | Appended as a query string. |
| `body` | string | Sent as-is. |
| `json` | any | Encoded, and sets `Content-Type: application/json`. |
| `form` | table | Encoded as `application/x-www-form-urlencoded`. |
| `timeout_ms` | number | Per-request, default 30000, capped at 120000. The whole call has its own deadline regardless. |
| `follow_redirects` | bool | Defaults to on, up to ten hops. Turn it off to read a `Location` header yourself. |

Only `http` and `https` URLs are accepted. At most one of `body`, `json` and
`form` may be set. `headers` is applied last, so a `content-type` there overrides
the one `json` or `form` would have used.

The response is a table:

| Field | Notes |
| --- | --- |
| `status` | Number. |
| `ok` | `status` is 2xx. |
| `headers` | Table with lowercased keys. |
| `body` | String. |
| `url` | The final URL after redirects. |
| `json()` | Decodes `body`, raising `invalid_input` if it is not JSON. |

A transport failure (DNS, TLS, connection, timeout) raises `connectivity`. A
non-2xx status does not raise: it comes back with `ok = false`, because only you
know whether a 404 is an error or an empty answer.

```lua
local function get(path, query)
  local response = kopuz.http.get(BASE .. path, {
    headers = { Authorization = "Bearer " .. token() },
    query = query,
  })
  if response.status == 401 then
    kopuz.fail("auth", "the session expired")
  end
  if not response.ok then
    kopuz.fail("connectivity", string.format("%s answered %d", path, response.status))
  end
  return response:json()
end
```

### `kopuz.json`

`encode(value)`, `decode(text)`, and `kopuz.json.null` for a JSON null, which
Lua cannot otherwise hold in a table.

### `kopuz.crypto`

`md5`, `sha1`, `sha256` of a string; `hmac_sha1(key, message)` and
`hmac_sha256(key, message)`; `base64_encode`, `base64_decode`,
`base64url_encode`, `base64url_decode`; `hex_encode`, `hex_decode`;
`random_bytes(n)`. Digests come back hex-encoded.

### `kopuz.url`

`encode(s)` and `decode(s)` for one component, `build_query(table)` and
`parse_query(string)` for a whole query string, and `join(base, relative)` for
resolving a relative URL against a base.

### `kopuz.store`

`get(key)`, `set(key, value)`, `delete(key)`, `keys()`, `clear()`. A JSON file at
`<data_dir>/store.json`, so values are strings, numbers, booleans, and tables of
those. This is where a plugin keeps its credentials.

Two things follow from it being JSON. An empty Lua table has no distinct JSON
form, so prefer a map (`{ [id] = true }`) over an array when a collection can be
empty. And the file is plain text on the user's disk: it is as private as their
config directory, and no more than that.

### The rest

| Call | Effect |
| --- | --- |
| `kopuz.notify.auth_changed(ok)` | Tells the host the sign-in state changed, so the settings row and the sync task pick it up without waiting for the next `validate`. |
| `kopuz.browser.open(url)` | Opens the user's browser. Prefer an `open_url` auth step during sign-in; this is for the cases outside one. |
| `kopuz.time.now_ms()` | Unix time in milliseconds. |
| `kopuz.time.sleep(ms)` | Suspends without blocking the host. Counts against the call deadline. |
| `kopuz.uuid()` | A random UUID string. |

### `require`

`require("lib.foo")` loads `<plugin dir>/lib/foo.lua`. Paths cannot escape the
plugin directory and there is no C module loader, so this is for splitting your
own file up, not for pulling in a rock. Each module is loaded once per plugin
state.

## Exported functions

Every function is optional except `setup`. **A missing function means
unsupported, and that is a normal thing to be**: the host either degrades to what
a source without the feature would have done, or never asks in the first place
because you did not claim the capability. Do not write a stub that returns an
empty success, because an empty library and an absent one lead the host to
different behaviour.

`resolve_stream` is the one you cannot skip in practice. Without it nothing
plays.

### Lifecycle

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `setup` | `ctx` | Handshake table | Required. |
| `unload` | none | nothing | Nothing runs before the state is dropped. |

`unload` is called at app shutdown and before a reload. Flush anything held only
in memory; nothing of yours runs afterwards.

### Sign-in

You author the whole wizard and Kopuz renders it with one generic popup, so it
never learns what you are asking for.

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `auth_begin` | none | prompt | The wizard reports `done` immediately. |
| `auth_submit` | `values` | prompt | Same. |
| `auth_cancel` | none | nothing | Nothing to tear down. |
| `validate` | none | `"valid"`, `"expired"`, `"unreachable"` | Treated as `"valid"`. |

The host calls `auth_begin`, renders what comes back, then calls `auth_submit`
with the collected `key = value` table, looping until `done` or `failed`. For an
`open_url` or `message` step the values table is empty, and the submit only means
the user pressed continue. `auth_cancel` means they closed the popup.

```lua
{ kind = "open_url", url = "https://…", message = "Sign in, then press continue." }
{ kind = "form", title = "Sign in", fields = {
    { key = "user", label = "Username" },
    { key = "password", label = "Password", secret = true },
  } }
{ kind = "message", text = "Enter 4821 on your other device." }
{ kind = "done" }
{ kind = "failed", message = "That code expired." }
```

`secret` renders a password input and keeps the value out of the logs. A `failed`
prompt closes the wizard with the message shown verbatim; to ask again, return
another `form`.

An auth step gets a five-minute deadline rather than the usual sixty seconds,
because a browser round trip legitimately takes that long.

`validate` answers `"unreachable"` when the backend cannot be reached, which
marks the source offline, and `"expired"` when the credentials are the problem,
which sends the user to the wizard. Anything unrecognised is read as
`"unreachable"`: a plugin that answers nonsense must not be reported as signed
in.

### Playback

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `resolve_stream` | `item_id` | stream table | Nothing plays. |

The host plays the URL with a plain buffered GET, so it must answer 2xx with real
audio bytes and an accurate `Content-Length`: scrubbing is driven off that
length. It must not look like a `.pls` or `.m3u` playlist, in the path or the
content type, or the host follows it as one instead of playing it. Range requests
are not required, and the container is detected by probing the bytes, so anything
the app can decode works.

Ask your backend for the URL on every call rather than caching it. Signed URLs
expire, and this function is called once per play.

### Library and search

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `fetch_library` | none | library table | The sync finds nothing. Gated on `sync`. |
| `search` | `query`, `limit` | search table | The host searches the synced library instead. |

`fetch_library` is a whole-library snapshot, and the sync task diffs it against
the database. A partial answer deletes the rest of the user's synced tracks, so
raise rather than return half a catalog. `limit` is a cap and not a target;
returning fewer results is normal.

### Favorites

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `fetch_favorites` | none | array of item ids | No favorites. Used when `favorites_sync = "instant"`. |
| `fetch_favorites_page` | `cursor` | track page | Same, for `"paginated"`. |
| `push_favorite` | `item_id`, `on` | nothing | The star fails. |

### Playlists

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `fetch_playlists` | none | array of playlist meta | No playlists. |
| `fetch_playlist_entries_page` | `playlist_id`, `cursor` | track page | The playlist reads as empty. |
| `add_to_playlist` | `playlist_id`, `item_ids` | array of ids that landed | Needed for `playlists = "add_remove"`. |
| `create_playlist` | `name`, `item_ids` | new `playlist_id` | Same. |
| `remove_from_playlist` | `playlist_id`, `item_id`, `playlist_item_id`, `position` | nothing | Same. |
| `reorder_playlist` | `playlist_id`, `ordered_ids`, `item_id`, `playlist_item_id`, `to` | nothing | Needed for `playlists = "reorder"`. |

`add_to_playlist` returns the ids that actually landed, which is how a backend
that silently drops a duplicate stays in step with the local mirror.

`playlist_item_id` is the handle for the *entry* rather than the track, from the
track you handed back on the page. It is what a removal names when a playlist
holds the same track twice. `ordered_ids` is the full new membership in order,
since the host does not track the old order and cannot send a `from` index.

### Albums

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `fetch_album` | `album_id` | album detail | Needed for remote album pages. |
| `fetch_album_tracks` | `album_id` | array of tracks | Same. |
| `fetch_album_by_ref` | `album_ref` | album detail or nil | Same. |
| `fetch_album_by_meta` | `title`, `artist` | album detail or nil | Same. |
| `resolve_album_id` | `title`, `artist` | `album_id` or nil | The lookup fails. |

A source whose album pages come from the synced library needs none of these.

### Artists

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `fetch_artist` | `artist_id` | artist page | Needed for `artist_view = "remote"`. |
| `resolve_artist_id` | `query` | `artist_id` or nil | The lookup fails. |
| `fetch_artist_images` | none | array of `{ name, image }` | No artist photos. |
| `fetch_artist_image` | `name` | URL or nil | Same, for one artist. |

`fetch_library` already carries `artist_images`, so a plugin that syncs usually
skips both of the image functions.

### Discover and radio

| Function | Arguments | Returns | If omitted |
| --- | --- | --- | --- |
| `discover_home` | none | discover table | Gated on `discover`. |
| `discover_continuation` | `token` | discover table | Needed for paging Discover. |
| `start_radio` | `seed_id` | array of tracks | Gated on `radio`. |

## Table shapes

Field names are exactly as written. An absent field is a default, which is how a
plugin built against an older generation keeps working.

```lua
-- Track. Only item_id and title are required.
{
  item_id = "t1",
  title = "Song",
  artist = "Artist",
  artists = { "Artist" },
  album = "Album",
  album_id = "a1",
  cover = "directurl:https://…/art.jpg",
  duration_secs = 210,
  khz = 44100,
  bitrate = 320,
  track_number = 3,
  disc_number = 1,
  playlist_item_id = nil,  -- only on tracks that came from a playlist page
}

-- Album
{ album_id = "a1", title = "Album", artist = "Artist", year = 2019,
  cover = "directurl:https://…", genre = "Rock" }

-- Album detail. play_ref is an opaque handle you accept back as
-- "play this whole album".
{ album = { … }, tracks = { … }, play_ref = nil }

-- Artist page
{ artist_id = "ar1", name = "Artist", subtitle = nil, description = nil,
  banner = nil, shuffle_ref = nil, shelves = { … } }

-- Shelf: one carousel, or a numbered song list when is_song_list is true.
-- more_ref is the token discover_continuation gets for a "see all".
{ title = "Top songs", strapline = nil, more_ref = nil, is_song_list = true,
  items = {
    { kind = "song", track = { … } },
    { kind = "album", album_id = "a1", title = "…", subtitle = "…", cover = nil },
    { kind = "artist", artist_id = "ar1", name = "…", image = nil },
    { kind = "playlist", playlist_id = "p1", title = "…", subtitle = "…", cover = nil },
    { kind = "category", id = "c1", title = "…", cover = nil },
  } }

-- Playlist meta
{ playlist_id = "p1", name = "Playlist", image = "directurl:https://…" }

-- Track page. next is opaque to the host and comes straight back on the next
-- call; nil ends the walk.
{ tracks = { … }, next = "cursor-2" }

-- Library snapshot
{ albums = { … }, tracks = { … },
  artist_images = { { name = "Artist", image = "https://…" } } }

-- Search result
{ tracks = { … }, albums = { … } }

-- Discover result
{ shelves = { … }, next = nil }

-- Stream. user_agent overrides the host's default (Kopuz/<version>) for
-- backends that are particular about who is asking.
{ url = "https://…", content_length = 5242880, duration_secs = 210,
  bitrate = 320, user_agent = nil }
```

## Errors

`kopuz.fail(code, message)` raises a classified error. Any other Lua error, from
an `error()` call or a genuine bug, is a backend failure carrying the method name
and the message.

| Code | Meaning | What the app does |
| --- | --- | --- |
| `auth` | Not signed in, or credentials expired | Prompts a re-sign-in. From `validate`, reads as `"expired"`. |
| `connectivity` | The backend was unreachable | Shows the source as offline. |
| `invalid_input` | The host asked for something malformed | Surfaces `message`. |
| `unsupported` | You do not implement this operation | Identical to leaving the function out. |
| anything else | A bug, or a plain `error()` | Surfaces `<method>: <message>`. |

Raising `unsupported` from a function that exists is for the case where support
depends on runtime state, such as an account tier that turns a feature off. When
it is a static fact about your plugin, leave the function out instead.

An ordinary call has sixty seconds; an auth step has five minutes. The deadline
covers waiting on the network and burning CPU alike, so a spin loop fails that
one call rather than wedging the app.

## Item ids

Ids you return are namespaced `"<plugin_id>/<your ref>"` before they are
persisted, and stripped again before they come back to you. You never see the
prefix, and you never write it.

Namespaced on the way out and stripped on the way in:

- a track's `item_id` and its `album_id`
- an album's `album_id`, including inside a shelf item
- the ids from `fetch_favorites` and `add_to_playlist`
- the id `resolve_album_id` returns, and the `album_ref` handed to
  `fetch_album_by_ref`
- the `seed_id` for `start_radio`

Opaque in both directions, passed through exactly as you wrote them: `artist_id`,
`playlist_id`, `playlist_item_id`, `play_ref`, `more_ref`, continuation tokens
and page cursors.

Two rules for the namespaced ones:

- **They must not contain `:`.** The app's playback-ref parser splits on it.
- They must be stable across runs. They are the primary key of the user's synced
  library, so an id that changes shape orphans everything under the old one.

## Cover art

Return covers as `directurl:https://…`, or as a bare `http://` or `https://`
URL. Both resolve with no server configured, which is why plugin covers need no
host-side code at all. The `urlhex_<hex>` form is accepted too, and exists for
refs that have to survive being split on `:`.

This applies to `cover` on tracks and albums, `image` on a playlist meta, and
`banner` on an artist page. Artist photos are the exception: `artist_images` and
`fetch_artist_image` hand back plain image URLs, not cover refs.

A plugin with no public image URL has nothing to fall back on here, since Lua
cannot serve bytes of its own. Leave `cover` out and the UI falls back the way it
does for any source without artwork.

## The sandbox

The Lua state gets the safe standard library and nothing else. Removed: `io`,
`package` and the C module loader, `load`, `dofile`, `loadfile`, and everything
in `os` except `time`, `clock`, `date` and `difftime`. `require` resolves only
inside the plugin's own directory.

Allocation is capped at 256 MiB per plugin, and every call carries a deadline
enforced from an instruction hook as well as by the async timeout, so a runaway
loop or a runaway table fails the call rather than the app.

**This is not a defence against a hostile plugin.** `kopuz.http` reaches the
network and `kopuz.store` reaches the disk, which is the entire point of both.
Installing a plugin is trusting whoever wrote it, the same as installing any
other extension.

## Developing and debugging

Log with `kopuz.log`. Output goes through the host's `tracing` under target
`plugin`, with your plugin id as a field:

```bash
KOPUZ_LOG="plugin=debug" kopuz   # just the plugins
KOPUZ_DEBUG=1 kopuz              # everything at debug
```

`RUST_LOG` works too; `KOPUZ_LOG` takes precedence.

The edit loop is: change the file, restart Kopuz. **Settings → Plugins → Rescan
plugins** re-reads the manifests, which is what picks up a plugin you just
installed or one whose manifest you fixed. It does not reload a plugin that is
already loaded: that state keeps serving until the app restarts.

A syntax error or a raise from `setup` shows up as a load failure in the log,
with the Lua chunk and line. A failed load is not cached, so the next call
retries it and a fixed file is picked up without a restart. A load that
*succeeded* is what sticks around.

Everything else is a per-call failure, logged with the method name and surfaced
in the UI according to its code.

## Checklist

1. Write `plugin.toml` with a stable `id` and the current `api`.
2. Return a table from your entry chunk, with a `setup` that reports the
   capabilities you actually implement.
3. Implement `resolve_stream`, and check that the URL answers a plain GET with an
   accurate `Content-Length`.
4. Implement the sign-in wizard if there is anything to sign into, and store the
   credential in `kopuz.store`.
5. Implement `validate`, so an expired session sends the user to the wizard
   rather than looking like an outage.
6. Add whatever else your backend supports. Leave the rest out.
7. Drop the directory into the plugins folder, press **Rescan plugins**, then add
   it as a source.
