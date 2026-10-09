# SMB libraries

Add **SMB / NAS** from the source selector. Enter a share URL such as
`smb://nas/Music` or a subfolder such as `smb://nas/Music/Albums`, then sign in
with your NAS username and password. Domain accounts can use `DOMAIN\username`.
A nonstandard port is supported, for example `smb://nas:1445/Music`.

Sync scans audio files recursively and reads embedded tags and covers. Playback
reads directly from the share and supports seeking; no OS mount is required,
including on Android. Favorites are stored in Kopuz. The share is read-only:
remote edits, playlists, and offline downloads are unavailable. This source
requires SMB 2/3 and account authentication; discovery and guest login are not
provided.

## Validation

Run these checks on a development machine before marking it ready for review:

```sh
cargo fmt --all -- --check
SQLX_OFFLINE=true cargo clippy --workspace --all-targets -- -D warnings
SQLX_OFFLINE=true cargo clippy --workspace --all-targets --release -- -D warnings
SQLX_OFFLINE=true cargo test -p kopuz-server smb
SQLX_OFFLINE=true cargo test -p kopuz-reader smb
SQLX_OFFLINE=true cargo test -p kopuz-db service_round_trips
```

The optional read/seek test requires an existing nonempty file on a test share.
Set `KOPUZ_SMB_TEST_URL`, `KOPUZ_SMB_TEST_USER`, `KOPUZ_SMB_TEST_PASSWORD`, and
`KOPUZ_SMB_TEST_FILE` in your environment. The file path is relative to the
configured URL, such as `Album/01.flac`. It only lists and reads the share.

```sh
SQLX_OFFLINE=true cargo test -p kopuz-server smb_live_read_and_seek -- --ignored
```

On desktop and Android, verify sign-in, a subfolder scan, embedded art,
playback, seeking, favorites after restarting, and recovery after disconnecting
the NAS. Confirm that a failed sync preserves the previous library and that a
second share keeps its library and favorites separate. Record these checks
before marking the corresponding PR testing cells complete.
