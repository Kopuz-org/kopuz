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
  the configured source, scan/sync jobs, scrobbling, and OS media
  integration (MPRIS/SMTC/Now Playing). No window, no webview. Build it with
  `cargo build --release -p kopuz-daemon --features kopuzd --bin kopuzd`.
- **Embedded**: the desktop app can serve the identical API from its own
  process. This exists because SQLite is single-writer: `kopuzd` and the GUI
  must never run against the same library at once, so a frontend that wants
  to attach while the GUI is open attaches to the GUI itself.

`kopuzd` flags: `--socket <path>`, `--db-path <file>`, `--supervised`.

`--supervised` marks a daemon that was launched by a frontend: it exits when
that frontend's stream ends, however it ended. A daemon started by hand or by
a service manager does not get the flag and ignores clients coming and going.

The desktop app can also run the daemon as a child of its own executable,
which is what `kopuz --daemon` does -- one binary, two processes, two logs,
each dying with the other.

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
  `library_invalidated {table, generation}`, `job_progress`,
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

| rpc | request | returns |
|---|---|---|
| `GetStatus` | | `{version, uptime_secs}` |
| `GetPlayerState` | | `PlayerState` snapshot |
| `GetQueue` | `Page {offset, limit}` | play-order window `{rev, total, items: [{index, track}]}` |
| `GetTracks` | `TracksRequest {filter, page}` | `{total, offset, items: [TrackInfo]}` |
| `GetFolderTracks` | `{prefix, page}` | same shape |
| `GetStats` | | `{listen_counts: {uid: count}}` |
| `GetLyrics` | `{key}` | `{plain?, synced: [lines with word timing]}` |
| `GetFavorites` | | `{refs: [key], generation}` |
| `GetJobs` | | `[{id, kind, state, phase, current?, total?, message?, error?}]` |
| `GetDownloads` | | `{keys}` of offline-available tracks |
| `GetConfig` | | `{config, locked_keys}` (credentials absent) |
| `SetQueue` | mode + context (+ `start_index`/`shuffle` for replace) | `MutationResult {rev}` |
| `EditQueue` | `jump {index}` / `move {from, to}` / `remove {index}` (play-order indices) | `MutationResult {rev}` |
| `SetFavorite` | `{key, favorite}` | optimistic; a rejected remote push reverts and emits a `notice` |
| `StartJob` | `{kind}` (scan / library_sync / favorites_sync) | `{job_id}`; single-flight per kind, ALREADY_EXISTS on conflict |
| `CancelJob` | `{id}` | |
| `StartDownloads` | `{keys}` | `{job_id}` |
| `RemoveDownload` | `{key}` | |
| `SetConfig` | a whole `Config` | updated view; read it, change what you want, send it back |
| `GetArtwork` | one of `track`/`album`/`artist`, plus `hq` | a stream of `ArtworkChunk` (first chunk carries `content_type`); thumbnails resized daemon-side (400px, 1920px hq) |

Queue **contexts** materialize daemon-side, so "play this album" never
round-trips a track list through the client: `tracks {keys}`, `album {id}`,
`artist {name}`, `genre {name}`, `playlist {id}`, `filter {TrackFilter}`,
`radio {station_id, stream_id}`.

`TrackInfo`: `key` (the library ref — use it for queueing, favorites, and
as the `GetArtwork` track entity), `uid` (the same track qualified by its
source, `"<service>:<id>"` for server tracks), `title`, `artist`, `album`,
`album_id`, `duration_ms?`, `khz`, `bitrate`, `track_number?`,
`disc_number?`, `kind` (normal/radio), `seekable`, `offline`.

`NowPlaying` carries `key` and `uid` with those same two meanings. There is
no artwork field on either: `key` is the artwork entity id, so ask
`GetArtwork` for it and treat NOT_FOUND as "no cover".

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
  instead of lying; probe once and hide the feature. The GUI's embedded
  server currently runs without the config-write and downloads services
  (the GUI owns those); `kopuzd` serves everything.
- Browser frontends need a grpc-web proxy in front of the daemon; the
  daemon itself speaks plain gRPC only.
- Browser-based sign-ins (YT Music, Apple Music, SoundCloud, Spotify)
  cannot complete on a browserless box; local files, Jellyfin,
  Subsonic/Navidrome, and Nextcloud need nothing but the daemon.
- Spotify playback is machine-bound to a browser with Widevine next to the
  GUI; a headless `kopuzd` cannot play Spotify audio itself.

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
