# Building a custom Kopuz frontend

A custom frontend does not need to run the Kopuz GUI. It connects to the
standalone `kopuzd` process over gRPC. The daemon owns playback, the queue,
the library database, configuration, jobs, downloads, integrations, and OS
media controls. The frontend owns presentation and user interaction.

```text
custom frontend  <--- gRPC on loopback --->  kopuzd  ---> audio and storage
```

The wire contract is
[`crates/proto/proto/kopuz.proto`](../crates/proto/proto/kopuz.proto), in the
`kopuz.v1` protobuf package. Any language with protobuf and gRPC support can
implement a frontend. The examples below use Python.

For the complete RPC and state semantics, also read [The Kopuz daemon
API](api.md).

## Choose how to provide the daemon

A frontend can treat `kopuzd` as a system dependency or bundle it as a
sidecar executable.

### System dependency

The user installs and starts `kopuzd` separately. The frontend only discovers
and connects to it. This keeps the frontend package small and lets several
frontends share one playback session.

During development, build the daemon from the Kopuz repository:

```sh
cargo build --release -p kopuz-daemon --features kopuzd --bin kopuzd
```

The executable is written to `target/release/kopuzd`, or
`target/release/kopuzd.exe` on Windows.

### Bundled sidecar

A standalone desktop frontend can include a prebuilt `kopuzd` for each
supported platform. Put the matching executable in the application package
and start it when no Kopuz API server is already running.

Do not require end users to install Rust or compile Kopuz. Build each sidecar
for its target in release mode, distribute checksums with it, and sign it by
the same process used to sign the frontend. If binaries are downloaded at
runtime, verify both their version and signature before executing them.

The sidecar is a process boundary, not a library binding. A Python frontend
should not load the Rust daemon into its interpreter, and it should never read
or write the Kopuz SQLite database directly.

## Daemon lifecycle

Use this startup sequence:

1. Look for the discovery file.
2. If it describes a server that answers an authenticated `GetStatus`, attach
   to that server. It may be `kopuzd` or the Kopuz GUI's embedded daemon.
3. If no server answers and the frontend bundles `kopuzd`, start the sidecar.
4. Wait for a valid discovery file and verify it with `GetStatus`.
5. Check `api_version` before enabling the rest of the UI.

Never start a second daemon merely because its PID looks unfamiliar. A live
server discovered for the current user owns the playback session and the
single-writer database. Attach to it.

Several frontends may race to start a daemon. `kopuzd` protects the discovery
file, so only one process will claim it. Treat an early sidecar exit as a cue
to retry discovery before reporting failure.

Decide whether the daemon is application-scoped or user-scoped:

- An application-scoped daemon can receive `Shutdown` when the frontend that
  started it exits.
- A user-scoped daemon stays alive so playback and other frontends continue.

Never shut down a daemon that was already running when the frontend started.
The embedded GUI server does not support `Shutdown`.

Start a bundled daemon without a shell and keep its diagnostics available:

```python
import subprocess

daemon = subprocess.Popen(
    [str(kopuzd_path), "--bind", "127.0.0.1:0"],
    stdin=subprocess.DEVNULL,
)
```

`kopuzd` accepts `--db-path <file>` when a frontend needs an isolated library.
Avoid choosing the normal Kopuz database path if another Kopuz process could
be using it.

## Discovery and authentication

The serving process writes this JSON record with user-only permissions:

```json
{"port": 49312, "token": "6f2c...", "pid": 74211}
```

The discovery path is:

- Linux: `$XDG_RUNTIME_DIR/kopuz/daemon.json`, falling back to the user cache
  directory when no runtime directory exists
- macOS: `~/Library/Caches/kopuz/daemon.json`
- Windows: `%LOCALAPPDATA%\kopuz\daemon.json`

This Python helper follows those locations:

```python
import os
import pathlib
import sys


def kopuz_discovery_path() -> pathlib.Path:
    if sys.platform == "linux":
        runtime = os.environ.get("XDG_RUNTIME_DIR")
        if runtime:
            base = pathlib.Path(runtime)
        else:
            base = pathlib.Path(
                os.environ.get("XDG_CACHE_HOME", pathlib.Path.home() / ".cache")
            )
    elif sys.platform == "darwin":
        base = pathlib.Path.home() / "Library" / "Caches"
    elif os.name == "nt":
        local_app_data = os.environ.get("LOCALAPPDATA")
        if not local_app_data:
            raise RuntimeError("LOCALAPPDATA is not set")
        base = pathlib.Path(local_app_data)
    else:
        raise RuntimeError(f"unsupported discovery platform: {sys.platform}")
    return base / "kopuz" / "daemon.json"
```

Connect to `127.0.0.1:<port>` and include this metadata on every Kopuz RPC:

```text
authorization: Bearer <token>
```

The connection is plaintext HTTP/2 because it is loopback-only. Do not expose
the daemon directly to an untrusted LAN or the public internet. Use an SSH
tunnel or another authenticated encrypted boundary for remote access.

The PID is diagnostic information. A process existing at that PID does not
prove that it is the same Kopuz server. The authenticated `GetStatus` call is
the liveness check.

## Generate Python bindings

Copy `kopuz.proto` into the frontend repository and pin it to the daemon API
version the frontend supports. Then generate bindings:

```sh
python -m pip install grpcio grpcio-tools
python -m grpc_tools.protoc \
  -I proto \
  --python_out=generated \
  --grpc_python_out=generated \
  --pyi_out=generated \
  proto/kopuz.proto
```

Add `generated` to the package import path, or adjust the generated import to
match the frontend's Python package layout.

The server also exposes unauthenticated gRPC reflection for development:

```sh
grpcurl -plaintext 127.0.0.1:<port> list kopuz.v1.Kopuz
```

Actual Kopuz RPCs still require the bearer token.

## Connect from Python

This example reads the discovery record, authenticates, checks the API
version, and fetches the initial player state:

```python
import json

import grpc

import kopuz_pb2 as pb
import kopuz_pb2_grpc as rpc


API_VERSION = 1


async def connect():
    record = json.loads(kopuz_discovery_path().read_text(encoding="utf-8"))
    channel = grpc.aio.insecure_channel(f"127.0.0.1:{record['port']}")
    stub = rpc.KopuzStub(channel)
    metadata = (("authorization", f"Bearer {record['token']}"),)

    status = await stub.GetStatus(pb.Empty(), metadata=metadata)
    if status.api_version != API_VERSION:
        await channel.close()
        raise RuntimeError(
            f"unsupported Kopuz API version {status.api_version}, "
            f"expected {API_VERSION}"
        )

    state = await stub.GetPlayerState(pb.Empty(), metadata=metadata)
    return channel, stub, metadata, state
```

Handle `grpc.StatusCode.UNIMPLEMENTED` as an unavailable capability and hide
that feature. Other RPC failures carry a stable `kopuz-error-code` metadata
value. Use that code for program logic and localization instead of matching
the human-readable message.

## Mirror state through `Attach`

Unary calls provide initial snapshots. Live updates and transport commands use
one bidirectional `Attach` stream per frontend.

The first client message must be `Hello`. For a fresh connection, send
`last_sequence=0` and fetch snapshots with unary calls. On reconnect, send the
highest event sequence already applied.

```python
class KopuzSession:
    def __init__(self, stub, metadata):
        self.stub = stub
        self.metadata = metadata
        self.last_sequence = 0
        self.next_request_id = 1
        self.pending_acks = {}

    async def run(self):
        call = self.stub.Attach(metadata=self.metadata)
        await call.write(
            pb.ClientMessage(
                hello=pb.Hello(last_sequence=self.last_sequence)
            )
        )

        async for message in call:
            if message.sequence:
                self.last_sequence = message.sequence

            message_kind = message.WhichOneof("msg")
            if message_kind == "ack":
                self.resolve_ack(message.ack)
                continue
            if message_kind != "event":
                continue

            event_kind = message.event.WhichOneof("kind")
            if event_kind is None:
                continue
            if event_kind == "resync":
                await self.refetch_snapshots()
            else:
                await self.apply_event(event_kind, message.event)

    async def send_toggle(self, call):
        request_id = self.next_request_id
        self.next_request_id += 1
        await call.write(
            pb.ClientMessage(
                command=pb.Command(
                    request_id=request_id,
                    toggle=pb.Empty(),
                )
            )
        )
        return request_id
```

In a real client, keep the stream call as session state, resolve each Ack by
`request_id`, reconnect with backoff after transport errors, and cancel all
pending command waiters when a connection is lost. An `Ack.error` is a command
failure and does not close the stream.

The server retains 512 events for reconnection. If the requested sequence is
too old, it emits `resync`. Refetch at least `GetPlayerState` and the visible
queue window before applying further incremental updates.

Unknown event variants decode with no selected oneof. Ignore them. Treat
unknown enum numbers as their `UNSPECIFIED` behavior. These rules let older
frontends continue working when additive protocol fields are introduced.

## Render player state correctly

`PlayerState` contains both `phase` and `intent`:

- `phase` reports current audio engine truth.
- `intent` reports what the daemon is trying to do.

Use `intent` for optimistic controls. For example, show a pause control while
a track is loading with the intention to play, even before the engine reports
the playing phase.

Position is an anchor rather than a once-per-second event stream. A state
snapshot carries daemon `now_ms`, while the position contains `{ms, at_ms,
playing}`. Record the offset between the local monotonic clock and daemon
`now_ms`, then interpolate locally while `playing` is true:

```python
import time


def local_monotonic_ms() -> int:
    return time.monotonic_ns() // 1_000_000


def clock_offset_ms(state) -> int:
    return local_monotonic_ms() - state.now_ms


def displayed_position_ms(position, offset_ms: int) -> int:
    if not position.playing:
        return position.ms
    estimated_daemon_now = local_monotonic_ms() - offset_ms
    elapsed = max(0, estimated_daemon_now - position.at_ms)
    return position.ms + elapsed
```

Clamp the displayed position to the track duration when a duration is present.
Radio tracks may have no duration and are not necessarily seekable.

While `PlayerState.fading` is present, display its track and position until the
crossfade resolves. This prevents the visible track from switching before the
audible transition finishes.

## Queue and library rules

Queue positions are logical play-order indices. This remains true while
shuffle is enabled. Use the indices returned by `GetQueue` and
`QueueSummary.index` for jump, move, and remove operations.

Prefer daemon-side queue contexts. To play an album, artist, genre, playlist,
filter result, or radio station, send that context through `SetQueue` rather
than fetching and resending every track.

Remote search, catalog, and radio results may not be stored in the library
yet. Queue those rows with the `inline_tracks` context. Use `QUEUE_MODE_INSERT`
with `insert_index` to insert at a logical play-order position. The daemon
validates literal tracks as remote media items, so this path cannot be used to
ask the daemon to open an arbitrary local filesystem path.

Use `TrackInfo.key` as the stable reference for queueing, favorites, lyrics,
downloads, and artwork. Do not infer file paths from keys. Server-side tracks
intentionally do not expose filesystem paths.

Fetch artwork incrementally with `GetArtwork`. The first chunk contains the
content type and later chunks contain bytes only. Stream chunks into a bounded
buffer or file instead of assuming the response is one message.

The daemon API is also the complete data and mutation boundary. Use the
library, album, artist, genre, recent, search, playlist, playlist-folder,
source, server, radio, artwork, metadata, favorites, download, and job RPCs in
`kopuz.proto`. `GetCatalog` returns the discover feed. `GetCatalogDetail`
returns paginated discovered playlists, remote album metadata and tracks, or
artist profiles and shelves. `GetTrackWebUrl` returns a provider share URL when
the active source has one.

Use `StreamLyrics` when the UI should improve the displayed result as better
providers finish. The stream can yield several complete replacements. The
last value is the final choice. `GetLyrics` remains available for clients that
only need the final result.

Credentials never appear in `GetConfig`, `GetSources`, events, logs, or normal
catalog DTOs. Create or update a credential-free server record with
`UpsertServer`, then send secrets only through `ProvisionCredentials`. Use
`AuthenticateSource` when Kopuz should own a supported browser sign-in flow.
Use `BrowseSource` to browse a configured Nextcloud account without receiving
its password. `ClearCredentials` revokes the stored source credential. A
config merge patch that tries to write credential-bearing fields is rejected.

Scrobbling credentials have their own write-only RPCs. Read only configured
flags with `GetIntegrationCredentials`. Store tokens or sessions with
`ProvisionIntegrationCredentials`, launch supported browser authorization
with `AuthenticateIntegration`, and revoke them with
`ClearIntegrationCredentials`. Secret values are never returned.

`StartYtdlp` runs the daemon's yt-dlp integration as a normal job. Progress and
completion use the job event stream. `GetDownloadStatuses` exposes individual
offline download byte progress, and `CancelDownloadItem` cancels one item
without cancelling the rest of its batch.

`SwitchSource` is a playback boundary. When the active source changes, the
daemon cancels any load, stops engine and external playback, and clears the
queue before the RPC returns. Refetch source-scoped views and player state
after it succeeds. Selecting the already active source is a no-op.

## External playback

The daemon can announce `external_command` events when a browser-based
frontend owns an external Spotify session. Only the frontend that owns that
external session should act on those commands. Other frontends must ignore
them.

Supporting external Spotify playback requires frontend-specific browser and
Widevine integration. A headless `kopuzd` cannot play Spotify audio by itself.
Call `GetExternalAccess` with `kind = "spotify"` when the user explicitly starts
or reconnects Spotify playback. It returns a refreshed short-lived access
token and the configured public Spotify client ID. It never returns the
refresh token. Use that access in the Spotify Web Playback SDK or Spotify
Connect API, then claim exclusive ownership with `ClaimExternalPlayback`.
Refresh ownership by calling `ReportExternalPlayback` at least once before the
returned lease expires. Each report includes the current track, position,
playing state, completion state, and device name, so the daemon can publish the
same player state, history, listens, scrobbles, and operating system media
state as the built-in GUI. Release ownership with `ReleaseExternalPlayback` on
handoff or shutdown. A stale or foreign lease ID is rejected.

While external playback is active, apply `external_command` transport events
to the Spotify session without sending the same command back to the daemon.
Queue changes should still be sent through `SetQueue`. Inline remote tracks
allow the daemon to preserve provider identity and resolve their artwork and
lyrics even when they are not yet in the local database. At handoff, release
the lease and resume the daemon engine queue.

A frontend that does not implement Spotify can still provide every daemon-owned
feature. It should hide Spotify playback controls while leaving catalog and
library browsing available according to the source capabilities.

## Browser frontends

`kopuzd` serves standard gRPC over HTTP/2, not gRPC-Web. Python and other
native clients connect directly. A browser frontend needs a trusted local
gRPC-Web proxy or a native host bridge. Keep the bearer token out of web
content that can be loaded from untrusted origins.

## Shipping checklist

- Pin and ship the supported `kopuz.proto` contract.
- Check `GetStatus.api_version` at connection time.
- Connect to a live discovered server before starting a sidecar.
- Bundle or document installation of `kopuzd` for every supported target.
- Keep the daemon on loopback unless an encrypted trusted tunnel is used.
- Send bearer metadata on every Kopuz RPC.
- Fetch initial snapshots before applying live events.
- Resume `Attach` with the last applied sequence.
- Refetch snapshots after `resync`.
- Correlate commands and Acks by `request_id`.
- Ignore unknown events and default unknown enum values.
- Interpolate position locally instead of polling the daemon.
- Use logical play-order queue indices.
- Stream artwork without unbounded buffering.
- Renew and explicitly release external playback leases.
- Report external playback state often enough to preserve position and
  completion accounting.
- Use write-only credential RPCs and never persist secrets in frontend logs.
- Never access the daemon's SQLite database from the frontend.
- Never stop a daemon the frontend did not start.
