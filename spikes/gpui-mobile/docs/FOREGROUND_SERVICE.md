# Running the daemon on Android as a foreground service (plan)

Status: **plan only, nothing here is implemented.** In the spike the daemon runs on the Mac
(`kopuzd --listen 127.0.0.1:50777`). The phone or emulator reaches it through
`adb reverse tcp:50777 tcp:50777`, and the APK has no service, no media session and no Java.
This document covers what a real GPUI app needs so that playback keeps going after the user
leaves the app, turns the screen off or swipes the app away.

Paths are from the repo root. "Legacy" means `apps/legacy/kopuz`. Its Kotlin is in
`apps/legacy/kopuz/android-src/`, and `apps/legacy/kopuz/build.rs` copies it into the Gradle project
that `dx` generates and patches the manifest there (`INSIDE_APPLICATION`, `PERMISSIONS`, `KEEP_RULES`).

## 1. How legacy does it today

- `backend::start` (`apps/legacy/kopuz/src/backend.rs:31`) runs `daemon::boot::assemble` on a
  `kopuz-core` thread with its own multi-thread tokio runtime. The UI calls `backend::api()`, the in-process
  `LocalApi`. On Android `serve_socket` just parks (`backend.rs:168-175`). No socket is served there, because
  `kopuzd` is a desktop-only dependency and nothing else on the device attaches to the core.
- Audio (cpal/AAudio) runs on engine threads and does not depend on the UI thread.
- The core calls `player::systemint::update_now_playing`, which goes through JNI to the static
  `MediaSessionHelper.updateNowPlaying`. That builds a framework `android.media.session.MediaSession`
  (not `MediaSessionCompat`, and no AndroidX) and a `Notification.MediaStyle` notification, then calls
  `MusicService.update(context, playing)`.
- `MusicService` (`foregroundServiceType="mediaPlayback"`, `stopWithTask="false"`) calls `startForeground`
  on every `onStartCommand` and holds a `PARTIAL_WAKE_LOCK` only while playing. It returns `START_STICKY`.
- Notification buttons are broadcasts to `MediaReceiver`. Session callbacks (lock screen, Bluetooth, Wear)
  and the audio-focus and becoming-noisy listeners all call `MediaReceiver.nativeOnAction(String)`. That is
  `Java_moe_kopuz_kopuz_MediaReceiver_nativeOnAction` (`daemon/player/src/systemint/android.rs:757`), which
  calls `dispatch_event(SystemEvent)` and then the handler that **the legacy UI** installs
  (`apps/legacy/hooks/src/use_player_task.rs:87-100`). `daemon/core/src/os_media.rs:155` is a no-op on Android.
- Every JNI call takes its `Context` from `ndk_context::android_context()`, both in systemint and in
  `db::config_dir()`, which calls `getFilesDir()` (`daemon/db/src/lib.rs:960`). Legacy has no Wi-Fi lock.

## 2. Process model

### (A) Core in the app process, kept alive by a mediaPlayback service (legacy model)

The GPUI cdylib boots the core on its own thread, exactly as `backend.rs` does. The UI talks to the
`LocalApi` directly, and a `MusicService` keeps the process at foreground priority while playing.

- Pro: proven by legacy, one process, no IPC, the smallest Java layer.
- Con: `link` speaks gRPC to an address. A mobile UI needs a new in-process `link` entry, for example
  `link::init_with_api(Arc<dyn KopuzApi>)`, or it must serve the core on a socket to itself.
- Con: `ndk_context` belongs to android-activity, which initializes it with the **Activity** and releases it
  when `android_main` returns (check against android-activity 0.6). After the activity is destroyed, every
  `ndk_context::android_context()` call from the core (now-playing updates, `getFilesDir`) uses a released
  context or panics. All of those call sites would have to move to an Application context that the core holds itself.
- Con: after the activity is destroyed while music plays, the process stays alive and the next launch runs
  `android_main` again in the same process. Whether gpui-pre and gpui-pre-mobile can build a second
  `Application` in one process is untested. A UI crash or a GPU driver crash also stops the music.
- Con: the foreground service protects the whole process, including GPUI's atlas and wgpu buffers, so the
  backgrounded app is a large process that the low-memory killer will not reclaim.

### (B) Core in a separate `android:process=":daemon"` service, UI over a Unix socket (recommended)

`KopuzDaemonService` (declared with `android:process=":daemon"`) loads the same `.so` and calls
`nativeStart(appContext, filesDir, cacheDir)`. Rust boots the core and serves it with `kopuzd`'s
existing `bind_socket` and `serve` on `<filesDir>/kopuzd.sock`. The GPUI activity runs in the main
process and calls `link::init(Address::Socket(path), start_own = false, cx)`, the same code path as
`KOPUZ_SOCKET` on desktop.

- Pro: Android gets the desktop architecture. `link`, `client::GrpcApi::new(path)` and the event relay stay
  unchanged, and `link`'s spawn, replace and relay paths are simply not used with `start_own = false`. This
  needs a check build: `replace.rs` is gated on `target_os = "linux"`, which is false on Android.
- Pro: the socket lives in the app's private directory, so the UID sandbox does what the 0600 mode does on
  desktop. Both processes share the UID. The path (about 45 bytes) is under the 108-byte `sun_path` limit.
- Pro: the daemon process owns its own Context. `nativeStart` calls `ndk_context::initialize_android_context`
  with the Application context, so `db::config_dir()` and all of systemint work **without edits**.
  Nothing in that process ever runs `android_main`.
- Pro: the foreground service protects only the small daemon process. The UI process can be killed in the
  background and restarts cleanly, since every launch is a fresh process and `link` resyncs (`ApiEvent::Resync`).
  A UI crash does not stop playback.
- Con: two processes (about +20 MB RSS for the second runtime) and gRPC over a Unix socket for each call.
  That is negligible next to rendering, and desktop already pays it.
- Con: the media Kotlin and anything it calls must live in `:daemon`. Calls that legacy makes into the
  Activity (`MainActivity.requestBatteryExemptionOnce`) become an intent or a UI-side prompt.
- Abstract socket instead of a file: no file to clean up, but abstract names have **no file permissions**.
  Any app could connect unless the server checks `SO_PEERCRED` against its own UID, and `GrpcApi` would need
  an abstract-address connector. Use the file in `getFilesDir()`.
- Never serve `--listen` TCP on a device. Every app on the phone can reach 127.0.0.1.

### (C) `kopuzd` as a separate executable: not recommended

Android does not support starting plain executables from an app. Targets of API 29 and above cannot
`execve` files from writable app storage (W^X). The only workaround is to ship the binary as `lib*.so` and
exec it from `nativeLibraryDir`. Such a process is not an Android component. It cannot be a foreground
service, has no `Context` or JNI (so no MediaSession, audio focus or `getFilesDir`), and the Android 12+
phantom-process killer counts it and kills it when it uses CPU in the background. It would die exactly when
playback needs it.

**Recommendation: B.** It costs about two more days than A (section 6) and removes A's three risks:
ndk_context ownership, re-entering `android_main` and the large protected process. It also keeps one client
code path for desktop and mobile. A stays the fallback if two processes turn out too costly on low-end phones.

## 3. Service lifecycle

1. **Start.** The activity calls `startService` and `bindService(BIND_AUTO_CREATE)` on `KopuzDaemonService`
   in `onStart`, which is allowed while it is visible. The service's `onCreate` loads the library and calls
   `nativeStart`, and the socket appears. `link`'s supervisor retries until it connects. The service is
   started but **not** foreground yet: no notification before the first track plays.
2. **First play.** The core's now-playing update reaches `MediaSessionHelper.updateNowPlaying`, which builds
   the MediaStyle notification and calls `startForeground` from inside the service. It must happen within 5 s
   of any `startForegroundService`, or the system throws `ForegroundServiceDidNotStartInTime`. Legacy already
   orders this correctly (`MusicService.kt:44-48`).
3. **Android 14+ (API 34).** The manifest declares `foregroundServiceType="mediaPlayback"`, with
   `FOREGROUND_SERVICE` and `FOREGROUND_SERVICE_MEDIA_PLAYBACK`. The call becomes
   `startForeground(id, notif, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)` on API 29+.
   mediaPlayback has no time limit (the Android 15 six-hour limit applies to dataSync and mediaProcessing).
4. **Android 12+ background-start limits.** An app in the background may not start a foreground service.
   The exemptions cover notification actions and media-button events through an active MediaSession. So
   the service must already be running (started, and foreground while a session is live) when those events
   arrive, and must not be stopped and restarted in between.
5. **Android 13+.** Ask for `POST_NOTIFICATIONS` from the activity. MediaSession notifications are exempt
   from it, but asking keeps the behaviour the same across OEMs.
6. **Pause.** Release the wake lock and Wi-Fi lock and stay in the foreground. After about 10 minutes paused,
   call `stopForeground(STOP_FOREGROUND_DETACH)` so the notification becomes swipeable.
7. **Stop.** If the notification is dismissed (`setDeleteIntent`) while paused, or on "stop", or when the
   queue ends: `MediaSessionHelper.stopSession` and then `stopSelf()`. The `:daemon` process may then be
   reclaimed. The next launch boots it again from the database. On `onTaskRemoved` keep playing (as legacy
   does), unless paused, in which case stop.
8. **Locks.** Keep `PARTIAL_WAKE_LOCK` while playing (legacy `MusicService.acquireWakeLock`). Add a
   `WifiManager.WifiLock` (`WIFI_MODE_FULL_HIGH_PERF`, as ExoPlayer's `WAKE_MODE_NETWORK` does) while a
   remote source (Jellyfin, Navidrome, radio) plays, so the radio does not doze mid-stream.
9. **Audio focus and becoming-noisy.** Reuse legacy as it is. `MediaSessionHelper` requests `AUDIOFOCUS_GAIN`
   with `setWillPauseWhenDucked(true)`, because cpal cannot duck. It pauses on loss and resumes after a
   transient loss, and pauses on `ACTION_AUDIO_BECOMING_NOISY`.
10. **MediaSession.** Keep the framework `MediaSession` that legacy uses. No AndroidX is needed, so the
    APK can still be built with `aapt2`, `javac` and `d8`. media3's `MediaSessionService` would add
    automatic notifications and foreground handling, media resumption on Android 13+ and a route to
    Android Auto. But it needs AndroidX, Gradle and a `SimpleBasePlayer` that mirrors the daemon's state.
    That is a later step (6.8). `MediaSessionCompat` is legacy AndroidX and is not worth adopting now.
11. **Buttons to `PlayerCommand`.** Lock screen, Bluetooth AVRCP and the notification all arrive at
    `nativeOnAction` and then `dispatch_event`. Add a `#[cfg(target_os = "android")]` branch to
    `os_media::spawn` modelled on the macOS one (`os_media.rs:32-52`): `set_background_handler` feeds an
    mpsc channel, and a task maps `Play/Pause/Toggle/Next/Prev/Seek` and the Android-only
    `Stop/ToggleShuffle/CycleRepeat` to `PlayerCommand`. The core then handles buttons with no UI at all.
    Legacy is unaffected: its hook calls `set_background_handler` later, and that call replaces the handler.
    The Dioxus-only parts of systemint (`capture_event_loop`, `set_keepalive`, the tokio waker) are not
    needed, because the core's runtime is not driven by a UI loop. `wake_run_loop` is a no-op when no looper is captured.

## 4. What the GPUI side needs

- **A small Java layer.** `android.app.NativeActivity` with `hasCode="false"` (the spike) cannot start a
  service or ask for permissions. Add `KopuzActivity extends NativeActivity` and keep android-activity's
  `native-activity` feature that gpui-pre-mobile uses. GameActivity would mean switching android-activity
  to `game-activity` and pulling in the AndroidX games AAR, for no benefit here. If IME is wanted, extend
  gpui-pre-mobile's `GpuiInputActivity` instead (check that it is itself a NativeActivity subclass).
  `KopuzActivity` asks for `POST_NOTIFICATIONS` and `READ_MEDIA_AUDIO`, starts and binds the service,
  passes the socket path into Rust, and handles `onNewIntent` from the notification's content intent
  (`launchMode="singleTask"`, as in legacy).
- **The JNI bridge.** In B, commands do **not** cross JNI. The UI uses `link`/`GrpcApi`, and only
  bootstrapping does: `KopuzActivity.nativeSetPaths(filesDir, cacheDir)` before `android_main` uses them,
  or a JNI call from Rust after `jni::init_platform`. In A, the UI would call the in-process
  `Arc<dyn KopuzApi>`, and Java would reach Rust only through `nativeOnAction`, as today.
- **Reconnect.** In B, a destroyed activity drops only the gRPC client. The next launch is a new
  process or a new `android_main`. It connects to the same socket, gets `ApiEvent::Resync` and re-reads
  the player state and the queue. If `:daemon` was reclaimed while idle, binding the service boots it again,
  and the supervisor's connect timeout covers the boot time (database open, about 1 s).
- **Data and cache paths.** `db::config_dir()` already uses `getFilesDir()` through `ndk_context`. The
  caches in `daemon/core/src/boot.rs:146,206,279,287` use `project_dirs()` and then `temp_dir()`, and both
  are wrong on Android. Add `cache_dir: Option<PathBuf>` to `CoreArgs` and pass `getCacheDir()` for covers
  and artwork. Offline tracks belong under `getFilesDir()`, because the system may empty the cache
  directory at any time. Setting `XDG_CACHE_HOME` before boot is a stop-gap.
- **TLS.** The daemon process must call `rustls_platform_verifier::android::init_with_env` (as legacy
  `main.rs:149` does) and ship `org/rustls/platformverifier/*.kt`. Otherwise HTTPS sources fail.

## 5. Reuse of the legacy Kotlin (`apps/legacy/kopuz/android-src/`)

| File | Reuse | Changes |
|---|---|---|
| `java/moe/kopuz/kopuz/MediaReceiver.kt` | verbatim | none. Keep the package `moe.kopuz.kopuz`, because the JNI symbol name and `find_app_class` depend on it. The applicationId may differ |
| `java/moe/kopuz/kopuz/MediaSessionHelper.kt` | nearly verbatim | move `hasMediaPermission`/`requestPermissions` to `KopuzActivity`. Add `setDeleteIntent` and a Wi-Fi lock hook. It needs the `R.drawable.kopuz_ic_*` resources that `build.rs` generates (port them as XML drawables) and an `R` in package `moe.kopuz.kopuz` |
| `java/moe/kopuz/kopuz/MusicService.kt` | mostly | drop the `dev.dioxus.main.MainActivity` import and battery call. Use typed `startForeground` on 29+. In B, add `nativeStart` in `onCreate`, `onBind`, and the paused-timeout and dismiss handling |
| `java/org/rustls/platformverifier/CertificateVerifier.kt`, `BuildConfig.kt` | verbatim | none |
| `java/moe/kopuz/kopuz/LoginActivity.kt` | probably verbatim | not needed for playback (WebView server login) |
| `kotlin/dev/dioxus/main/MainActivity.kt` | no | extends `WryActivity`. Port the permission, battery-exemption, edge-to-edge and folder-picker parts into `KopuzActivity` |
| `SystemColors.kt`, `Haptics.kt` | no | take `MainActivity`. Port later onto `KopuzActivity` |

Building it: the files are Kotlin. "Verbatim" means a Gradle build with the Kotlin plugin (the upstream
gpui-mobile example already uses Gradle). The other way is to port the roughly 620 media lines to Java and keep the
spike's `aapt2` + `javac` + `d8` pipeline without Gradle or kotlinc. The 480-line verifier can come from the
AAR that the `rustls-platform-verifier-android` crate ships instead. `build.rs`'s manifest additions, network
security config (cleartext for LAN servers) and R8 keep rules move into a checked-in `AndroidManifest.xml`
and resource files, because no `dx` template exists to patch.

## 6. Steps and effort (B; A in brackets where it differs)

| # | Step | Days |
|---|---|---|
| 1 | Java/Kotlin build: `hasCode="true"`, compile, dex, resources (icons, network config), manifest with service, receiver and permissions | 1.5 |
| 2 | Port the Kotlin (section 5), `KopuzActivity`, `KopuzDaemonService` with `android:process=":daemon"` | 2 |
| 3 | Rust daemon entry: `nativeStart`, ndk_context with the Application context, platform verifier, `assemble`, `kopuzd::bind_socket`/`serve` on the files-dir socket [A: core thread + `link::init_with_api`, 1.5] | 2 |
| 4 | Android branch of `os_media::spawn` (buttons to `PlayerCommand`) | 0.5 |
| 5 | `CoreArgs.cache_dir` and the cache paths in `boot.rs` | 0.5 |
| 6 | GPUI side: `link::init(Address::Socket, false)`, check-build `link` for Android, permission flow [A: move every systemint/db Context lookup off `ndk_context`, 2] | 1.5 |
| 7 | Lifecycle matrix on device and emulator: screen off for 30 min, swipe from recents, Bluetooth buttons, phone call, unplugging headphones, Doze (`adb shell dumpsys deviceidle force-idle`), killing the UI process, killing `:daemon`, app update | 2.5 |
| 8 | Optional: media3 `MediaSessionService` + `SimpleBasePlayer`, media resumption, Android Auto | 4 |

Total for B is about **10.5 days** without step 8, and about 14.5 with it. A is about 8.5 days, but
re-entering `android_main` is still an open risk there.

## 7. Open questions

- Whether android-activity 0.6 really releases `ndk_context` when `android_main` returns, and whether
  gpui-pre-mobile survives a second `android_main` in the same process (matters for A only).
- Whether `kopuzd` (tonic server, `bind_socket`) and `link` build for `aarch64-linux-android` (expected,
  not yet checked).
- Battery-optimization exemption: legacy prompts for it. With a correct mediaPlayback service it should not
  be needed. Check on aggressive OEMs (Xiaomi, Samsung) before keeping the prompt.
