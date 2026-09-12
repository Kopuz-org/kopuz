# The Kopuz daemon API

Kopuz's playback core is a daemon. Every feature the built-in GUI has goes
through the gRPC surface described here. The contract is one file:
`crates/proto/proto/kopuz.proto` (package `kopuz.v1`).

This is a local IPC channel between the daemon and a frontend on the same
machine. It is never exposed to a network, and it is not a stable public
API -- the schema changes with the app, in the same commit.

## Running the daemon

Two deployment shapes serve the same API:

- **Headless**: run `kopuzd`. It owns the audio engine, the SQLite library,
  the configured sources and their credentials, scan/sync jobs, downloads,
  scrobbling, and OS media integration (MPRIS/SMTC/Now Playing). No window,
  no webview. Build it with `cargo build --release -p kopuz-daemon --features
  kopuzd --bin kopuzd`.
- **Embedded**: the desktop app runs that same core in its own process and
  serves the identical API from it. This exists because SQLite is
  single-writer: `kopuzd` and the app must never run against one library at
  once, so a frontend that wants to attach while the app is open attaches to
  the app itself.

`kopuzd` flags: `--socket <path>`, `--db-path <file>`.

Exclusive ownership is enforced with a lock file beside the database, so a
second process pointed at the same library fails to start with a readable
error rather than becoming a second writer.
## Connecting

The daemon listens on a Unix domain socket, created `0600`:

- Linux: `$XDG_RUNTIME_DIR/kopuz/kopuzd.sock`
- macOS: the user cache dir, `~/Library/Caches/kopuz/kopuzd.sock` (the
  exact path is logged at startup)

The path is the whole rendezvous -- there is no discovery file, no port and
no token. **There is no authentication.** The socket's file mode is the
access control: the kernel admits your own processes and refuses everyone
else, which is the same boundary a token over loopback was reconstructing
in userspace, minus the secret.

A leftover socket from a crashed daemon has no listener behind it, so a
failed connect is what marks it stale; `kopuzd` clears it and takes the
path. A socket that *is* being served makes a second `kopuzd` exit with
`AddrInUse` rather than stealing the channel.

Server reflection (v1 and v1alpha) is registered, so `grpcurl` works out of
the box:

```sh
SOCK=$XDG_RUNTIME_DIR/kopuz/kopuzd.sock
grpcurl -unix -plaintext $SOCK list kopuz.v1.Kopuz
grpcurl -unix -plaintext $SOCK kopuz.v1.Kopuz/GetPlayerState
```

## Errors

RPC failures are gRPC statuses, and the status code is the whole story —
localize by it, never by the message. The mapping is one-to-one, so
nothing rides alongside it in metadata:

| gRPC code | `ErrorCode` | meaning |
|---|---|---|
| INVALID_ARGUMENT | `invalid_input` | malformed request or out-of-range position |
| FAILED_PRECONDITION | `source_auth_expired` | the media server needs a re-login |
| NOT_FOUND | `not_found` | unknown key/id |
| ALREADY_EXISTS | `conflict` | a single-flight job of that kind is already running |
| UNIMPLEMENTED | `unsupported` | this daemon runs without that service |
| UNAVAILABLE | `source_unreachable` | the media server did not answer |
| UNAVAILABLE | `daemon_gone` | *raised locally* — nothing is listening on the socket |
| INTERNAL | `internal` | daemon-side failure |

A failed mutation fails its own RPC with that status, so ordinary gRPC
error handling applies. Those last two share a status code because gRPC has one for "unreachable"
and both are: the daemon distinguishes them by whether the status carries a
transport cause, which only one tonic raised itself does. `daemon_gone` is
never sent -- the daemon cannot report its own absence.

The `ErrorCode` enum in the schema is for failures
reported *inside* a message, where there is no status to carry them, such
as `JobStatus.error`. Treat an unrecognized status code as `internal`.

## Events and playback

Playback commands are ordinary unary RPCs: `Play`, `Pause`, `Toggle`,
`Next`, `Previous`, `Stop`, `Seek {position_ms}`, `SetVolume {0..1}`, and
`SetMode {shuffle?, loop?}`. Each returns `MutationResult {rev}`, where
`rev` names the state revision containing the command's effect, so you can
wait for your mirror to catch up before trusting it.

`Kopuz/Subscribe` is the event stream, one server-streaming RPC per client.
It carries events from the moment you subscribe -- there is no resume
cursor and no replay log. Both processes live and die together, so a stream
that ends means the daemon is gone, not that a connection blipped; the
answer is to reattach and refetch, which is what a cursor would have made
you do anyway.

- **Attach order.** Subscribe first, then fetch `GetPlayerState` and your
  queue window. Events emitted while the snapshot is in flight are already
  buffered for you, so nothing is missed.
- **`resync`** means your mirror is untrustworthy: either you fell behind
  the buffer, or the daemon restarted under you. Refetch the snapshots.
- **Events** arrive in an `EventEnvelope`. The kinds mirror the state
  machine: `player_state` (full snapshot on every transition), `position`
  (new anchor after seek/pause), `buffered`, `queue_changed`,
  `library_invalidated {table}`, `job_progress`,
  `job_finished`, `config_changed`, `source_status`, `notice`, `resync`.

Two rules make a frontend feel native:

- **Position is an anchor, not a ticker.** `PlayerState.position` is
  `{ms, at_ms, playing}` and `now_ms` is the daemon clock at send time.
  Compute a clock offset once and interpolate locally while `playing` is
  true. The daemon does not stream per-second ticks.
- **`intent` vs `phase`.** `phase` is engine truth; `intent` is what the
  daemon is trying to do. Render optimistic UI from `intent` (show the
  pause glyph while a track is still loading), exactly like the built-in
  GUI. While `fading` is present, keep displaying `fading.track` and drive
  the seek bar from `fading.position_ms`.

## Unary RPCs

Playback and queue:

| rpc | request | returns |
|---|---|---|
| `GetStatus` | | `{version, uptime_secs}` |
| `GetPlayerState` | | `PlayerState` snapshot |
| `GetQueue` | `Page {offset, limit}` | play-order window `{rev, total, items: [{index, track}]}` |
| `GetQueueSnapshot` | | the whole queue: `{rev, items, shuffle_order, position?, shuffle}` |
| `SetQueue` | mode + context (+ `start_index`/`shuffle` for replace) | `MutationResult {rev}` |
| `EditQueue` | `jump {index}` / `jump_physical {index}` / `move {from, to}` / `remove {index}` / `insert {index, keys}` | `MutationResult {rev}` |
| `GetExternalDevices` | `{kind}` | where an integration can play: `[{id, name, kind, active}]` |
| `SelectExternalDevice` | `{kind, device_id?}` | move playback there, or back to this app with no id |

Library:

| rpc | request | returns |
|---|---|---|
| `GetTracks` | `TracksRequest {filter, page}` | `{total, offset, items: [TrackInfo]}` |
| `GetFolderTracks` | `{prefix, page}` | same shape |
| `GetTracksByKeys` | `{keys}` | rows in the order asked for |
| `GetAlbums` / `GetAlbum` / `GetAlbumTracks` | | album rows and their tracks |
| `GetRecentlyAddedAlbums` | `Page` | newest first, by each album's newest track |
| `GetArtists` / `GetArtistTracks` / `GetArtistSampleTracks` | | artist rows and their tracks |
| `GetGenres` / `GetTopGenre` / `GetGenreTracks` | | |
| `GetRecentTracks` | `Page` | most recently played first |
| `Search` | `{query}` | tracks and albums; a remote source answers over the network |
| `GetTrackWebUrl` / `GetAlbumWebUrl` | `{key}` / `{id}` | the source's public page, absent when it has none |
| `GetStats` | | `{listen_counts: {uid: count}}` |
| `GetLyrics` | `{key}` | `{plain?, synced: [lines with word timing]}` |
| `GetFavorites` / `SetFavorite` | | `{refs, generation}`; a rejected remote push reverts and emits a `notice` |
| `RefreshArtistArtwork` | `{names}` | find photos for these artists |

Browsing what the library does not hold:

| rpc | request | returns |
|---|---|---|
| `GetCatalog` | `{continuation?}` | shelves of tiles; songs come as `TrackInfo` and are registered, so their keys queue |
| `GetCatalogDetail` | `{kind, id, continuation?}` | one album, playlist or artist: its tracks, or its own shelves |
| `GetRadioStations` | | every station the configured registries hold |
| `SearchRadio` | `{query, limit}` | the public directory; hits join the live registry |
| `PinRadioStation` | `{id, pinned}` | |
| `ValidateRadioRegistry` | `{url}` | how many stations that URL holds, or `invalid_input` |

Playlists: `GetPlaylists`, `CreatePlaylist`, `RenamePlaylist`,
`DeletePlaylist`, `AddPlaylistTracks`, `RemovePlaylistTrack`,
`ReorderPlaylist`, `RefreshPlaylist`, and the folder equivalents.

Changing the library: `UpdateTrackMetadata` (tags and the embedded cover in
one call), `DeleteTracks {keys, from_disk}`, `DeleteAlbum`, `UploadArtwork`,
`RemoveArtwork`. A from-disk delete is refused for anything outside the
configured library roots.

Jobs: `StartJob {kind}` (scan / library_sync / favorites_sync /
playlist_sync), `CancelJob`, `StartDownloads {keys}`, `RemoveDownload`,
`GetDownloadStatuses`, `StartYtdlp {url, output_dir, format, options}`.
A job kind is single-flight: a second start returns ALREADY_EXISTS.

Sources and integrations: `GetSources`, `SelectSource`, `UpsertServer`,
`DeleteServer`, `UpsertLocalSource`, `DeleteLocalSource`,
`SetSourceDirectories`, `ProvisionCredentials`, `LoginSource`,
`ClearCredentials`, `AuthenticateSource`, `BrowseSource`, `ValidateSource`,
`CanOpenBrowser`, and `GetIntegrations` / `ProvisionIntegration` /
`ClearIntegration` / `AuthenticateIntegration` for the scrobblers.

Settings: `GetConfig`, `SetConfig`, `PreviewEqualizer` (heard, not kept).

Artwork: `GetArtwork {target, hq}` streams `ArtworkChunk`, the first
carrying `content_type`.

Queue **contexts** materialize daemon-side, so "play this album" never
round-trips a track list through the client: `tracks {keys}`, `album {id}`,
`artist {name}`, `genre {name}`, `playlist {id}`, `filter {TrackFilter}`,
`radio {station_id, stream_id}`, `track_radio {key}` and `playlist_radio
{id}` for a source's own mixes.

`TrackInfo`: `key` (the library ref -- use it for queueing, favorites and
lyrics), `uid` (the same track qualified by its source,
`"<service>:<id>"`), `title`, `artist`, `artists`, `album`, `album_id`,
`duration_ms?`, `khz`, `bitrate`, `track_number?`, `disc_number?`, `kind`
(normal/radio), `seekable`, `offline`, `service?`, the MusicBrainz ids,
`playlist_item_id?`, and `artwork?`.

## Artwork

Rows carry a reference, not an image: `ArtworkRef {target, version}`, where
`target` names the entity (`track`, `album`, `artist`, `playlist`,
`catalog`, `station`) and `version` changes exactly when the picture does.
An absent ref means the entity has no picture, so a client draws its
placeholder and asks for nothing.

`GetArtwork` streams the bytes for a ref's target. Two things follow from
that shape:

- **The bytes are the contract.** A client that cannot render a URL asks
  for a ref and gets an image; nothing needs a webview.
- **The version is the cache key.** It is a hash of the resolved cover
  reference, so a response is safe to cache forever and a new photo, a
  re-uploaded cover or a rotated tag produces a different ref.

Why it works this way: a Jellyfin or Subsonic cover URL is signed with the
account's credentials, and those never leave the daemon. A client that
built its own URL could only ever fetch local files.

## External playback

Some services do not hand out decodable audio. Spotify plays itself, either
in a browser tab running its Web Playback SDK or on a Connect device the
account already owns, and the daemon drives both.

To a client it looks like ordinary playback. `PlayerState.external` names
the integration and the device when one owns playback; the transport
commands are the same ones; the queue, the shuffle order and what plays
next stay the daemon's. When the queue reaches a track only the integration
can play the daemon hands it over, and when it reaches one the engine can
play it hands back.

`GetExternalDevices {kind}` lists where an integration can play and
`SelectExternalDevice {kind, device_id?}` moves it, with an absent id
meaning this machine's own player.
## Minimal client (Python)

```sh
pip install grpcio grpcio-tools
python -m grpc_tools.protoc -I proto --python_out=. --grpc_python_out=. proto/kopuz.proto
```

```python
import os, pathlib
import grpc
import kopuz_pb2 as pb
import kopuz_pb2_grpc as rpc

sock = pathlib.Path(os.environ["XDG_RUNTIME_DIR"]) / "kopuz/kopuzd.sock"
channel = grpc.insecure_channel(f"unix:{sock}")
stub = rpc.KopuzStub(channel)

state = stub.GetPlayerState(pb.GetPlayerStateRequest())
print(state.track.title if state.HasField("track") else "nothing playing")

print("toggled at rev", stub.Toggle(pb.ToggleRequest()).rev)
```

## Capability caveats

- A daemon built without a service answers UNIMPLEMENTED for its RPCs
  instead of lying; probe once and hide the feature. Both shapes serve
  everything today.
- Browser frontends need a grpc-web proxy in front of the daemon; the
  daemon itself speaks plain gRPC only.
- Browser sign-ins (YouTube Music, Apple Music, SoundCloud, Spotify) are
  run by the daemon, so they need a machine it can open a browser on. Ask
  `CanOpenBrowser` first: a sandboxed daemon says no. Local files,
  Jellyfin, Subsonic/Navidrome and Nextcloud need nothing but the daemon.
- Spotify audio comes out of a browser with Widevine, spawned by the
  daemon, so a headless box with no browser cannot play it -- though it can
  still drive a Connect device that can.
## Settings

`Config` mirrors the app's settings struct field for field. It is a real
message, not JSON in a string: the settings are a closed Rust struct on both
ends of one binary, so the schema says so and a wrong key or type fails to
compile rather than at runtime.

Two groups of fields never cross: credentials (media-server logins, Last.fm
and Libre.fm keys, the MusicBrainz token) and machine-local path state
(`offline_tracks`). They come back as defaults in the view, and `SetConfig`
ignores whatever you send for them -- the daemon keeps its own -- so reading
a view and writing it straight back cannot erase them.

`SetConfig` replaces the surface wholesale rather than patching: read the
view, change what you want, send it back. The daemon diffs it against what
it holds and reports only the keys that actually changed in
`config.changed`. `locked_keys` are pinned by a managed settings layer -- a
read-only or Nix-store `settings.toml`, a `settings.d` drop-in, or a
`KOPUZ_CONFIG_*` override -- and changing one is refused; leaving it at the
value you read is not, so a read-modify-write of any other key still works.
