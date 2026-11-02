# kopuz

A [GPUI](https://gpui.rs) frontend for the [Kopuz](https://github.com/Kopuz-org/kopuz)
music daemon, built with [gpui-kit](https://github.com/longbridge/gpui-kit).

Kopuz is split into a daemon that owns the library, the media sources and
playback, and a frontend that talks to it over a socket. This is a second
frontend for that daemon, native rather than webview-based.

## Running

From the repo root, `just gpui run` builds both and starts them together. By
hand, start a daemon, then the app:

```sh
kopuzd --socket $XDG_RUNTIME_DIR/kopuzd.sock
nix develop --command cargo run -p kopuz
```

The app finds the daemon at the standard socket path. `KOPUZ_SOCKET` points it
at another one, and `KOPUZ_DAEMON_ADDR=host:port` attaches to a daemon started
with `--listen`.

## Status

Early. The daemon seam, the shared models and the window shell work; the pages
are being ported against `docs/PARITY.md`.

## License

EUPL-1.2, as Kopuz is.
