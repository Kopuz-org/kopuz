# Agent Development Guide

A file for [guiding coding agents](https://agents.md/). Kopuz is a Rust + Dioxus
music player; one Cargo workspace holds the daemon and its frontends, laid out by
layer under `shared/`, `daemon/` and `apps/` (see Directory Structure). See
`CONTRIBUTING.md` for the human process and `README.md` for platform setup.

## Commands

- **Serve (dev):** `just serve` — regenerates Tailwind, then `dx serve --package kopuz-legacy`.
  Debug builds use a separate `kopuz-debug.db`, so this never touches real data
  (`KOPUZ_DB_PATH` overrides the location).
- **Build (release):** `just build` (= `dx build --package kopuz-legacy --release`).
- **GPUI frontend:** `just gpui run` builds it and `kopuzd` and starts both on
  a dev socket; `just gpui` lists the rest. See `apps/kopuz/CLAUDE.md`.
- **Lint (gate):** `cargo clippy --workspace --all-targets -- -D warnings`. Run it
  in **both** debug and `--release` — some code is `cfg`-gated. Prefix with
  `SQLX_OFFLINE=true` if it tries to reach a live DB.
- **Format:** `cargo fmt --all` (check: `cargo fmt --all -- --check`).
- **Test:** `SQLX_OFFLINE=true cargo test -p <crate>`, filtering by name where you
  can — the suite is large. E.g. `cargo test -p kopuz-db <test name>`.

Run clippy (debug + release), fmt, and the tests covering your change before each commit.

## Database (`daemon/db`)

- SQLite via `sqlx`. Schema migrations are `daemon/db/migrations/*.sql`; sqlx
  checksums their bytes, so keep them **LF** and never edit an already-applied
  migration — add a new one.
- `query!` / `query_as!` macros are compile-checked against `daemon/db/.sqlx/`.
  After adding or changing one, regenerate the cache: point `DATABASE_URL` at a
  temp DB, `sqlx migrate run`, then `cargo sqlx prepare` (run from `daemon/db`).
  Runtime `sqlx::query_as` (most track queries) is not macro-checked and needs no
  prepare.
- **Crate wall:** the frontend crates (`hooks`, `pages`, `components`) reach
  the daemon through `api` and nothing else. They do not depend on `db`,
  `daemon`, `server`, `reader` or (off Android) `player`: no database handle,
  no domain model, no media source, no credentials, no system integration. A
  feature that needs one of those is a daemon service with an API method, not a
  hook. Pages render the wire rows themselves — `api::TrackInfo`,
  `AlbumInfo`, `PlaylistCatalog` — so there is no conversion layer to keep in
  step.
- **Settings file:** `config::Settings` (typed sections, flattened to top-level
  keys) lives only in the TOML layers next to the DB (`shared/config/src/store.rs`):
  defaults → `settings.local.toml` → `settings.toml` → `settings.d/*.toml`
  drop-ins → `KOPUZ_CONFIG_*` env. State, credentials, sources and UI prefs are
  not settings; they live in their own daemon stores and tables. Change settings
  with typed `SettingsEdit`s through `edit_config`; a save writes only the keys
  that changed, into `settings.toml`, or `settings.local.toml` when the main
  file is hjem-managed (store symlink / read-only, never written). A key a
  managed file, drop-in or env var pins is refused per edit and renders locked,
  citing that file's path or the var's name. The daemon polls the files' mtimes
  and reloads live; env is read once.

## Sources & covers (`daemon/server`, daemon-only)

- Each backend implements the `MediaSource` trait (`source.rs`): Local, Jellyfin,
  Subsonic/Custom, YtMusic, SoundCloud, Spotify, Apple Music, Nextcloud. The
  daemon owns them; a frontend asks `SourceApi` what the active one can do
  and never branches on a service name.
- Cover resolution lives in `cover.rs` (`locate` / `track` / `from_path`);
  dispatch on the cover ref's own shape, not the active source. What a
  frontend sees is an `ArtworkRef`, resolved to bytes by `ArtworkApi`.
- **InnerTube headers are all-or-nothing.** Every `youtubei/v1/*` call sends
  `User-Agent` (from the `YouTubeClient`), `X-Goog-Api-Format-Version`,
  `X-YouTube-Client-Name`/`-Version`, `X-Origin` and `Referer`, plus
  `Cookie` + a SAPISIDHASH `Authorization` when signed in. YouTube answers a
  request missing them with a bare 403 while the endpoints that send them keep
  working from the same session, so the symptom looks like an expired login and
  is not. `discover.rs::post` is the reference; copy it rather than hand-rolling
  a builder.

## i18n (`apps/common/i18n`)

- Fluent `.ftl` in `apps/common/i18n/locales/`, baseline `en.ftl`. Add every new key to
  all locales — `scripts/check_locales.nu` (CI) requires parity with `en.ftl`, and
  `scripts/check_i18n_usage.nu` checks that keys are actually used.

## Conventions (enforced)

- Diagnostics via `tracing`; `println!` / `eprintln!` are clippy-denied outside
  explicit exceptions.
- `.clippy.toml` forbids holding a Dioxus signal borrow across `.await` — clone the
  value out first.
- Prefer real error handling over `unwrap()` / `expect()` outside tests.
- Keep comments to the non-obvious *why*; don't restate the code.

## Directory Structure

Three layers, enforced by `scripts/check_layers.nu` (CI):

- `shared/` — the daemon ↔ frontend contract. Depends on nothing else in the repo.
- `daemon/` — the daemon and everything behind it. Depends only on `shared/`.
- `apps/` — the frontends. `apps/common/` (shared by frontends only) depends only
  on `shared/`; each app (`apps/cli`, `apps/legacy/`, `apps/kopuz/`) on `shared/` and
  `apps/common/`, never on `daemon/` or another app. The one exception, listed
  in the script: the Dioxus app hosts the daemon in-process and drives `player`
  on Android.

A bare `cargo build` / `cargo test` covers the default members (everything but
the Dioxus app); pass `--workspace` or `-p kopuz-legacy` for it.

- `apps/legacy/kopuz` — the legacy Dioxus app, package and binary `kopuz-legacy`
  (entry `main.rs`, `build.rs` font/asset embedding + Android packaging). Hosts
  the daemon core in-process.
- `shared/api` — the client-facing trait and its wire types; `shared/proto` —
  the gRPC schema; `apps/common/client` — the same trait over a socket.
- `daemon/core` — the core: session, library, catalog, radio, sources,
  mutations, jobs, artwork, and `LocalApi` over them. `daemon/kopuzd` — the
  headless binary and the tonic shell.
- `shared/config` — `Settings` and its typed edits, the settings store, `Source`, `MusicService`, `MusicServer`.
- `daemon/reader` — domain models (`Track`, `Album`, `TrackId`), scanner, tag
  IO. Daemon-side: what crosses to a frontend is `api::TrackInfo`.
- `daemon/db` — SQLite backend, `ReadStore` / `Storage`, migrations.
- `daemon/server` — `MediaSource` backends, sync, cover resolution.
- `apps/legacy/hooks` — Dioxus data hooks over the API (queries, player controller,
  artwork, sources).
- `apps/legacy/pages`, `apps/legacy/components`, `apps/legacy/kopuz_route` — UI + routing.
- `apps/kopuz/` — the native GPUI frontend: `app` (the `kopuz` binary) and
  its feature crates, a socket-only client. `vendor/gpui-pre-linux` is its
  patched gpui platform crate, applied through the root `[patch.crates-io]`.
- `daemon/player` audio · `daemon/radio` · `daemon/scrobble` · `daemon/discord-presence`
  · `apps/common/i18n` · `shared/utils` (`CoverUrl`, image-URL builders; frontend utilities do not link SQLite).
- `apps/legacy/kopuz/android-src/` — Kotlin media-session classes its `build.rs` copies into the Android build.
- `packaging/` (flatpak / AUR / nix) · `scripts/` (codegen + vendor helpers).

## Issues and PRs

The bar for every issue and PR is the AI Policy in `CONTRIBUTING.md`. On top
of it, an agent:

- opens an issue or PR only when its human explicitly asks for one;
- opens PRs as drafts; the human marks them ready for review;
- keeps a PR to one behavior change, and puts a refactor the change needs in
  its own PR lower in a stack;
- fills the template as given: no added, removed or renamed headings, and no
  subheadings;
- writes Why and What Changed as behavior in a few sentences, never a
  file-by-file walk through the diff;
- turns a Testing cell from ❎ to ✅ only for a check it ran, with the
  screenshot or recording attached;
- makes follow-up commits only when its human asks, and never replies to
  reviewers;
- writes commit subjects in the scoped format from `CONTRIBUTING.md`
  (`crate/module: description`, `various:` across crates), never the
  conventional `fix(scope):` / `feat:` / `perf(scope):` form;
- creates no new branch: it commits on the branch its human is on, or the
  existing branch of the PR it is updating, and asks before starting one;
- never adds an AI `Co-authored-by:` trailer or a "Generated with …" footer to
  a commit, issue or PR. The AI usage checkbox is the only disclosure.
