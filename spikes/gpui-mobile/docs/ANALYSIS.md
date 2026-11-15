# GPUI on Android: analysis for the gpui-mobile spike

Read-only survey of branch `gpui-mobile-spike` (base `73c2b8a66`). All paths are from the repo root.
No builds were run. Line numbers point at the code as it was read.

## 1. How each frontend reaches the daemon

### 1.1 GPUI app (`apps/kopuz/link`)

- **Address selection** happens in `apps/kopuz/app/src/main.rs:74-84` (`daemon_address`):
  1. `KOPUZ_DAEMON_ADDR=host:port` gives `link::Address::Tcp(host)`, and the app never starts its own daemon.
  2. `KOPUZ_SOCKET=/path` gives `Address::Socket(path)`, again with no self-start.
  3. Otherwise it uses `link::default_socket_path()` (= `proto::default_socket_path`, `shared/proto/src/lib.rs:35-59`):
     `directories::BaseDirs` runtime dir, falling back to the cache dir, then `/<flavor>/kopuzd.sock`. `KOPUZ_RUNTIME_DIR` overrides the
     directory (`:38`). Here `start_own = true`.
- **Transport** is `client::GrpcApi` (`apps/common/client/src/lib.rs`). `Address` is defined at `:39`, `GrpcApi::connect_tcp(addr)` at `:83`
  (tonic, `http://{addr}`, lazy), and `GrpcApi::new(path)` at `:97` (Unix socket through `tokio::net::UnixStream`, `:56`; named pipe on Windows).
  Both connect lazily and never block. There are no credentials: socket mode is the access control (`:8-15`).
- **Runtime**: `link::init(address, start_own, cx)` (`apps/kopuz/link/src/lib.rs:210`) builds its own multi-thread tokio runtime
  with 2 workers named `kopuz-link` (`:212-217`). It stores `Link { api: Arc<dyn KopuzApi>, runtime handle, gate }` and
  a `Connection` entity as a GPUI `Global` (`:237-242`). The runtime is kept alive in `GlobalLink._runtime`.
- **Calls**: `Link::call(|api| async { api.x().await }) -> Call<T>` (`:120`) runs on the link runtime and can be awaited from a
  GPUI task. `Link::fire(cx, what, f)` (`:154`) is fire-and-forget and logs failures. `link::call_once` (`:189`) is a one-shot
  call on a current-thread runtime, for processes that open no window. A **gate** (`:47-76`) refuses every call
  until the supervisor reports `Connected`.
- **Supervisor** (`Connection::new`, `:276`):
  1. Probes `api.player_state()` with a 5 s timeout (`:284`).
  2. If that fails and the socket is its own, it spawns kopuzd (`start_daemon`, `:438`, 3 tries, 60 s wait).
  3. It then checks `api.daemon_status().proto_revision` against `api::WIRE_REVISION` (=3, `shared/api/src/lib.rs:114`) and replaces an older daemon (`replace.rs`).
  4. It emits `ApiEvent::Resync` and relays `api.events()` into GPUI as `EventEmitter<ApiEvent>` on `Connection` (`relay_events`, `:513`).
  5. When the stream ends it goes back to `Connecting`.
- **Spawning kopuzd**: `daemon_process.rs:9` `spawn_daemon` runs `std::process::Command` with `--socket <p> --exit-with-app` and
  `process_group(0)`. The program is `$KOPUZD`, else `$APPIMAGE --daemon`, else `kopuzd` beside the exe, else `PATH` (`:37-55`).
  `run_daemon_flag` (`:63`) lets the app binary exec as kopuzd. **None of this applies on Android.**
- Other desktop-only pieces in link are `relay.rs` (a deep-link relay socket, used at `main.rs:170`) and `replace.rs` (which stops
  an old daemon using the Linux `libc` peer pid or the Windows `GetNamedPipeServerProcessId`). `instance.rs` holds a single-instance file lock (`main.rs:109`).
- The kopuzd binary flags are `--socket <path>`, `--listen <ip>:<port>` and `--exit-with-app` (`daemon/kopuzd/src/main.rs:14-17`).

### 1.2 Legacy Dioxus app (`apps/legacy/kopuz/src/backend.rs`)

- **In-process core.** `backend::start(CoreArgs)` (`:31`) spawns a `kopuz-core` thread with its own multi-thread runtime. It calls
  `daemon::boot::assemble(&args)` (`:48`), stores the `Core` in a `OnceLock` (`:83`), and then parks in `serve_socket` (`:88`).
  The UI reaches it through `backend::api() -> Arc<dyn api::KopuzApi>` (`:183`), which is `core.api` (the in-process LocalApi).
  So the legacy app uses the **same `KopuzApi` trait** as the GPUI app, without gRPC.
- **Desktop** (`#[cfg(not(android))]`, `:106-166`): it also serves the core on `kopuzd::default_socket_path()` (`kopuzd::listen`)
  and on the configured TCP listen address (`kopuzd::listen_tcp`). That is how `kopuzctl` and other frontends attach.
- **Android** (`:168-175`): it **does not serve any socket**. `kopuzd` is listed as a desktop-only dependency
  (`apps/legacy/kopuz/Cargo.toml:24-33`). The Android deps are `dioxus/mobile`, `axum` (a localhost artwork HTTP server,
  `backend.rs:55-68`), `rustls-platform-verifier`, `jni 0.22` and `ndk-context` (`Cargo.toml:35-44`). The kopuzd lib's own deps
  (tonic, tokio net/signal, tracing) do not look Android-hostile. It is excluded by choice, not because it cannot build.
- **Startup order on Android** (`apps/legacy/kopuz/src/main.rs:300-310`): `player::systemint::init()` (captures the JVM, caches the
  classloader, calls `MediaSessionHelper.init`), then `backend::start(CoreArgs::default())`, then the Dioxus mobile launch.
- **Data and cache folders**:
  - DB and settings: `db::config_dir()` (`daemon/db/src/lib.rs:946`) uses `android_files_dir()` (`:960`), which calls JNI
    `Context.getFilesDir()` through `ndk_context::android_context()`. `boot.rs:115-123` derives `data_dir`, `plugins/` and settings from the DB path.
  - Caches still use `config::flavor::project_dirs()` and fall back to `std::env::temp_dir()`: covers (`daemon/core/src/boot.rs:146`),
    offline_tracks (`:206`), artwork (`:279`) and artwork-uploads (`:287`). On Android `directories` resolves under `$HOME`, which is not writable.
    **Open:** it is not confirmed where these land on a device. The spike only needs the DB.
  - The default music folder is hard-coded to `/storage/emulated/0/Music` (`shared/config/src/lib.rs:1361`). `systemint/android.rs:323,336`
    also exposes `get_files_dir` and `get_android_music_dir` over JNI.
- **Media session and background playback**:
  - Audio is produced by `cpal` 0.18 (AAudio) on the engine's own threads, with a 2048-frame buffer on Android
    (`daemon/player/src/engine/sink.rs:151`). It does not depend on the UI thread.
  - The core session pushes now-playing information through `player::systemint::update_now_playing` (`daemon/core/src/session/mod.rs:2000`,
    `daemon/player/src/systemint/android.rs:503`). That calls the static Kotlin `MediaSessionHelper.updateNowPlaying`, which builds
    an `android.media.session.MediaSession` (`MediaSessionHelper.kt:119`) plus a notification, then calls
    `MusicService.update(context, playing)` (`:375`).
  - `MusicService.kt` is a `foregroundServiceType="mediaPlayback"` service with `stopWithTask=false`. It calls `startForeground` (`:47`)
    and holds a `PARTIAL_WAKE_LOCK` while playing (`:53,70`). It is started with `startForegroundService` (`:19-30`).
  - Notification buttons go to `MediaReceiver.kt`, then the JNI function `Java_moe_kopuz_kopuz_MediaReceiver_nativeOnAction`
    (`systemint/android.rs:757`), then `dispatch_event`, then the background handler. That handler is installed **by the legacy UI**
    (`apps/legacy/hooks/src/use_player_task.rs:87-100`). `daemon/core/src/os_media.rs:155` (`spawn`) is a **no-op on Android**.
    A GPUI port must install its own `set_background_handler` that maps `SystemEvent` to `PlayerCommand`, as the macOS
    branch of `os_media.rs:32-52` does.
  - `MainActivity.kt:27` extends **`WryActivity`** and overrides `onPause` to keep the WebView running (`:145-157`).
    `MusicService.update` calls `MainActivity.requestBatteryExemptionOnce()` (`MusicService.kt:20`). These parts are tied to Wry and
    must be rewritten for a NativeActivity or GameActivity host.
- **Manifest**: the repo has no AndroidManifest.xml. `dx` generates the Gradle project, and `apps/legacy/kopuz/build.rs` patches it
  at build time through `WRY_ANDROID_KOTLIN_FILES_OUT_DIR` (`:294-365`):
  - adds permissions (`:22-33`): INTERNET, POST_NOTIFICATIONS, FOREGROUND_SERVICE(_MEDIA_PLAYBACK), WAKE_LOCK, battery-opt,
    READ/WRITE_EXTERNAL_STORAGE, READ_MEDIA_AUDIO/IMAGES;
  - adds `<service MusicService>`, `<receiver MediaReceiver>` and `<activity LoginActivity>` (`:113-128`);
  - adds R8 keep rules, a cleartext network config, icons, and versionCode/SDK patches.
- **Dioxus.toml** has no `[android]` section. `bundle.identifier = "moe.kopuz.kopuz"` (`Dioxus.toml:9`) is kept on Android, while desktop
  uses `moe.kopuz.legacy` (`:22-30`). Builds are `just android-patch` (`Justfile:62`) and `packaging/android/build-apk.sh:36`
  (`dx build --platform android --target aarch64-linux-android`), with a 16 KB page-alignment check.

### 1.3 Minimum API for "library list + play"

These are all methods of `api::KopuzApi` (`shared/api/src/lib.rs:731`, `#[async_trait]`, object-safe as `Arc<dyn KopuzApi>`).
`GrpcApi` and the core's LocalApi both implement them.

```rust
// shared/api/src/lib.rs
async fn tracks(&self, filter: TrackFilter, page: Page) -> Result<TrackPage, ApiError>;        // :210 (LibraryApi)
async fn set_queue(&self, req: SetQueueRequest) -> Result<CommandAck, ApiError>;              // :182 (PlayerApi)
async fn player_command(&self, cmd: PlayerCommand) -> Result<CommandAck, ApiError>;           // :174
async fn player_state(&self) -> Result<PlayerState, ApiError>;                                // :172
async fn queue_edit(&self, edit: QueueEdit) -> Result<CommandAck, ApiError>;                  // :184 (Jump/JumpPhysical)
fn events(&self) -> EventStream; // BoxStream<'static, ApiEvent>                               // :723, :133
```

- Types:
  - `TrackFilter` (`library.rs:131`): `Default` lists everything.
  - `Page { offset: u32, limit: u32 }` (`library.rs:116`): the default limit is `DEFAULT_PAGE_LIMIT`, and `store::all()` uses `u32::MAX` (`apps/kopuz/store/src/lib.rs:93`).
  - `TrackPage { total, offset, items: Vec<TrackInfo> }` (`library.rs:216`).
  - `TrackInfo { key, title, artist, album, duration_ms, .. }` (`library.rs:25`).
- Play a track the way the GPUI app does (`apps/kopuz/store/src/actions.rs:34-44`, `play_tracks`):
  ```rust
  api.set_queue(SetQueueRequest { mode: QueueMode::Replace,
      context: QueueContext::Tracks { keys }, start_index: Some(i), shuffle: None }).await
  ```
  `QueueMode` is at `queue.rs:4` and `QueueContext` at `queue.rs:15` (also `Filter { filter }`, `Album { id }` and others). `SetQueueRequest` is at `queue.rs:57`.
  Toggle with `player_command(PlayerCommand::Toggle)` (`player.rs:133`).
- Events (`shared/api/src/events.rs:110`): `PlayerState(Box<PlayerState>)`, `PlayerPosition{..}`, `QueueChanged{rev,length,index}`,
  `LibraryInvalidated { table: Table }` (`:130`; `Table::Tracks` at `:11` means "re-run `tracks()`"), and `Resync` (it reattached, so re-read everything).
  The GPUI app consumes them by subscribing to `Link::connection(cx)` (`EventEmitter<ApiEvent>`).
- Smallest Android client with no `link` crate: build `GrpcApi::connect_tcp("127.0.0.1:PORT")` inside a tokio runtime, keep it as
  `Arc<dyn KopuzApi>`, run calls on that runtime, and send results to GPUI over a `futures::channel`.

## 2. What the GPUI app relies on

### 2.1 Crates

- `gpui-kit = "0.6.4"` (`Cargo.toml:174`). It pulls `gpui-pre 0.3.5`, `gpui-component 0.6.4`, `gpui-base`, `gpui-kit-assets`,
  `gpui-pre-platform` and `gpui-pre-web` (Cargo.lock). `gpui-pre-platform` dispatches to `gpui-pre-macos` (the `gpui-pre-apple` Metal backend),
  `gpui-pre-linux` (vendored, `Cargo.toml:224-225`; wayland/x11/ashpd/notify-rust/oo7/zed-scap) and `gpui-pre-web`. Linux uses `gpui-pre-wgpu 0.3.5`.
  No feature flags are set on gpui-kit.
- `gpui-component 0.6.4` depends on `core-text`, `objc2-app-kit` and `windows 0.58` (target-gated), plus `notify`, `lsp-types`,
  `markdown`, `ropey` and `resvg 0.45`.
- App-level: `image 0.25` (jpeg/png/webp, `Cargo.toml:37`), `tray-icon 0.21` (mac/win only, `apps/kopuz/app/Cargo.toml:88-89`),
  `tikv-jemallocator` (`:91-92`), `winres`. In `now_playing`: `objc2*`, `block2`, `dispatch2`, `core-video` and `core-foundation` for
  AVFoundation video, all gated to macOS. `settings_interface` uses `fontdb`. `profile` uses `tracing-chrome` and `tikv-jemalloc-ctl`. `store`, `profile` and
  `settings_playback` use `directories`. `rfd` is in the workspace but no GPUI crate uses it (they use gpui `prompt_for_paths`).
- **Version clash:** `gpui-pre-mobile` (crates.io name of longbridge/gpui-mobile) is not in Cargo.lock. Its published releases
  pin a different `gpui-pre` (0.3.4, later 0.3.6 after upstream PR #48) from this workspace's 0.3.5. With two gpui-pre copies,
  gpui-kit widgets will not interoperate with a gpui-pre-mobile window. The spike should depend only on `gpui-pre` +
  `gpui-pre-mobile` at whatever version mobile pins, and **must not pull `gpui-kit` or the `link` crate (`link` depends on gpui-kit)**.
  Unifying versions is a later task.

### 2.2 GPUI features used (counts of `grep -rn` hits in `apps/kopuz`, by crate)

| Feature | Use sites | Where | Android |
|---|---|---|---|
| `open_window` / `WindowOptions` | 7 / 12 | app/main.rs:146,268; now_playing/window.rs:103 (mini/overlay); palette | **variant**: one full-screen window |
| `TitlebarOptions`, custom frame | 56 hits | topbar/src/frame.rs (mac traffic lights, CSD), app | drop |
| `set_menus` (app menu) | 1 | app/src/menus.rs:58 (macOS, main.rs:206) | drop |
| Tray | 1 crate | app/src/tray.rs:108 (mac/win, main.rs:220) | drop (use media notification) |
| Global hotkeys | 0 | — | n/a |
| `bind_keys` / actions | 35 / 6 | app/main.rs:30-71, commands keymap, library | portable (mostly unused on touch) |
| Text input (`InputState`/`Input`) | 114 | components(7 files), settings_sources, metadata_dialog, library | **variant**: needs soft keyboard + IME in mobile platform (open upstream) |
| Drag and drop (internal `on_drop<T>`) | 20 | components/reorder.rs, track_header.rs, now_playing/queue_view.rs, playlists_panel | variant (long-press); no OS file drops used |
| File dialogs `prompt_for_paths`/`prompt_for_new_path` | 6 | library/playlists.rs:199, settings_sources/source_tools.rs:340, components/folders.rs:43, store/artwork_edit.rs:27, store/picture.rs:217, settings_playback/logs.rs:66 | **variant** (SAF intents) or drop |
| Clipboard | 6 | store/actions.rs:162,189, store/picture.rs:170-172 (image), settings_* | variant (ClipboardManager over JNI) |
| Deep links `on_open_urls`, `on_reopen` | 1 / 1 | app/main.rs:127,133 + link/relay.rs | variant (intent filter) / drop |
| `cx.open_url` | 1 | status/src/update.rs:106 | variant (intent) |
| Multiple windows | 3 kinds | main, now_playing mini/overlay, palette preview | drop extras |
| Asset source | 1 | `with_assets(gpui_kit::assets::AllAssets)` main.rs:121 | portable if gpui-kit is portable |
| Fonts | 4 | settings_interface/font.rs:56 `add_fonts`, `fontdb` system scan | variant (bundle fonts; mobile uses cosmic-text) |
| SVG / icons | 17 | components, primitives (`IconName`) | portable |
| Images (`img`, `RenderImage`) | 46 hits | store/artwork.rs, primitives, components | portable (artwork bytes come from `ArtworkApi::artwork`, lib.rs:414) |
| Animation | 12 | components, discover, page_home | portable |
| `uniform_list` | 8 sites | library/tracks.rs:130, components/track_list.rs:567, grid.rs:157, queue_view.rs:389, palette, status, playlists_panel | **portable — the spike's list** |
| `list`/`ListState` | 2 | page_radio/src/lib.rs:107,385 | portable |
| gpui-component `Table` | 48 hits | settings_sources, store, discover, pages | portable via gpui-kit |
| gpui-component `VirtualList` | 0 | — | n/a |
| Widgets | button 65, input 25, menu 19, tooltip 13, slider 5, popover/dialog/notification (toasts, status/toasts.rs:9) | | tooltip/menu need touch variants |

### 2.3 Platform calls outside gpui

| Call | Where | Android |
|---|---|---|
| spawn kopuzd (`std::process::Command`) | link/daemon_process.rs:9,68 | drop (connect over TCP or host the core in-process) |
| Windows named pipes / `windows` crate | link/replace.rs:97-106, relay.rs:57 | desktop-only |
| Linux `libc` peer pid | link/replace.rs (`cfg linux`) | desktop-only |
| macOS objc2/AVFoundation video | now_playing/src/video_mac.rs, video.rs (`cfg macos`) | desktop-only (non-mac fallback exists) |
| `directories::BaseDirs` | store/layout.rs:915, store/picture.rs:195, profile/lib.rs:78, settings_playback/logs.rs:60 | **variant**: returns None or non-writable path; use `getFilesDir` |
| Notifications / MPRIS | none in the GPUI app (the daemon does OS media); toasts are in-window | n/a; Android media session is daemon-side (§1.2) |
| Auto-updater | store/updates.rs (`UpdateStatus` from the daemon) + status/update.rs | drop (stores handle updates) |
| jemalloc | app/main.rs:87-95, profile | probably portable, but not needed for the spike |

## 3. Reuse of `apps/kopuz` crates

Dependency facts (from each `Cargo.toml`):
- **Every UI crate depends on `gpui-kit`.** No crate other than `app`, `link` and `now_playing` adds a target-specific desktop
  dependency. The desktop coupling is in **gpui-kit itself** (via gpui-pre-platform) and in a few code paths, not in crate dependencies.
- `link` is the root of the graph: `store` depends on `link`, `theme` depends on `store`, and `components` depends on `store`, `link`, `primitives`, `theme` and `profile`.

| Crate (LOC) | Desktop-only deps / code | Mobile verdict |
|---|---|---|
| shared `api`, `config`, `proto`, `apps/common/client` | none; proto builds with `protox`, no system protoc (`shared/proto/build.rs:1-12`) | **reuse unchanged** (spike uses these) |
| `link` (1056) | `tokio process`, `daemon_process.rs`, `relay.rs`, `replace.rs`, `windows`/`libc` | needs an Android variant (`Address::Tcp`, no spawn, no relay); `init()` itself is fine |
| `store` (10188) | `directories` (layout.rs:915, picture.rs:195), `prompt_for_*`, clipboard, updater | reuse after gpui-kit works; guard 3-4 call sites |
| `theme` (1588) | none (material-colors, store) | reuse |
| `primitives` (2716) | none (fluent-bundle, i18n) | reuse |
| `commands` (2505) | keymap only | reuse (bindings inert on touch) |
| `components` (20134) | folders.rs:43 prompt; hover/tooltip/drag UX | reuse; touch tweaks |
| `library` (4446) | playlists.rs:199 prompt | reuse (main candidate after the spike) |
| `profile` (497) | `directories`, `tracing-chrome`, jemalloc-ctl | variant (log dir) |
| `settings_interface` (584) | `fontdb` system fonts | variant |
| `settings_playback` (1123) | `directories::UserDirs`, save dialog | variant |
| `now_playing` (5027) | objc2/AVFoundation (mac), extra windows | reuse panel; drop mini/overlay windows and mac video |
| `topbar` (1342) | titlebar / window frame | replace with a mobile header |
| `app` (1798) | tray-icon, menus, single-instance, deep-link relay, kopuzd flag | **replace** with a mobile entry point |
| discover, page_*, playlists_panel, palette, status, overlays, onboarding, metadata_dialog, shortcuts, debug_overlay, settings_appearance/sources | gpui-kit only | reuse in principle once gpui-kit renders on mobile |

## 4. Recommendation for the spike

1. A new crate under `spikes/gpui-mobile/` (cdylib) using `gpui-pre` + `gpui-pre-mobile` only, plus `api` and `client` from the workspace.
   No gpui-kit, no `link`, no `store`.
2. Connect over TCP: on the Mac run `kopuzd --socket <tmp>.sock --listen 127.0.0.1:<PORT>` and `adb reverse tcp:<PORT> tcp:<PORT>`.
   The app uses `GrpcApi::connect_tcp("127.0.0.1:<PORT>")`, from a constant or an intent extra, since Android has no env vars.
   Everything stays on localhost.
3. UI: one `uniform_list` of `TrackInfo` from `api.tracks(TrackFilter::default(), Page{offset:0,limit:500})`. On a row tap or the play
   button, call `set_queue(Replace, Tracks{keys}, start_index)`. A Toggle button calls `player_command(Toggle)`. Refresh on
   `LibraryInvalidated{Tracks}` and `Resync` from `api.events()`.
4. Later: host the core in-process like `backend.rs` does, using `daemon::boot::assemble`, `getFilesDir` and the media-session Kotlin
   (rewritten off `WryActivity`). Install a `systemint::set_background_handler`, because `os_media::spawn` is a no-op on Android.

## 5. Open items

- Whether `gpui-kit` 0.6.4 builds for `aarch64-linux-android` at all. `gpui-pre-platform` probably selects no backend there, and
  `gpui_kit::application()` is the app's only entry point (`main.rs:121`). This needs a check build.
- Aligning versions between gpui-pre-mobile's `gpui-pre` and the workspace's 0.3.5.
- Upstream gpui-pre-mobile notes that IME composition, accessibility and lifecycle hooks are incomplete, and there are reports of slow frame pacing on emulators.
- On Android, the daemon caches resolve through `directories` or `temp_dir` (`boot.rs:146,206,279,287`).
- Why the legacy app treats `kopuzd` as desktop-only (`backend.rs:168-171`). Serving the in-process core on a Unix socket in
  `getFilesDir` would let a GPUI UI keep using `GrpcApi`/`link` unchanged.

Sources (web): https://docs.rs/gpui-pre-mobile/latest/gpui_mobile/ ,
https://github.com/longbridge/gpui-mobile , https://github.com/itsbalamurali/gpui-mobile/pull/48 ,
https://github.com/Bombatomica64/Gpui-android
