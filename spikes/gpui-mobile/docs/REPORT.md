# Report: kopuz's GPUI app on Android (gpui-mobile spike)

Branch `gpui-mobile-spike`. Everything is under `spikes/gpui-mobile/`. Nothing outside that folder
changed, and the legacy app is untouched. The details behind this summary are in `ANALYSIS.md` (how the
frontends reach the daemon, and what the GPUI app depends on), `UPSTREAM.md` (gpui-pre-mobile against
kopuz's versions, plus the build recipe) and `FOREGROUND_SERVICE.md` (the on-device daemon plan).

## Follow-up: in-process daemon and the real kopuz app on Android

**In-process daemon.** The kopuz daemon core runs inside the app, as it does in the legacy Android app
(`kopuz_core::boot::assemble` on its own thread, used through `LocalApi`). Its database, settings and
caches live under the app's files directory, and `HOME` and `TMPDIR` point there too. Six test tones
are seeded on first run (`src/seed.rs`). rusty_v8 and deno_core are not involved, because
kopuz-server already leaves them out on Android. A host kopuzd over TCP is used only when the device
property `debug.kopuz.spike.addr` is set, which `scripts/run-dev.sh` does.

**Resume fix.** `src/lifecycle.rs` calls `window.refresh()` when the surface is recreated or the
platform becomes active, and again on the next three ticks. The screenshot 2 s after resume is now 0%
black (it was 100% black before). The runtime density change is still not applied (upstream).

**The real app runs.** `apps/kopuz/app` now has a library target, `kopuz_app`. Its old `main` body is
`run()`, and an Android-only `run_in_process` opens the main window from the `Application::run`
callback rather than from a task, because of gpui-pre-mobile's lock (see above). `link::init_in_process`
hands the in-process `Arc<dyn KopuzApi>` to the same supervisor and event relay that desktop uses.
The host package `spikes/gpui-mobile/kopuz-android` builds `apps/kopuz/app` with gpui-kit 0.6.4
for aarch64-linux-android. On the emulator it opened its real first-run screen, and a tap went
through to the real shell, with no crash. **That proves gpui-kit 0.6.4 builds and runs on Android.**
The root `Cargo.toml` and `Cargo.lock` are unchanged; the Android build lives in the spike's own
workspace and lock. A desktop `cargo check -p kopuz -p link -p profile --locked` passes.

**Real app on Android: what is gated** (`cfg(not(target_os = "android"))` or `cfg!`; desktop is
unchanged):
- `app/src/lib.rs`: the `args`, `instance` and `links` modules (command line, single-instance lock,
  deep links), `daemon_address`, the desktop `run()` (the `--daemon` flag, profile init, instance
  claim, open-urls and reopen handlers, the relay listener, the preferences wait, the tray),
  `register_mini_player` (a second window) and `PREFS_WAIT`. The menu and tray gates already left
  Android out.
- `app/src/shell.rs`: the window is not redecorated or reopened when the titlebar setting changes.
- jemalloc (`app/Cargo.toml`, `profile/Cargo.toml`, `profile/src/heap.rs`): its build needs `-lgcc`,
  which the NDK no longer ships, so heap figures read zero on Android.
- `link/src/lib.rs`: spawning kopuzd (`daemon_process`, `start_daemon`, the START constants and
  `answers`), replacing an older daemon (`replace::stop_daemon`), the deep-link `relay`, and
  `run_daemon_flag`.
- File dialogs return early: `components/src/folders.rs`, `library/src/playlists.rs`,
  `settings_sources/src/source_tools.rs`, `store/src/artwork_edit.rs`, `store/src/picture.rs` and
  `settings_playback/src/logs.rs`.
- `now_playing/src/bar.rs`: the mini-player button is hidden. `status/src/update.rs`: the updater
  banner is hidden.
- Small compile-only gates: the `run_daemon_flag` re-export and an `Instant` import in `link`, and
  `allow(dead_code)` on `replace` and on `MainWindow`/`main_window`.
- Android start-up (`run_in_process`) skips some desktop steps: there is no preferences wait (so the
  keymap is set up before the preferences load, and saved shortcut overrides are probably ignored),
  no offline flag, no deep links or relay, and no Booted signal.

**Known gaps of the real app on the emulator:**
- Home shows its empty state although 6 tracks were seeded. It may need a source switch or a reload;
  not investigated.
- The layout is desktop-sized: the top bar is cramped, and the 720 px minimum window size does not
  apply.
- The APK has no Java, so there is no media notification (`MediaSessionHelper`), TLS verification
  fails (the radio registry cannot be fetched), and the clipboard and `open_url` are stubs.
- Text input and IME are untested.
- Any code that opens windows, or touches the clipboard or display, from a `cx.spawn` task would hit
  the gpui-pre-mobile lock deadlock.

This lowers the first row of the effort table below ("prove gpui-kit 0.6.4 on Android"), which is now
done. The recommendation stands: keep the legacy app on Android for now, because IME, accessibility
and the media session are still missing.

## What was built

- A standalone crate, `kopuz-mobile-spike` (about 500 lines of Rust), on gpui-pre 0.3.5 and
  gpui-pre-mobile 0.1.0. It has one screen: a `uniform_list` of the library, where tapping a row plays it,
  and a play/pause button.
- It talks to a real kopuzd through kopuz's own `kopuz-client` (`GrpcApi::connect_tcp`) and `kopuz-api`
  (`tracks`, `set_queue`, `player_command`).
- The daemon runs on the Mac with `--listen 127.0.0.1:50777`, isolated from the user's library. The
  emulator reaches it through `adb reverse`.
- A Java-free `NativeActivity` APK, built with plain cargo, aapt2, zipalign and apksigner from the
  installed SDK and NDK. There is no Gradle or cargo-ndk, and nothing was installed.
- `scripts/test-emulator.sh` runs the whole test once, against an emulator that is already running.

## Versions

- kopuz uses gpui-kit 0.6.4, which sits on gpui-pre 0.3.5.
- gpui-pre-mobile 0.1.0's manifest pins `gpui-pre =0.3.5` and `gpui-pre-wgpu =0.3.5`. Its README says
  0.3.4, but that is out of date. **The distance is zero:** the spike resolves to exactly kopuz's
  versions (wgpu 29.0.4, cosmic-text 0.19).
- gpui-kit 0.6.4 already leaves out its desktop platform crate on Android. The spike did not compile
  gpui-kit for Android, so that is the first thing to prove in a follow-up.
- `gpui-mobile` 0.1.0 is an empty placeholder that targets zed's gpui. `gpui-pre-mobile` (Longbridge) is
  the one that fits kopuz.
- Upstream `main` (unpublished) has moved to gpui-pre 0.3.10 and adds TalkBack. Using it would mean
  upgrading kopuz to gpui-kit 0.8.x at the same time.

## Test results (Pixel 8 Pro emulator, API 35, one end-to-end run after one fix)

| Area | Result |
|---|---|
| Rendering | Works through wgpu on **Vulkan**. The emulator's GL translator is rejected, so it runs on the llvmpipe CPU Vulkan; a phone would use its GPU. |
| Daemon | Connects, loads 24 tracks, and starts playback (the daemon reports `playing`). Play/pause works. |
| Touch input | Taps hit the right rows and buttons. |
| Scrolling | A touch swipe scrolls the list. Upstream uses iOS-style scroll physics on Android. |
| Suspend and resume | **Partial.** Home and relaunch keep the process alive (same pid), and the pause, resume, surface-lost and surface-recreated events fire. No crash. **But the window shows black after resume until the next input:** `07-resumed.png` (6 s after `am start`, about 6 s after `surface_recreated`) and `08-density-280.png` (about 8 s later still) are black below the status bar. The normal 0x121212 screen only comes back in `09`, after a tap. The likely cause is upstream: no frame is forced after the surface is replaced (see below). |
| Rotation | Works; the new size is 947x448 logical pixels. |
| Display scaling | Works at launch (scale 3 at 480 dpi). **A density change at runtime is not applied.** This comes from reading the source. The run could not confirm it on screen, because the window was still black (`08-density-280.png`); `summary.txt` only shows that no new `SPIKE scale=` line appeared. In gpui-pre-mobile 0.1.0, the scale is stored only in `AndroidWindow::new` (`src/android/window.rs:336-371`, `scale_factor` at :371). `ConfigChanged` only updates the keyboard layout and the appearance (`jni.rs:695-709`). `handle_resize` returns early when the pixel size has not changed (`window.rs:538-541`), and otherwise reuses the stored scale (`window.rs:553`). There is no setter. |
| Soft keyboard and IME | **Not available** in this spike, and **not tested by the run**. The `keyboard FAIL` in `summary.txt` is built into this spike: there is no text field, so the probe taps the spot where a Filter field would be. The gap itself comes from reading the source: gpui-pre-mobile shows the keyboard and does IME through its Java `GpuiInputActivity`, which a Java-free NativeActivity does not have (UPSTREAM.md section 5). |
| Accessibility | **None.** gpui-pre 0.3.5's no-op accessibility defaults apply. |

**A pitfall the spike hit (now fixed):** gpui-pre-mobile runs every GPUI foreground task while it holds
its platform `state` lock (`platform.rs:941`). Calling a platform method that takes the same lock (for
example `primary_window()`) from inside `cx.spawn` therefore deadlocks the UI thread, and Android raises an
ANR. The workaround is in `src/lifecycle.rs`, and any port has to keep to the same rule.

**Why the window is black after resume (likely cause, from the source and the logs; not tested):**

- After `TerminateWindow`, the event loop's one-time init block finds `INIT_WINDOW_DONE` false while the
  window still exists. It sets the flag back to true and requests a frame on the unconfigured surface
  (`jni.rs:757-779`; `logcat-spike.txt:195`, "NATIVE_INITIALIZED = true", 1 ms after `term_window`).
- So on resume, the render loop (`jni.rs:785-790`) is already running from `set_active(true)` at
  22:51:17.977 (`logcat-spike.txt:222`), 18 ms before `init_window` replaces the surface at 17.995 (:232).
  If GPUI redraws for the activation, that frame goes to the dead surface.
- `init_window` swaps the wgpu surface (`window.rs:441-464`) and then calls `handle_resize` (`jni.rs:649`).
  The pixel size has not changed (1344x2992), so `handle_resize` returns before GPUI's resize callback, which
  would mark the window dirty (`window.rs:538-541`). This fits rotation, which does change the pixel size
  (`logcat-spike.txt:372`) and renders correctly (`10-rotated.png`).
- Every frame request passes `require_presentation: false` and sets `force_render` only when text input
  is pending (`window.rs:1299-1306`). GPUI draws only a dirty window, so nothing reaches the new surface
  until an input event dirties it. That happened in `09`, after a tap.

The bug is upstream (gpui-pre-mobile 0.1.0), not in the spike. The spike just has nothing that would hide
it. An upstream fix would force one render, or call the resize callback, after a surface replacement. A
workaround in the spike would be to call `window.refresh()` when `src/lifecycle.rs` sees
`surface_recreated` (`logcat-spike.txt:239`). Neither has been tried.

## What is missing upstream (gpui-pre-mobile 0.1.0)

- **IME and soft keyboard:** these need its Java input activity, so a port needs a Java or Kotlin host,
  not a pure NativeActivity. The keyboard height is never reported to GPUI.
- **Accessibility:** none (TalkBack is only on the unpublished `main`).
- **Lifecycle hooks:** the pause, resume and surface-recreation events fire, but no frame is forced after
  the surface is recreated, so the window stays black until the next input (see above). `onSaveInstanceState`
  and low-memory are only logged, there is no callback for losing the surface, and it renders continuously,
  which costs battery.
- **Runtime density changes** are not applied (from the source; the run could not confirm it on screen). Safe-area insets are not passed through to GPUI. Clipboard and
  `open_url` are stubs. Android-style scroll physics are never selected.
- There is no bundled colour emoji font; Android's COLRv1 emoji are not supported.

## What a full port would take (rough, one developer who knows the code)

| Step | Effort |
|---|---|
| Prove gpui-kit 0.6.4 on Android; move the spike onto gpui-kit, `store` and `theme` | 3–5 days |
| A Kotlin host activity (GpuiInputActivity or a subclass) for the keyboard and IME, plus the Gradle or packaging pipeline (F-Droid, release CI) | 5–8 days |
| The daemon on the device as a `:daemon` foreground service with a media session (FOREGROUND_SERVICE.md) | about 10.5 days |
| Touch layouts for the screens the Android app needs (home, library, albums, artists, playlists, now playing, settings, sources); desktop-only features replaced or dropped (tray, menus, file dialogs, spawning kopuzd, the updater) | 15–25 days |
| Upstream fixes or patches: redraw after resume, density, insets, clipboard, emoji, scroll physics, render-on-demand | 5–10 days |
| Accessibility (TalkBack) by moving to upstream `main` together with gpui-kit 0.8.x | 5–10 days, plus the kopuz-wide upgrade |
| Testing on real phones (GPU drivers, 16 KB pages, Android 12–15) | 3–5 days |

That totals about **9–14 weeks** to reach parity with the legacy Android app, before release polish.

## Recommendation

**Later, not now.** The spike shows the hard part works: kopuz's exact gpui-pre version renders on
Android through Vulkan, takes touch input and drives the real daemon with kopuz's own client crates.
Its process survives backgrounding and the lifecycle events fire, but after resume the window shows
black until the next input, most likely because gpui-pre-mobile forces no frame after it replaces the
surface. But the published gpui-pre-mobile still lacks IME and accessibility, both of
which a music app's search and sign-in screens need, and its accessibility work sits behind a GPUI upgrade
kopuz has not made. Keep the legacy app on Android for now.

Two cheap steps are worth doing now, because both keep the option open:

1. Serve the in-process daemon on a Unix socket in `filesDir` (process model B in FOREGROUND_SERVICE.md),
   so a later GPUI UI can reuse `link` unchanged.
2. Re-run this spike after kopuz moves to gpui-kit 0.8.x and gpui-pre-mobile publishes its TalkBack and
   IME work. `scripts/test-emulator.sh` makes that a single command.

## Not verified

- Real-phone GPU rendering (the emulator used CPU Vulkan), frame rate, and battery use.
- gpui-kit compiling for Android.
- Text input, because the spike has no text field. The soft keyboard and IME gap comes from reading the
  source; the run's keyboard check cannot pass by design.
- That resume renders on its own. It did not, within 6 s (`07`) or after the density change (`08`). The
  cause above comes from reading the source, and neither fix has been tried.
- The runtime density change on screen (the window was black). The "not applied" finding comes from
  reading the source.
- The test script judged the screenshots in `build/shots/` (for example, whether the image changed after
  scrolling). Afterwards, only shots 05 to 10 were sampled pixel by pixel: 07 and 08 are black below the
  status bar, and 05, 09 and 10 show the 0x121212 background.
