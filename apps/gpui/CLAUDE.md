# Agent Development Guide

`kopuz-gpui` is a GPUI frontend for the Kopuz music daemon. It talks to the
daemon over a socket and nothing else: no database, no media source, no
credentials, no audio. The Dioxus frontend it is porting lives in
`../kopuz` (upstream `Kopuz-org/kopuz`), and `docs/PARITY.md` is the
feature-by-feature checklist that port is measured against.

## Commands

Everything runs inside the dev shell: `nix develop --command <cmd>`. A bare
`cargo` fails on this machine.

- **Check your crate:** `nix develop --command cargo clippy -p <crate> --all-targets -- -D warnings`
- **Whole workspace:** `nix develop --command cargo clippy --workspace --all-targets -- -D warnings`
- **Format:** `nix develop --command cargo fmt --all`
- **Run:** `KOPUZ_SOCKET=$XDG_RUNTIME_DIR/kopuzd-test.sock nix develop --command cargo run`
  with a daemon started as `~/kopuz/target/debug/kopuzd --socket $XDG_RUNTIME_DIR/kopuzd-test.sock`.
  `KOPUZ_DAEMON_ADDR=host:port` attaches to a `--listen` daemon instead.

## Architecture

Dependencies point one way: `app` → feature crates → `components` →
`primitives` → `store` → `link` → `kopuz-api`.

- `crates/link` — the daemon seam. `Link::call` runs an API call on the tokio
  runtime tonic needs and hands back a future GPUI can await; `Connection`
  supervises the socket and re-emits every `ApiEvent`, plus `Resync` on each
  attach.
- `crates/store` — app-wide models, each `Model::global(cx) -> Entity<Model>`:
  `PlayerModel` (+ `Playhead` for the ticking position), `QueueModel`,
  `ConfigModel`, `SourcesModel`, `FavoritesModel`, `PlaylistsModel`,
  `DownloadsModel`, `JobsModel`, `Navigator`, `Chrome`, `Notices`.
  `Query<T>` is the read that keeps itself fresh; `store::actions` holds the
  play/queue/share commands; `store::artwork_source` feeds `img()`.
- `crates/components` — every shared component, each defined once: `Cover`,
  `Tile`/`TileRow`, `TrackList`/`TrackRow`/`TrackHeader`, `PageHeader`,
  `EmptyState`, `Loading`, `SchemaForm`, `MenuButton`, `PickList`, menus, dialogs.
  It re-exports `primitives`, so a feature crate depends on `components` alone.
  Before drawing something a second page also draws, put it here instead.
- `crates/primitives` — what components are drawn with: `style` tokens,
  `motion`, `focus`, `icons`, `format::{t, t_with, duration}`, `scroll`,
  `skin`. It cannot reach a component, and the compiler holds that line.
- `crates/theme` — Kopuz's 13-token palettes projected onto the component theme.
- `crates/page_*`, `crates/sidebar`, `player_bar`, `side_panel`, `fullscreen`,
  `quick_search`, `overlays`, `metadata_dialog` — one feature each. The app
  shell composes them; features never depend on each other or on `app`.

## Conventions (enforced)

- **Comments: one line, maximum.** Write one only for a non-obvious "why".
  Never restate the code. This applies to doc comments too.
- Diagnostics via `tracing`; `println!`/`eprintln!` are clippy-denied.
- Never invent a GPUI or gpui-kit API: grep the real signature under
  `~/.cargo/registry/src/*/gpui-{pre,component,base,kit}-*/src`. The vendored
  skills in `.claude/skills/gpui-kit` are the reference for patterns.
- Read theme values from `cx.theme()`. No raw hex, `rgb()`, or `hsla()` in
  feature code, and prefer rem-scale helpers (`p_2()`, `text_sm()`, `w_64()`)
  over `px(...)` except for genuinely measured geometry such as row heights.
- Repeated elements need stable domain ids, never a list index that can move.
- Every user-visible string goes through `components::format::t` with a key that
  exists in `../kopuz/crates/i18n/locales/en.ftl`. If no key fits, use the
  literal English the Dioxus app used and leave a `TODO(i18n)`.
- Gate every affordance on `SourcesModel::global(cx).read(cx).capabilities()`,
  the way the checklist says. Never branch on a service name.
- Prefer real error handling over `unwrap()`/`expect()` outside tests.

## Issue and PR Guidelines

- Never create an issue or a PR.
