# gpui-pre-mobile 0.1.0, patched for kopuz

A copy of the crates.io release (longbridge/gpui-mobile `e65684f`), without its screenshots, CI
files, lock and `.cargo` config. `spikes/gpui-mobile/Cargo.toml` patches it in. The host activity
these patches talk to is `spikes/gpui-mobile/android/kopuz-app/java/dev/gpui/mobile/GpuiInputActivity.java`.

- `android/platform.rs`: `tick` and `flush_main_thread_tasks` release the platform `state` lock
  before they run tasks. Before, any platform call from a `cx.spawn` task deadlocked the UI thread,
  for example writing the clipboard after an await (Share in the player).
- `android/platform.rs`, `android/jni.rs`: the clipboard and `open_url` go to the host activity
  (`gpuiClipboardRead`, `gpuiClipboardWrite`, `gpuiOpenUrl`). A host without them gets the old
  in-process clipboard and a logged `open_url`. `jni::move_task_to_back` is new.
- `android/platform.rs`: `Platform::gestures` gives Android's long-press timeout (400 ms) and
  `ScrollPhysics::android()` in place of gpui's iOS fling.
- `android/text_input.rs`: IME event kind 4 (back closed the IME) no longer sends `escape`, which
  would also have gone back a screen. It sets a flag the host reads with
  `android::take_ime_dismissed`. The new kind 5 is an `enter` keystroke. The host sends it for the
  IME's enter and action keys, so a single-line input gets its submit instead of a stripped newline.
- `android/window.rs`: a hardware key's character is inserted into the focused input when no
  binding took the key, as GPUI's desktop platforms do. A physical keyboard (or `adb shell input
  text`) typed nothing before.
- `android/jni.rs`: `GpuiInputActivity.nativeKeyboardHeight(dp, visible)` feeds
  `set_keyboard_height`, which had no Android caller, so `gpui_mobile::keyboard_height()` now reads
  the IME's height. While the IME is up (in the insets, or asked for by GPUI and not put away since, which covers
  Gboard's toolbar-only mode), back hides it and raises the same flag as kind 4. So does a back press within 500 ms of the IME
  closing itself: Android hands back to the IME first and then delivers the same press to the activity
  instead of reaching GPUI as `escape`: a NativeActivity sees back before the IME does, so one press
  used to close the keyboard and the screen under it together.
- `android/window.rs`: the surface presents with vsync-paced `Fifo` instead of `Mailbox`. Mailbox drew
  whenever input dirtied the window, so on a 60 Hz panel a scroll's steps reached the screen unevenly.
- `android/window.rs`: the window starts in the system's night mode. It used to start Light until
  the first configuration change.
- `android/jni.rs`, `android/text_input.rs`: Android 13+ delivers back through
  `OnBackInvokedDispatcher` instead of a key event. The host's callback is queued apart from IME
  sessions and reaches GPUI as `escape` on its own thread. It registers at default priority, so the
  keyboard still gets back first.
- `android/jni.rs`: the volume keys are left to Android, which turns them into media volume.
  They used to reach GPUI and change nothing.
- `android/document_picker.rs`: folder and file prompts open the host's system picker
  (`GpuiInputActivity.gpuiPickPaths`) instead of failing. A folder comes back as its storage path,
  and picked files are copied into the app's cache, since a content URI is not a path.
