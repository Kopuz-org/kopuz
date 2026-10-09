# kopuzctl

`kopuzd` runs the player; `kopuzctl` controls a running player. The desktop app
also serves the daemon API, so the same commands work while its window is open.
Run either the app or `kopuzd` against a library. The desktop app currently
embeds its own daemon and cannot attach to a separately running `kopuzd`.

## Install and connect

The Nix package includes both binaries. To build them from a checkout:

```sh
cargo build --release -p kopuz-kopuzd -p kopuz-ctl
```

Run `kopuzd` in one terminal and `kopuzctl status` in another. For development,
`just daemon` and `just ctl status` use debug builds and the debug database.
Install matching versions of both binaries; the client refuses a daemon with a
different protocol revision.

The client discovers the same default socket as the daemon: the user runtime
directory on Linux, the user cache directory on macOS, or the current user's
named pipe on Windows. Use `--socket PATH` or `KOPUZ_SOCKET` for a custom
address. No daemon starts automatically when a client connects.

```sh
kopuzctl --socket /path/to/kopuzd.sock status
kopuzctl --address 127.0.0.1:7770 --token-file /path/to/kopuzd.token status
```

TCP needs `kopuzd --listen` and its token file. It uses unencrypted HTTP/2; see
[the transport documentation](api.md#over-tcp).

## Playback and library

```sh
kopuzctl status
kopuzctl search "artist or title" --limit 20
kopuzctl play TRACK_KEY
kopuzctl pause
kopuzctl play
kopuzctl next
kopuzctl previous
kopuzctl seek 90
kopuzctl volume 65
kopuzctl queue --offset 0 --limit 50
kopuzctl stop
```

Search lists the active source's indexed library. Use the returned `key` with
`play`; multiple keys replace the queue in the supplied order. `play` without
keys resumes playback, `toggle` switches play/pause, and `stop` stops playback
while leaving the daemon running. Seek positions are absolute seconds.

## Sources

Service IDs and setup fields come from the daemon. Inspect them before adding a
source, including required fields, defaults and choice values:

```sh
kopuzctl services
kopuzctl services jellyfin
kopuzctl source add jellyfin --name Home --url https://music.example
kopuzctl source list
kopuzctl source login SOURCE_ID --username alice
kopuzctl source use SOURCE_ID
kopuzctl source check SOURCE_ID
```

Adding a source prints its ID. `--activate` on `source add` also selects it.
Additional nonsecret setup fields use repeated `--field KEY=VALUE` flags. For
example, a folder source uses the daemon's `directories` field:

```sh
kopuzctl source add folders --name Music --field 'directories=["/home/alice/Music"]'
```

Password sign-in prompts without echoing input. Scripts can use
`source login SOURCE_ID --username alice --password-stdin` and pipe a secret
from their secret store. With no username, `source login SOURCE_ID` requests the
service's browser sign-in flow on the daemon's machine.

`source token SOURCE_ID --user-id alice` prompts for a token obtained elsewhere;
add `--secret-stdin` to read it from standard input. A write-only setup field
can instead be supplied with `source add ... --secret-stdin FIELD`. Secrets are
never accepted as password/token arguments or printed in source results.
`source logout SOURCE_ID` clears credentials; `source remove SOURCE_ID` removes
the configured source.

## Jobs and scripting

```sh
kopuzctl jobs start scan
kopuzctl jobs start library-sync
kopuzctl jobs start favorites-sync
kopuzctl jobs start playlist-sync
kopuzctl jobs
kopuzctl jobs cancel JOB_ID
kopuzctl --json source list
kopuzctl --json status
```

Job commands return immediately with a job ID. Use `jobs` to check completion.
`--json` writes one JSON value to standard output. Diagnostics go to standard
error; failed commands exit nonzero. Requests time out after 300 seconds,
including browser sign-in; override this with `--timeout SECONDS`. `--help`
works at every command level.

## Run with systemd

On Linux, `packaging/systemd/kopuzd.service` is an optional user unit. The Nix
package installs it under `share/systemd/user`. For a manual installation, copy
the unit to `~/.config/systemd/user/kopuzd.service` and set `ExecStart` to the
absolute path of the installed release binary. Then:

```sh
systemctl --user daemon-reload
systemctl --user enable --now kopuzd.service
kopuzctl status
journalctl --user -u kopuzd.service -f
```

Use `systemctl --user stop kopuzd.service` to stop the daemon and
`systemctl --user disable kopuzd.service` to prevent startup at login. Stop the
service before opening the desktop app against the same library. The unit uses
the user's existing audio session and does not enable a TCP listener.
