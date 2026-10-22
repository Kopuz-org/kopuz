# Plugins

A plugin adds a source to Kopuz. It is a WebAssembly component built against
the WIT package in [`wit/`](../wit), plus a `kopuz-plugin.toml` manifest that
says what it is, which services it serves, what each of them supports and
which hosts it may reach. The daemon runs it in a sandbox: no files, no
sockets, no environment. Its only way out is the `http` import, and every
request and every redirect is checked against the hosts its manifest lists.
The database stays on the daemon's side; a plugin answers for the remote
service and the daemon caches what it returns.

There are two kinds:

- **Official plugins** live in [`plugins/`](../plugins) and are built into
  Kopuz. Subsonic (and custom Subsonic-compatible servers), Jellyfin, Clippsly
  and SoundCloud are served this way. They update with Kopuz and cannot be
  removed.
- **Community plugins** live in their authors' repositories and are installed
  from the marketplace under **Settings → Library → Plugins** with one click.
  They may add services of their own, which then show up in the source
  picker, marked experimental. They may not claim a service Kopuz serves
  itself, since that would hand them its sources' credentials.

Local folders, YouTube Music, Apple Music, Spotify, Nextcloud and SMB are
still built in: they need what a plugin cannot do yet (a JavaScript engine,
DRM, a browser, raw sockets, a disk cache).

## Writing one

Start a `cdylib` crate that depends on
[`kopuz-plugin-sdk`](../plugins/sdk), implement `Source`, and export it:

```rust
use kopuz_plugin_sdk::{AuthOutcome, Connection, Source, SourceError, StreamInfo, StreamQuality};
use kopuz_plugin_sdk::http::Client;

pub struct Radio {
    http: Client,
    url: String,
}

impl Source for Radio {
    fn open(connection: Connection) -> Self {
        Radio { http: Client::new(), url: connection.url }
    }

    fn validate(&self) -> AuthOutcome {
        match self.http.get(&self.url).send() {
            Ok(response) if response.is_success() => AuthOutcome::Valid,
            _ => AuthOutcome::Unreachable,
        }
    }

    fn resolve_stream(&self, item_id: &str, _: StreamQuality) -> Result<StreamInfo, SourceError> {
        Ok(StreamInfo::url(format!("{}/stream/{item_id}", self.url)))
    }
}

kopuz_plugin_sdk::export_source!(Radio);
```

Every other operation has a default. Implement what the service does and
declare it in the manifest; the daemon never asks for an operation whose
capability is off. Calls block: the HTTP client looks like reqwest without
the `.await`, and the daemon runs each call on its own async runtime, a few
instances per source so a long sync never holds up the next track.

Build it for `wasm32-wasip2` (the repository's toolchain file installs the
target):

```sh
cargo build --release --target wasm32-wasip2
```

[`examples/plugins/internet-archive`](../examples/plugins/internet-archive)
is a complete community plugin that adds a service of its own.

## The manifest

```toml
id = "internet-archive"      # unique; also its directory name
name = "Internet Archive"
version = "0.1.0"
api = "0.1"                  # the plugin API it was built against
description = "Albums and live recordings from archive.org."

[permissions]
# Hosts it may reach. `$server` is the address the source was added with;
# `*.example.com` matches example.com and every subdomain.
network = ["archive.org"]

[[service]]
id = "internet-archive"      # a new service, or an official plugin's slug
name = "Internet Archive"    # how a new service is shown in the picker
summary = "Freely licensed albums and live recordings"
icon = "M12 2L2 7..."        # optional SVG path in a 24x24 box
accent = "#428bca"           # optional
sign-in = "none"             # or "password", which the plugin's `login` exchanges
# url = "https://..."        # a hosted service's one address; leave out to ask for one
capabilities = { sync = true, downloads = true }
```

Capabilities default to off. The keys mirror the daemon's: `sync`,
`downloads`, `uploads`, `storage-quota`, `play-queue`,
`playlists = "none" | "add-remove" | "reorder"`,
`radio = ["track", "playlist", "search"]`,
`favorites-sync = "instant" | "paginated"`, and the rest listed in
`crates/plugin-host/src/manifest.rs`. A misspelled key is an error, not a
silently disabled feature.

## Installing by hand

Put the manifest and the component in a directory of their own under the
plugin directory, which sits next to the library database
(`KOPUZ_PLUGIN_DIR` overrides it):

```text
plugins/
  internet-archive/
    kopuz-plugin.toml
    plugin.wasm
```

The daemon reads it at start. A plugin installed this way with an official
plugin's id replaces the official one, which is how a fix to an official
plugin is tried without rebuilding Kopuz.

## Publishing to the marketplace

The marketplace is a JSON index, by default
`https://raw.githubusercontent.com/Kopuz-org/plugins/main/index.json`
(`KOPUZ_PLUGIN_INDEX` overrides it, and a `file://` index works for testing).
A listing names where the manifest and the component are, and the SHA-256 of
each:

```json
{
  "schema": 1,
  "plugins": [
    {
      "id": "internet-archive",
      "name": "Internet Archive",
      "version": "0.1.0",
      "api": "0.1",
      "description": "Albums and live recordings from archive.org.",
      "author": "someone",
      "homepage": "https://github.com/someone/kopuz-internet-archive",
      "services": ["internet-archive"],
      "network": ["archive.org"],
      "manifest": { "url": "https://.../kopuz-plugin.toml", "sha256": "..." },
      "component": { "url": "https://.../plugin.wasm", "sha256": "..." }
    }
  ]
}
```

Installing fetches both files over HTTPS, refuses either if its hash differs,
and refuses a manifest whose id, version, services or hosts differ from the
listing, so what the Plugins page showed is what gets installed. The
component is compiled once before anything is written, and the plugin
directory is swapped in with one rename.

To try the flow locally, build the examples into a local marketplace and run
the app against it (`just plugin-market` alone writes the index and prints
the `KOPUZ_PLUGIN_INDEX` to use with a daemon started some other way):

```sh
just serve-market
```

## Validation

```sh
cargo fmt --all -- --check
SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
SQLX_OFFLINE=true cargo clippy --workspace --all-targets --release -- -D warnings
SQLX_OFFLINE=true cargo test -p kopuz-plugin-host
SQLX_OFFLINE=true cargo test -p kopuz-server --test plugin_sources
SQLX_OFFLINE=true cargo test -p kopuz-daemon --test plugin_market
cargo test -p kopuz-plugin-subsonic -p kopuz-plugin-jellyfin -p kopuz-plugin-clippsly -p kopuz-plugin-soundcloud
```

A sandboxed packager that cannot run the nested cargo build of the official
plugins can build them first and set `KOPUZ_PLUGIN_WASM_DIR` to the directory
holding their `.wasm` files.
