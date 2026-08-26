# The Kopuz daemon API

Kopuz's playback core is a daemon. Every feature the built-in GUI has goes
through the gRPC surface documented here, so a frontend in any language with
protobuf support is a first-class citizen. The contract is one file:
`crates/proto/proto/kopuz.proto` (package `kopuz.v1`). Copy it, generate
stubs for your language, connect.

Current API version: `1` (reported by `Kopuz/GetStatus`).

## Quickstart for frontend authors

1. Have any Kopuz running. The desktop app serves the API by default (the
   Remote Control API toggle in Settings is the off switch); `kopuzd` serves
   it headless. Users install nothing extra and issue no API keys.
2. Read the discovery file for `{port, token, pid}` (paths below). Its 0600
   permissions are the trust model: same OS user, trusted; anyone else
   cannot read the token.
3. Copy `proto/kopuz.proto` into your project and run your language's
   protobuf/gRPC codegen. For poking around without codegen, use `grpcurl`:
   reflection is enabled.
4. Send `authorization: Bearer <token>` metadata on every call.
5. Read state: `GetPlayerState`, `GetQueue`, `GetTracks`.
6. Open the bidirectional `Attach` stream: send `Hello` first, then mirror
   the `Event` messages it streams (`player_state`, `queue_changed`, ...);
   interpolate the seek bar from the position anchor. Send transport
   commands on the same stream; each is answered by an `Ack`.
7. Treat `UNIMPLEMENTED` as "this daemon runs without that feature" and
   hide it; ignore `Event`s whose oneof you do not recognize (they decode
   as unset) and map unknown enum values to their defaults.
8. Reference clients: kopuz-tui (Go), the built-in GUI itself (the same
   core in-process), and the `kopuz`/`kopuzd` control CLI.

## Running the daemon

Two deployment shapes serve the same API:

- **Headless**: run `kopuzd`. It owns the audio engine, the SQLite library,
  the configured source, scan/sync jobs, scrobbling, and OS media
  integration (MPRIS/SMTC/Now Playing). No window, no webview. Build it with
  `cargo build --release -p kopuz-daemon --features kopuzd --bin kopuzd`.
  A systemd user unit ships in `packaging/systemd/kopuzd.service`.
- **Embedded**: the desktop app serves the identical API from its own
  process, on by default. This exists because SQLite is single-writer:
  `kopuzd` and the GUI must never run against the same library at once, so
  a frontend that wants to attach while the GUI is open attaches to the GUI
  itself.

Both binaries double as control clients: `kopuz pause`, `kopuz next`,
`kopuz seek 1:30`, `kopuz volume 80`, `kopuz status [--json]`, and the same
subcommands on `kopuzd`, act on whichever process is serving and exit.

`kopuzd` flags: `--bind 127.0.0.1:0` (default: loopback, ephemeral port),
`--token <hex>` (default: random), `--db-path <file>`.

## Discovery and auth

Whichever process serves the API writes a discovery file with `0600`
permissions:

- Linux: `$XDG_RUNTIME_DIR/kopuz/daemon.json`
- macOS: the user cache dir, `~/Library/Caches/kopuz/daemon.json` (the
  exact path is logged at startup)

```json
{ "port": 49312, "token": "6f2c…", "pid": 74211 }
```

Every RPC needs the token as metadata, constant-time checked:

```
authorization: Bearer <token>
```

The connection is plaintext HTTP/2 on loopback. If you bind a LAN address
you are trusting that network with your library and the bearer token;
prefer an SSH tunnel. Server reflection (v1 and v1alpha) is served without
auth so `grpcurl` can list the schema:

```sh
grpcurl -plaintext 127.0.0.1:<port> list kopuz.v1.Kopuz
grpcurl -plaintext -H "authorization: Bearer $TOKEN" \
  127.0.0.1:<port> kopuz.v1.Kopuz/GetPlayerState
```

## Errors

RPC failures are gRPC statuses. The stable machine-readable Kopuz code
rides the `kopuz-error-code` response metadata; localize by that code,
never by message. The gRPC code is the nearest standard equivalent:

| kopuz-error-code | gRPC code | meaning |
|---|---|---|
| `invalid_input` | INVALID_ARGUMENT | malformed request or out-of-range position |
| `unauthorized` | UNAUTHENTICATED | missing/invalid token |
| `source_auth_expired` | UNAUTHENTICATED | the media server needs a re-login |
| `not_found` | NOT_FOUND | unknown key/id |
| `conflict` | ABORTED | a single-flight job of that kind is already running |
| `unsupported` | UNIMPLEMENTED | this daemon runs without that service |
| `source_unreachable` | UNAVAILABLE | the media server did not answer |
| `internal` | INTERNAL | daemon-side failure |

Structured details, when present, are JSON in the
`kopuz-error-details-bin` metadata. Command failures on the Attach stream
arrive as `Ack.error` (an `ErrorBody` message) instead of killing the
stream. Unknown codes must be treated as `internal`; the set can grow.

## The Attach stream (the control plane)

`Kopuz/Attach` is one bidirectional stream per client: you write
`ClientMessage`, the daemon writes `ServerMessage`.

- **First message must be `Hello`**, carrying `last_sequence`: the highest
  event sequence you saw before a reconnect, or 0 for a fresh session. The
  daemon replays newer events from a 512-event ring, or sends one `resync`
  event when the gap is too old; then live events flow. On `resync`,
  refetch `GetPlayerState` and your queue window.
- **Events** arrive as `ServerMessage.event` with a monotonic `sequence`
  (your next resume point). The kinds mirror the state machine:
  `player_state` (full snapshot on every transition), `position` (new
  anchor after seek/pause), `buffered`, `queue_changed`,
  `library_invalidated {table, generation}`, `job_progress`,
  `job_finished`, `config_changed`, `source_status`, `notice`,
  `external_command`, `resync`.
- **Commands** (`play`, `pause`, `toggle`, `next`, `previous`, `stop`,
  `seek {position_ms}`, `volume {0..1}`, `mode {shuffle?, loop?}`) ride the
  same stream with a client-chosen `request_id`; each is answered by an
  `Ack {request_id, rev}` where `rev` names the state revision containing
  the command's effect, so you can wait for your mirror to catch up before
  trusting it.
- An events-only client may half-close its send side after `Hello`; the
  daemon keeps streaming.
- `external_command`: a transport command that arrived while playback is
  external (Spotify in a browser next to a frontend). Only the frontend
  that owns the external session acts on it; everyone else ignores it.

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

| rpc | request | returns |
|---|---|---|
| `GetStatus` | | `{version, api_version, uptime_secs}` |
| `GetPlayerState` | | `PlayerState` snapshot |
| `GetQueue` | `Page {offset, limit}` | play-order window `{rev, total, items: [{index, track}]}` |
| `GetQueueSnapshot` | | durable physical-order queue, progress, and shuffle permutation |
| `GetTracks` | `TracksRequest {filter, page}` | `{total, offset, items: [TrackInfo]}` |
| `GetFolderTracks` | `{prefix, page}` | same shape |
| `GetStats` | | `{listen_counts: {uid: count}}` |
| `GetLyrics` | `{key}` | `{plain?, synced: [lines with word timing]}` |
| `StreamLyrics` | `{key}` | progressive stream of complete lyrics replacements |
| `GetFavorites` | | `{refs: [key], generation}` |
| `GetJobs` | | `[{id, kind, state, phase, current?, total?, message?, error?}]` |
| `GetDownloads` | | `{keys}` of offline-available tracks |
| `GetDownloadStatuses` | | per-item state and byte progress |
| `GetConfig` | | `{config_json, locked_keys}` (credentials stripped) |
| `GetAlbums`, `GetAlbum`, `GetArtists`, `GetGenres` | filters/pages where applicable | library summaries |
| `GetRecentTracks`, `GetAlbumTracks`, `GetArtistTracks`, `GetGenreTracks` | entity + page | track page |
| `GetArtistSampleTracks`, `GetTracksByKeys`, `GetTopGenre` | page/keys/empty | library data |
| `GetTrackWebUrl`, `GetAlbumWebUrl` | track/album ref | provider share URL when supported |
| `Search` | query + limits | tracks, albums, artists, playlists |
| `GetPlaylists`, `GetPlaylistTracks` | empty/entity + page | playlist tree and tracks |
| `RefreshPlaylist` | playlist + page | refreshed playlist tracks |
| `GetSources`, `ValidateSource` | empty/source | configured sources and health |
| `BrowseSource` | source + path | Nextcloud folder entries without credentials |
| `GetIntegrationCredentials` | | configured flags only, never secret values |
| `GetCatalog`, `GetCatalogDetail` | provider catalog requests | discover feed and entity details |
| `GetRadioStations`, `SearchRadio`, `GetRadioRegistries` | empty/query | radio data |
| `StartTrackRadio`, `StartPlaylistRadio` | track/playlist | generated track list; track radio places its seed first |
| `GetExternalAccess` | provider kind | short-lived frontend access, never refresh credentials |
| `SetQueue` | mode + context (+ `start_index`/`shuffle` for replace) | ack |
| `EditQueue` | `jump {index}` / `move {from, to}` / `remove {index}` (play-order indices) | ack |
| `SaveQueueSnapshot` | durable physical-order queue snapshot | empty |
| `SetFavorite` | `{key, favorite}` | optimistic; a rejected remote push reverts and emits a `notice` |
| `StartJob` | `{kind}` (scan / library_sync / favorites_sync / playlist_sync) | `{job_id}`; single-flight per kind, ABORTED on conflict |
| `CancelJob` | `{id}` | |
| `StartDownloads` | `{keys}` | `{job_id}` |
| `RemoveDownload` | `{key}` | |
| `CancelDownloadItem` | `{key}` | cancel one item without stopping its batch |
| `PatchConfig` | RFC 7396 merge patch as JSON in `merge_patch_json` | updated view; credential keys refused, `locked_keys` are pinned by a managed settings layer |
| `CreatePlaylist`, `RenamePlaylist`, `DeletePlaylist` | playlist mutation | entity/empty |
| `AddPlaylistTracks`, `RemovePlaylistTracks`, `ReorderPlaylistTracks` | playlist + keys | empty |
| `CreatePlaylistFolder`, `RenamePlaylistFolder`, `DeletePlaylistFolder`, `MovePlaylist` | folder mutation | entity/empty |
| `SwitchSource`, `UpsertLocalSource`, `DeleteLocalSource`, `SetSourceDirectories` | source mutation | source/empty |
| `UpsertServer`, `DeleteServer` | credential-free server mutation | source/empty |
| `ProvisionCredentials`, `LoginSource`, `ClearCredentials` | dedicated secret operation | source/empty |
| `AuthenticateSource` | source | daemon-owned browser sign-in where supported |
| `ProvisionIntegrationCredentials`, `ClearIntegrationCredentials` | write-only scrobbling secret operation | configured flag/empty |
| `AuthenticateIntegration` | integration + public app credentials where needed | configured flag |
| `ClaimExternalPlayback` | provider + device | short-lived exclusive lease |
| `ReportExternalPlayback` | lease + track/position/playing/completed/device | renews lease and projects full playback state |
| `ReleaseExternalPlayback` | lease | returns playback ownership to the daemon |
| `StartYtdlp` | URL, output directory, format, options | `{job_id}` |
| `SetExternalPlayback` | legacy request | retained for wire compatibility; returns UNIMPLEMENTED |
| `AddRadioRegistry`, `RemoveRadioRegistry`, `SetRadioRegistryEnabled`, `PinRadioStation` | radio mutation | empty |
| `UpdateTrackMetadata`, `DeleteTracks`, `DeleteAlbum` | library mutation | track/empty |
| `UploadArtwork`, `RemoveArtwork` | artwork mutation | empty |
| `Shutdown` | | daemon flushes and exits; UNIMPLEMENTED on the embedded server |
| `GetArtwork` | one of `track`/`album`/`artist`, plus `hq` | a stream of `ArtworkChunk` (first chunk carries `content_type`); thumbnails resized daemon-side (400px, 1920px hq) |

Queue **contexts** materialize daemon-side, so "play this album" never
round-trips a track list through the client: `tracks {keys}`, `album {id}`,
`artist {name}`, `genre {name}`, `playlist {id}`, `filter {TrackFilter}`,
`radio {station_id, stream_id}`, or `inline_tracks {tracks}` for ephemeral
remote search, catalog, and radio results. `insert` mode requires
`insert_index` in logical play order.

`TrackInfo`: `key` (stable ref, use it for queueing/favorites/artwork),
`uid`, `title`, `artist`, `album`, `album_id`, `duration_ms?`, `khz`,
`bitrate`, `track_number?`, `disc_number?`, `kind` (normal/radio),
`seekable`, `artwork?` (an opaque hint that artwork exists; fetch bytes
via `GetArtwork`), `offline`.

## Minimal client (Python)

```sh
pip install grpcio grpcio-tools
python -m grpc_tools.protoc -I proto --python_out=. --grpc_python_out=. proto/kopuz.proto
```

```python
import json, os, pathlib
import grpc
import kopuz_pb2 as pb
import kopuz_pb2_grpc as rpc

disc = json.loads((pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", "/tmp")) / "kopuz/daemon.json").read_text())
channel = grpc.insecure_channel(f"127.0.0.1:{disc['port']}")
stub = rpc.KopuzStub(channel)
auth = (("authorization", f"Bearer {disc['token']}"),)

state = stub.GetPlayerState(pb.Empty(), metadata=auth)
print(state.track.title if state.HasField("track") else "nothing playing")

def messages():
    yield pb.ClientMessage(hello=pb.Hello(last_sequence=0))
    yield pb.ClientMessage(command=pb.Command(request_id=1, toggle=pb.Empty()))

for msg in stub.Attach(messages(), metadata=auth):
    if msg.HasField("ack") and msg.ack.request_id == 1:
        print("toggled at rev", msg.ack.rev)
        break
```

## Capability caveats

- A daemon built without a service answers UNIMPLEMENTED for its RPCs
  instead of lying; probe once and hide the feature. The GUI's embedded
  server and `kopuzd` expose the same feature services. Only process ownership
  differs: `Shutdown` is unavailable on the embedded server.
- Browser frontends need a grpc-web proxy in front of the daemon; the
  daemon itself speaks plain gRPC only.
- Browser-based sign-ins (YT Music, Apple Music, SoundCloud, Spotify)
  cannot complete on a browserless box; local files, Jellyfin,
  Subsonic/Navidrome, and Nextcloud need nothing but the daemon.
- Spotify playback is frontend-owned and needs a browser with Widevine. A
  headless `kopuzd` cannot play Spotify audio itself, but it provides access,
  queue, ownership lease, state reporting, history, scrobbling, and media
  control operations for a custom frontend that implements playback.
