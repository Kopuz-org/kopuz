# Writing a Kopuz plugin

A plugin is an ordinary executable that Kopuz runs as a child process and talks
to over stdio. It provides a **music source**: a library, search, playlists,
covers and playable audio. Kopuz knows nothing about what is behind it — the
display name, icon, capabilities, sign-in steps and stream URLs are all runtime
data supplied by the plugin.

Nothing in Kopuz is specific to any provider. If your service can answer the
methods below, it can be a source.

- [Installing](#installing)
- [The manifest](#the-manifest)
- [Transport and framing](#transport-and-framing)
- [Lifecycle](#lifecycle)
- [Errors](#errors)
- [Method reference](#method-reference)
- [Notifications](#notifications)
- [Audio](#audio)
- [Cover art](#cover-art)
- [Item ids](#item-ids)
- [Capabilities](#capabilities)
- [Checklist](#checklist)

## Installing

Kopuz scans `<config dir>/plugins/*/plugin.toml` at startup:

| Platform | Directory |
| --- | --- |
| Linux | `~/.config/kopuz/plugins/` |
| macOS | `~/Library/Application Support/com.temidaradev.kopuz/plugins/` |
| Windows | `%APPDATA%\temidaradev\kopuz\config\plugins\` |

One directory per plugin, holding at minimum `plugin.toml` and the executable:

```
plugins/
  example/
    plugin.toml
    kopuz-plugin-example
    data/            # created by Kopuz, owned by you
```

A malformed manifest, or one whose executable is missing, is logged and skipped
— it never hides the other plugins. **Settings → Plugins** lists what was found
and has a rescan button.

## The manifest

```toml
id        = "example"                     # [a-z0-9_-]+, stable forever
name      = "Example"                     # shown in the sidebar and settings
version   = "0.1.0"
protocol  = 1                             # must equal the host's PROTOCOL_VERSION
executable = "kopuz-plugin-example"       # relative to this directory, or absolute
args      = []                            # optional
icon      = "fa-solid fa-puzzle-piece"    # optional Font Awesome class
accent    = "#8a8a8a"                     # optional CSS colour
```

`id` is load-bearing: it namespaces every track this plugin contributes to the
database. Changing it orphans the user's synced library. It may not contain
`:` or `/`.

Capabilities are deliberately **not** in the manifest — they come from the
handshake, so they cannot drift from what the shipped binary can actually do.

## Transport and framing

JSON-RPC 2.0 over newline-delimited JSON:

- **stdin** — host → plugin. One compact JSON object per line.
- **stdout** — plugin → host. Same framing. **Nothing else may be written
  here**; a stray `print` corrupts the stream. Pin your logger to stderr.
- **stderr** — free-form. Kopuz re-emits each line through `tracing` at debug
  level with target `plugin`.

Lines are capped at 16 MiB; a longer one kills the child.

Requests carry an integer `id` and get exactly one response with that id.
Responses may arrive out of order — the host correlates on `id`, so answer slow
calls on their own task rather than blocking the read loop (`ping` has a 10 s
deadline).

```jsonc
// host → plugin
{"jsonrpc":"2.0","id":4,"method":"resolve_stream","params":{"item_id":"t1"}}
// plugin → host
{"jsonrpc":"2.0","id":4,"result":{"url":"http://127.0.0.1:51234/a/tok/t1","content_length":5242880}}
```

Notifications carry no `id` and are never answered.

## Lifecycle

1. **Discovery** — the manifest is read. No process is started.
2. **Spawn** — on first use, with:
   - working directory = the manifest directory
   - `KOPUZ_PLUGIN_DATA_DIR` = your private state directory (created for you;
     Kopuz never reads or deletes it, including when the user removes the
     source)
   - `KOPUZ_PLUGIN_PROTOCOL` = the protocol version
   - the host's own environment otherwise, so `RUST_LOG` and friends pass
     through
3. **Handshake** — the host sends `initialize` and you reply. A `protocol`
   mismatch is fatal: the child is killed and the user is told which version
   each side speaks.
4. **Health** — `ping` every 20 s with a 10 s deadline. Three consecutive
   failures, or an unexpected exit, restarts the child with backoff
   1/2/4/8/16 s capped at 30 s. More than five restarts in five minutes and the
   host gives up until the user switches back to the source. Requests in flight
   when a child dies resolve as `backend` errors — never a hang.
5. **Shutdown** — a `shutdown` notification, then stdin closes. Exit promptly;
   the host kills the process shortly after.

## Errors

Use the JSON-RPC error object with a `data.kind` discriminant:

```json
{"jsonrpc":"2.0","id":7,"error":{"code":-32000,"message":"not signed in","data":{"kind":"auth"}}}
```

| `kind` | Meaning | What the app does |
| --- | --- | --- |
| `unsupported` | You do not implement this method | Falls back to the built-in default (see below) |
| `connectivity` | Your backend was unreachable | Shows the source as offline |
| `auth` | Not signed in, or credentials expired | Prompts a re-sign-in |
| `invalid_input` | The host asked for something malformed | Surfaces `message` |
| `backend` | Anything else | Surfaces `message` |

Returning `unsupported` is always safe and is the right answer for anything you
cannot do — do **not** fake an empty success. For the optional methods the host
degrades to the same behaviour a built-in source without that feature would have
had (an empty library, an empty page, a local-corpus search).

## Method reference

Only these six are effectively required: `initialize`, `ping`, `validate`,
`auth_begin`, `auth_submit`, `resolve_stream`. Everything else may return
`unsupported`.

### `initialize`

```jsonc
// params
{"protocol":1,"host_version":"0.13.0","locale":"en","data_dir":"/…/plugins/example/data"}
// result
{
  "protocol": 1,
  "name": "Example",
  "version": "0.1.0",
  "capabilities": { /* see Capabilities */ },
  "auth_required": false,
  "data_base_url": "http://127.0.0.1:51234",
  "data_token": "9f2c…",
  "account": "user@example.test",          // optional
  "web_url_template": "https://example.test/track/{id}"  // optional
}
```

`data_base_url` and `data_token` describe your byte server (see [Audio](#audio)).
Kopuz never parses them; they exist so the host can redact the token from logs.

`web_url_template` backs the "open on the web" affordance. `{id}` is replaced
with the bare item id. Omit it if your source has no public pages.

### `ping`

`params: {}` → any result. Answer promptly.

### `validate`

`params: {}` → `"Valid"` | `"Expired"` | `"Unreachable"`.

### `auth_begin` / `auth_submit`

You author the whole sign-in wizard; Kopuz renders it with one generic popup.
`auth_begin` takes `{}`, `auth_submit` takes `{"values":{"<key>":"<value>"}}`.
Both return one prompt:

```jsonc
{"OpenUrl": {"url": "https://…", "message": "Sign in, then press Continue."}}
{"Form": {"title": "Sign in", "fields": [{"key":"user","label":"Username","secret":false},
                                          {"key":"pass","label":"Password","secret":true}]}}
{"Message": {"text": "Enter 4821 on your other device."}}
"Done"
{"Failed": {"message": "That code expired."}}
```

The host loops `auth_submit` until `Done` or `Failed`. A browser-OAuth plugin
emits `OpenUrl` then `Done`; a username/password plugin emits `Form` then
`Done`. `auth_cancel` (a notification) means the user closed the popup — tear
down any listener or poll you started.

### Catalog and library

| Method | Params | Result |
| --- | --- | --- |
| `fetch_library` | `{}` | `{albums:[Album], tracks:[Track], artist_images:[[name,url]]}` |
| `search` | `{query, limit}` | `{tracks:[Track], albums:[Album]}` |
| `fetch_album` | `{album_id}` | `AlbumDetail` |
| `fetch_album_tracks` | `{album_id}` | `[Track]` |
| `fetch_album_by_ref` | `{album_ref}` | `AlbumDetail` or `null` |
| `fetch_album_by_meta` | `{title, artist}` | `AlbumDetail` or `null` |
| `resolve_album_id` | `{title, artist}` | `"<album_id>"` or `null` |
| `fetch_artist` | `{artist_id}` | `ArtistPage` |
| `resolve_artist_id` | `{query}` | `"<artist_id>"` or `null` |
| `fetch_artist_images` | `{}` | `[[name, url]]` |
| `fetch_artist_image` | `{name}` | `"<url>"` or `null` |
| `discover_home` | `{}` | `{shelves:[Shelf], next}` |
| `discover_continuation` | `{token}` | `{shelves:[Shelf], next}` |
| `start_radio` | `{seed_id}` | `[Track]` |

### Favorites and playlists

| Method | Params | Result |
| --- | --- | --- |
| `fetch_favorites` | `{}` | `["<item_id>"]` |
| `fetch_favorites_page` | `{cursor}` | `{items:[Track], next}` |
| `push_favorite` | `{item_id, on}` | `null` |
| `fetch_playlists` | `{}` | `[{playlist_id, name, image}]` |
| `fetch_playlist_entries_page` | `{playlist_id, cursor}` | `{items:[Track], next}` |
| `add_to_playlist` | `{playlist_id, item_ids}` | `["<item_id>"]` (what landed) |
| `create_playlist` | `{name, item_ids}` | `"<playlist_id>"` |
| `remove_from_playlist` | `{playlist_id, item_id, playlist_item_id, position}` | `null` |
| `reorder_playlist` | `{playlist_id, ordered_ids, item_id, playlist_item_id, to}` | `null` |

`favorites_sync: "Instant"` uses `fetch_favorites`; `"Paginated"` uses
`fetch_favorites_page`. Cursors are opaque — whatever you return comes straight
back on the next call, and `null` ends the walk.

### Playback

`resolve_stream {item_id}` → `{url, content_length?, duration_secs?, bitrate?}`.

### Data shapes

```jsonc
// Track — only item_id and title are required
{"item_id":"t1","title":"Song","artist":"Artist","artists":["Artist"],
 "album":"Album","album_id":"a1","cover":"directurl:https://…/art.jpg",
 "duration_secs":210,"khz":44100,"bitrate":320,
 "track_number":3,"disc_number":1,"playlist_item_id":null}

// Album
{"album_id":"a1","title":"Album","artist":"Artist","year":2019,
 "cover":"directurl:https://…","genre":"Rock"}

// AlbumDetail
{"album":{…},"tracks":[…],"play_ref":null}

// ArtistPage
{"artist_id":"ar1","name":"Artist","subtitle":null,"description":null,
 "banner":null,"shuffle_ref":null,"shelves":[…]}

// Shelf
{"title":"Top songs","strapline":null,"more_ref":null,"is_song_list":true,
 "items":[{"Song":{…}}, {"Album":{"album_id":"a1","title":"…","subtitle":"…","cover":null}},
          {"Artist":{"artist_id":"ar1","name":"…","image":null}},
          {"Playlist":{"playlist_id":"p1","title":"…","subtitle":"…","cover":null}},
          {"Category":{"id":"c1","title":"…","cover":null}}]}
```

## Notifications

Plugin → host, no `id`, no reply:

```jsonc
{"jsonrpc":"2.0","method":"log","params":{"level":"info","message":"…","target":"module"}}
{"jsonrpc":"2.0","method":"auth_changed","params":{"authenticated":true}}
{"jsonrpc":"2.0","method":"library_changed","params":{}}
```

`level` is one of `trace`, `debug`, `info`, `warn`, `error`. Host → plugin:
`shutdown` and `auth_cancel`.

## Audio

Kopuz's player consumes a **URL**. There is no byte-pipe into the decoder, so a
plugin that holds its own bytes serves them itself:

1. Bind an ephemeral loopback port (`127.0.0.1:0`) at startup.
2. Mint a random per-process token.
3. Report both as `data_base_url` / `data_token` in the handshake.
4. Return `http://127.0.0.1:<port>/a/<token>/<item_id>` from `resolve_stream`.

The token goes in the **path** because the host's reader sends no custom
headers. Compare it in constant time and answer a mismatch with a plain 404.

Your endpoint must:

- answer a plain `GET` with a 2xx and real audio bytes;
- send an accurate `Content-Length` — the host waits for it before allowing
  scrubbing, so without it seeking degrades;
- **not** look like a `.pls` or `.m3u` playlist, in either the path or the
  content type, or the host will try to follow it as one.

Range requests are not required. Format is detected by probing the bytes, so any
container the app can decode works.

If your service already exposes a public stream URL, return that directly and
skip the byte server entirely.

**Downloads are not supported for plugin sources in this release.** The offline
download path does not go through `resolve_stream`, so declaring
`downloads: true` would be a lie — the host forces it off.

## Cover art

Return covers as `directurl:https://…` (or the equivalent `urlhex_<hex>` form).
These resolve with no server configured, which is why plugin covers need no
host-side code at all. A plugin with no public image URL can serve images off
its own data port and return that URL — still with the `directurl:` prefix.

## Item ids

Ids you return are namespaced `"<plugin_id>/<your id>"` before they are
persisted, and stripped again before they are sent back to you. You never see
the prefix. Two consequences:

- **Your ids must not contain `:`.** The app's playback-ref parser splits on it.
- Ids must be stable across runs; they are the primary key of the user's synced
  library.

## Capabilities

Sent in the handshake. Omitted fields default to "not supported", so a plugin
built against an older Kopuz keeps working.

| Field | Type | Meaning |
| --- | --- | --- |
| `sync` | bool | `fetch_library` is worth polling |
| `discover` | bool | `discover_home` works |
| `radio` | bool | `start_radio` works |
| `downloads` | bool | Ignored for plugins in this release |
| `edit_tags`, `delete_from_disk`, `scan_folders`, `folders` | bool | Local-library features; leave false |
| `playlists` | `"None"` \| `"AddRemove"` \| `"Reorder"` | Playlist write support |
| `artist_view` | `"Library"` \| `"Remote"` | Artist pages from the synced library, or from `fetch_artist` |
| `albums` | `"Standard"` \| `"YtMusic"` | Album page layout |
| `favorites_sync` | `"Instant"` \| `"Paginated"` | Which favorites method the host calls |

## Checklist

1. Write your protocol loop: read stdin lines, dispatch, write one response per
   request to stdout. Log to stderr only.
2. Answer `initialize` with your real capabilities.
3. Answer `ping` immediately, on its own task.
4. Implement `auth_begin`/`auth_submit`, even if they just return `"Done"`.
5. Implement `resolve_stream` and, if you serve your own bytes, the loopback
   endpoint with an accurate `Content-Length`.
6. Add whatever else you can; return `unsupported` for the rest.
7. Drop the binary plus `plugin.toml` into the plugins directory, open
   **Settings → Plugins → Rescan**, then add it as a source.

`crates/plugin-example/` is a complete working plugin in about 300 lines — it
indexes a folder of audio files and serves them. Start from it.
