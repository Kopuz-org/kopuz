package dev.gpui.mobile;

import android.Manifest;
import android.app.NativeActivity;
import android.app.NotificationManager;
import android.content.ActivityNotFoundException;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.database.Cursor;
import android.media.AudioManager;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.os.Environment;
import android.os.PowerManager;
import android.provider.DocumentsContract;
import android.provider.OpenableColumns;
import android.provider.Settings;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import moe.kopuz.kopuz.MediaReceiver;
import moe.kopuz.kopuz.MediaSessionHelper;
import android.window.OnBackInvokedDispatcher;
import android.text.Editable;
import android.text.InputType;
import android.text.Selection;
import android.text.TextWatcher;
import android.util.Log;
import android.view.Display;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.WindowInsets;
import android.view.WindowManager;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputConnectionWrapper;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;

/**
 * NativeActivity with a UI-thread InputConnection for multistage IMEs.
 *
 * <p>gpui-pre-mobile's example host (longbridge/gpui-mobile e65684f) with kopuz's changes,
 * listed in vendor/gpui-pre-mobile/PATCHES.md. The class name is fixed: gpui-pre-mobile
 * binds its natives to it.
 */
public class GpuiInputActivity extends NativeActivity {
    private static final String TAG = "kopuz-host";
    /** The IME's enter or action key, sent to GPUI as an "enter" keystroke. */
    private static final int IME_ENTER = 5;

    private InputProxy input;
    private ClipboardManager clipboard;
    private float keyboardDp = -1f;
    private boolean keyboardShown;

    /** Whether this process already ran the native app: GPUI starts once per process. */
    private static boolean started;

    private static final int REQUEST_NOTIFICATIONS = 1001;
    private static final int REQUEST_MEDIA = 1002;
    /** What `MediaSessionHelper.requestPermissions` asks with, for the daemon's media prompt. */
    private static final int REQUEST_MEDIA_FROM_DAEMON = 1;
    private static final int REQUEST_PICK = 2001;
    private static final String PREFS = "kopuz_host";
    private static final String ASKED_BATTERY = "asked_battery_exemption";

    /** The live activity, for the static entry points the daemon and the shared Kotlin call. */
    private static volatile GpuiInputActivity current;
    private static volatile boolean batteryExemptionPending;
    private boolean resumed;

    /** One picker at a time: who waits on it, and what it answered. */
    private static final Object PICK_LOCK = new Object();
    private static CountDownLatch pickDone;
    private static String[] picked;
    private static boolean pickForDaemon;
    private static boolean pickFolder;

    /**
     * Loads the library itself: NativeActivity's dlopen alone registers no JNI natives. A second
     * activity in a process that outlived the first (its task swiped away, the process kept cached)
     * cannot start GPUI again, so it relaunches the app in a fresh process instead. The media
     * session starts here, on the main thread, before the daemon does: its callbacks need a Looper,
     * which the daemon's threads lack.
     */
    @Override protected void onCreate(Bundle state) {
        if (started) {
            Log.i(TAG, "activity recreated in a live process; relaunching");
            Intent relaunch = getPackageManager().getLaunchIntentForPackage(getPackageName());
            if (relaunch != null && relaunch.getComponent() != null) {
                startActivity(Intent.makeRestartActivityTask(relaunch.getComponent()));
            }
            Runtime.getRuntime().exit(0);
            return;
        }
        started = true;
        MediaSessionHelper.init(this);
        try {
            String library = getPackageManager().getActivityInfo(getComponentName(),
                    PackageManager.GET_META_DATA).metaData.getString("android.app.lib_name");
            if (library != null) System.loadLibrary(library);
        } catch (PackageManager.NameNotFoundException error) {
            throw new IllegalStateException(error);
        }
        super.onCreate(state);
        current = this;
        setVolumeControlStream(AudioManager.STREAM_MUSIC);
        clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        followKeyboard();
        preferSteadyRefresh();
        if (Build.VERSION.SDK_INT >= 33) {
            getOnBackInvokedDispatcher().registerOnBackInvokedCallback(
                    OnBackInvokedDispatcher.PRIORITY_DEFAULT, GpuiInputActivity::nativeBack);
        }
        if (!requestNotificationPermission()) requestMediaPermission();
    }

    @Override protected void onResume() {
        super.onResume();
        resumed = true;
        askPendingBatteryExemption();
    }

    @Override protected void onPause() {
        resumed = false;
        super.onPause();
    }

    /**
     * Closing the app ends playback with it. Swiping the task away destroys the activity while the
     * media service keeps the process, and so the in-process player, alive; GPUI cannot start
     * again in that process anyway, so the service is stopped and the process exits.
     */
    @Override protected void onDestroy() {
        if (current == this) current = null;
        super.onDestroy();
        if (isChangingConfigurations()) return;
        Log.i(TAG, "activity destroyed; stopping playback and exiting");
        stopService(new Intent(this, moe.kopuz.kopuz.MusicService.class));
        NotificationManager notifications =
                (NotificationManager) getSystemService(NOTIFICATION_SERVICE);
        if (notifications != null) notifications.cancelAll();
        Runtime.getRuntime().exit(0);
    }

    /**
     * Asks for the notification permission Android 13+ needs before the now-playing notification
     * shows. Returns whether a dialog opened: two dialogs at once can cancel each other, so the
     * media permission waits for this answer.
     */
    private boolean requestNotificationPermission() {
        if (Build.VERSION.SDK_INT < 33
                || checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        == PackageManager.PERMISSION_GRANTED) {
            return false;
        }
        requestPermissions(new String[] {Manifest.permission.POST_NOTIFICATIONS},
                REQUEST_NOTIFICATIONS);
        return true;
    }

    /** Reading music folders in shared storage needs its own grant, which the scanner cannot ask for. */
    private void requestMediaPermission() {
        String permission = Build.VERSION.SDK_INT >= 33
                ? Manifest.permission.READ_MEDIA_AUDIO
                : Manifest.permission.READ_EXTERNAL_STORAGE;
        if (checkSelfPermission(permission) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[] {permission}, REQUEST_MEDIA);
        }
    }

    @Override public void onRequestPermissionsResult(
            int requestCode, String[] permissions, int[] grantResults) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults);
        if (requestCode == REQUEST_NOTIFICATIONS) {
            requestMediaPermission();
            return;
        }
        if (requestCode != REQUEST_MEDIA && requestCode != REQUEST_MEDIA_FROM_DAEMON) return;
        boolean granted = grantResults.length > 0;
        for (int result : grantResults) granted &= result == PackageManager.PERMISSION_GRANTED;
        MediaReceiver.nativeOnAction(granted ? "media-granted" : "media-denied");
    }

    /**
     * `MusicService` asks when playback starts, so the screen-off player is not put to sleep. The
     * dialog opens once the activity is in front, and only once ever.
     */
    public static void requestBatteryExemptionOnce() {
        batteryExemptionPending = true;
        GpuiInputActivity activity = current;
        if (activity != null) activity.runOnUiThread(activity::askPendingBatteryExemption);
    }

    private void askPendingBatteryExemption() {
        if (!resumed || !batteryExemptionPending) return;
        batteryExemptionPending = false;
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        if (prefs.getBoolean(ASKED_BATTERY, false)) return;
        prefs.edit().putBoolean(ASKED_BATTERY, true).apply();
        PowerManager power = (PowerManager) getSystemService(POWER_SERVICE);
        if (power.isIgnoringBatteryOptimizations(getPackageName())) return;
        try {
            startActivity(new Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                    .setData(Uri.parse("package:" + getPackageName())));
        } catch (ActivityNotFoundException error) {
            Log.w(TAG, "no battery optimization settings", error);
        }
    }

    /** The daemon's way to the background, for a back press on a root screen. */
    public static void moveToBack() {
        GpuiInputActivity activity = current;
        if (activity != null) activity.runOnUiThread(() -> activity.moveTaskToBack(true));
    }

    /**
     * Opens the system picker and blocks the calling worker thread until it answers: a folder's
     * storage path, or the picked files copied into the cache. Null when it was cancelled, or for a
     * folder no filesystem path stands behind. Called through JNI by GPUI's path prompt.
     */
    public static String[] gpuiPickPaths(boolean folder, boolean multiple) {
        CountDownLatch done = startPick(folder, multiple, false);
        if (done == null) return null;
        try {
            done.await();
        } catch (InterruptedException error) {
            Thread.currentThread().interrupt();
            return null;
        }
        synchronized (PICK_LOCK) {
            return picked;
        }
    }

    /** The daemon's folder picker: the answer comes back as a `MediaReceiver` action, not a return. */
    public static boolean pickFolderForDaemon() {
        return startPick(true, false, true) != null;
    }

    private static CountDownLatch startPick(boolean folder, boolean multiple, boolean forDaemon) {
        GpuiInputActivity activity = current;
        if (activity == null) return null;
        CountDownLatch done = new CountDownLatch(1);
        synchronized (PICK_LOCK) {
            finishPick(null);
            pickDone = done;
            picked = null;
            pickForDaemon = forDaemon;
            pickFolder = folder;
        }
        activity.runOnUiThread(() -> {
            Intent intent;
            if (folder) {
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            } else {
                intent = new Intent(Intent.ACTION_OPEN_DOCUMENT)
                        .addCategory(Intent.CATEGORY_OPENABLE)
                        .setType("*/*")
                        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, multiple);
            }
            try {
                activity.startActivityForResult(intent, REQUEST_PICK);
            } catch (ActivityNotFoundException error) {
                Log.w(TAG, "no document picker", error);
                synchronized (PICK_LOCK) {
                    finishPick(null);
                }
            }
        });
        return done;
    }

    /** Hands the answer to whoever waits; a picker superseded by a newer one answers null. */
    private static void finishPick(String[] paths) {
        if (pickDone == null) return;
        picked = paths;
        if (pickForDaemon) {
            String action;
            if (paths != null && paths.length > 0) action = "folder-picked:" + paths[0];
            else action = "folder-cancelled";
            MediaReceiver.nativeOnAction(action);
        }
        pickDone.countDown();
        pickDone = null;
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode != REQUEST_PICK) return;
        boolean folder;
        synchronized (PICK_LOCK) {
            folder = pickFolder;
        }
        List<Uri> uris = new ArrayList<>();
        if (resultCode == RESULT_OK && data != null) {
            if (data.getClipData() != null) {
                for (int i = 0; i < data.getClipData().getItemCount(); i++) {
                    uris.add(data.getClipData().getItemAt(i).getUri());
                }
            } else if (data.getData() != null) {
                uris.add(data.getData());
            }
        }
        if (folder) {
            String path = uris.isEmpty() ? null : treeUriToPath(uris.get(0));
            if (!uris.isEmpty() && path == null) {
                Log.w(TAG, "picked folder has no filesystem path: " + uris.get(0));
            }
            synchronized (PICK_LOCK) {
                finishPick(path == null ? null : new String[] {path});
            }
            return;
        }
        new Thread(() -> {
            String[] paths = copyIntoCache(uris);
            synchronized (PICK_LOCK) {
                finishPick(paths);
            }
        }, "kopuz-pick-copy").start();
    }

    /** A document tree's storage path, for trees on the device's own storage. */
    private static String treeUriToPath(Uri uri) {
        String docId;
        try {
            docId = DocumentsContract.getTreeDocumentId(uri);
        } catch (IllegalArgumentException error) {
            return null;
        }
        if (docId.startsWith("raw:")) return docId.substring(4);
        if (!"com.android.externalstorage.documents".equals(uri.getAuthority())) return null;
        int colon = docId.indexOf(':');
        String volume = colon < 0 ? docId : docId.substring(0, colon);
        String relative = colon < 0 ? "" : docId.substring(colon + 1).replaceAll("^/+|/+$", "");
        String primary = Environment.getExternalStorageDirectory().getAbsolutePath();
        String root;
        if (volume.equalsIgnoreCase("primary")) root = primary;
        else if (volume.equalsIgnoreCase("home")) root = primary + "/Documents";
        else root = "/storage/" + volume;
        return relative.isEmpty() ? root : root + "/" + relative;
    }

    /** Picked documents have no path of their own, so each is copied into the cache under its name. */
    private String[] copyIntoCache(List<Uri> uris) {
        if (uris.isEmpty()) return null;
        File dir = new File(getCacheDir(), "picked/" + System.nanoTime());
        if (!dir.mkdirs()) return null;
        List<String> paths = new ArrayList<>();
        for (Uri uri : uris) {
            File target = new File(dir, displayName(uri));
            try (InputStream in = getContentResolver().openInputStream(uri);
                    OutputStream out = new FileOutputStream(target)) {
                if (in == null) continue;
                byte[] buffer = new byte[64 * 1024];
                int read;
                while ((read = in.read(buffer)) != -1) out.write(buffer, 0, read);
                paths.add(target.getAbsolutePath());
            } catch (Exception error) {
                Log.w(TAG, "could not copy " + uri, error);
            }
        }
        return paths.isEmpty() ? null : paths.toArray(new String[0]);
    }

    private String displayName(Uri uri) {
        try (Cursor cursor = getContentResolver().query(
                uri, new String[] {OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (cursor != null && cursor.moveToFirst()) {
                String name = cursor.getString(0);
                if (name != null && !name.contains("/")) return name;
            }
        } catch (Exception error) {
            Log.w(TAG, "no display name for " + uri, error);
        }
        return "picked-" + System.nanoTime();
    }

    /** A frame costs about 12 ms on a phone today, so 60 Hz is the rate every frame makes. */
    private static final float TARGET_HZ = 60f;

    /**
     * Ask for the display mode nearest 60 Hz at its current size while the app is in front: on a
     * 120 Hz screen every frame missed its 8.3 ms vsync and scrolling fell to about 34 fps, while at
     * 60 Hz it holds 60.
     */
    private void preferSteadyRefresh() {
        Display display = getWindow().getDecorView().getDisplay();
        if (display == null) display = getWindowManager().getDefaultDisplay();
        Display.Mode current = display.getMode();
        Display.Mode chosen = null;
        for (Display.Mode mode : display.getSupportedModes()) {
            if (mode.getPhysicalWidth() != current.getPhysicalWidth()
                    || mode.getPhysicalHeight() != current.getPhysicalHeight()
                    || mode.getRefreshRate() < TARGET_HZ - 1f) {
                continue;
            }
            if (chosen == null || mode.getRefreshRate() < chosen.getRefreshRate()) chosen = mode;
        }
        if (chosen == null) return;
        WindowManager.LayoutParams params = getWindow().getAttributes();
        params.preferredDisplayModeId = chosen.getModeId();
        params.preferredRefreshRate = chosen.getRefreshRate();
        getWindow().setAttributes(params);
        Log.i(TAG, "preferred display mode " + chosen.getRefreshRate() + " Hz");
    }

    /**
     * Report the soft keyboard's height in dp and whether it is up (a floating one is up with no
     * height); the window keeps its size (adjustNothing).
     */
    private void followKeyboard() {
        View decor = getWindow().getDecorView();
        decor.setOnApplyWindowInsetsListener((view, insets) -> {
            int bottom = insets.getInsets(WindowInsets.Type.ime()).bottom;
            boolean shown = insets.isVisible(WindowInsets.Type.ime());
            float dp = shown ? bottom / getResources().getDisplayMetrics().density : 0f;
            if (dp != keyboardDp || shown != keyboardShown) {
                keyboardDp = dp;
                keyboardShown = shown;
                nativeKeyboardHeight(dp, shown);
            }
            return view.onApplyWindowInsets(insets);
        });
    }

    /** Called from GPUI's thread, which has no Looper, so the manager comes from onCreate. */
    public String gpuiClipboardRead() {
        if (clipboard == null || !clipboard.hasPrimaryClip()) return null;
        ClipData clip = clipboard.getPrimaryClip();
        if (clip == null || clip.getItemCount() == 0) return null;
        CharSequence text = clip.getItemAt(0).coerceToText(this);
        return text == null ? null : text.toString();
    }

    public void gpuiClipboardWrite(String text) {
        if (clipboard == null) return;
        clipboard.setPrimaryClip(ClipData.newPlainText("kopuz", text));
    }

    public void gpuiOpenUrl(String url) {
        runOnUiThread(() -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
            } catch (ActivityNotFoundException error) {
                Log.w(TAG, "no app opens " + url, error);
            }
        });
    }

    /**
     * Shows the IME over a hidden proxy. No multi-line flag: the IME then shows an action key,
     * which GPUI gets as "enter", and a multi-line GPUI input makes that a newline itself.
     */
    public void gpuiShowKeyboard(int keyboardType, long session) {
        runOnUiThread(() -> {
            if (input == null) {
                input = new InputProxy();
                input.setAlpha(0f);
                input.setPadding(0, 0, 0, 0);
                addContentView(input, new ViewGroup.LayoutParams(1, 1));
            }
            input.reset(session);
            input.setFocusable(true);
            input.setFocusableInTouchMode(true);
            int type = InputType.TYPE_CLASS_TEXT;
            switch (keyboardType) {
                case 1: type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS; break;
                case 2: type = InputType.TYPE_CLASS_PHONE; break;
                case 3: type = InputType.TYPE_CLASS_NUMBER; break;
                case 4: type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_URI; break;
                case 5: type = InputType.TYPE_CLASS_NUMBER | InputType.TYPE_NUMBER_FLAG_DECIMAL; break;
            }
            input.setInputType(type);
            input.setImeOptions(EditorInfo.IME_ACTION_DONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                    | EditorInfo.IME_FLAG_NO_FULLSCREEN);
            input.requestFocus();
            InputMethodManager imm = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
            imm.restartInput(input);
            imm.showSoftInput(input, InputMethodManager.SHOW_IMPLICIT);
        });
    }

    /**
     * The proxy stops being focusable too: as the window's only focusable view it would take the
     * focus straight back, and the system would show the IME for it again on resume.
     */
    public void gpuiHideKeyboard(long session) {
        runOnUiThread(() -> {
            if (input == null) return;
            input.reset(session);
            InputMethodManager imm = (InputMethodManager) getSystemService(INPUT_METHOD_SERVICE);
            imm.hideSoftInputFromWindow(input.getWindowToken(), 0);
            input.setFocusable(false);
            input.setFocusableInTouchMode(false);
            input.clearFocus();
        });
    }

    public void gpuiResetComposition(long session) {
        runOnUiThread(() -> {
            if (input == null) return;
            input.reset(session);
            ((InputMethodManager) getSystemService(INPUT_METHOD_SERVICE)).restartInput(input);
        });
    }

    private final class InputProxy extends EditText {
        private long session;
        private int depth;
        private boolean marked;

        /** Hardware keyboards edit the widget outside its InputConnection; the watcher forwards those. */
        InputProxy() {
            super(GpuiInputActivity.this);
            addTextChangedListener(new TextWatcher() {
                public void beforeTextChanged(CharSequence s, int start, int count, int after) {}
                public void onTextChanged(CharSequence s, int start, int before, int count) {}
                public void afterTextChanged(Editable text) {
                    if (depth == 0) { depth++; endEdit(); }
                }
            });
        }

        @Override public boolean onKeyDown(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_DEL && getText().length() == 0 && !marked) {
                nativeIme(session, 3, "", 1, 0);
                return true;
            }
            if (code == KeyEvent.KEYCODE_ENTER || code == KeyEvent.KEYCODE_NUMPAD_ENTER) {
                enter();
                return true;
            }
            return super.onKeyDown(code, event);
        }

        void reset(long nextSession) {
            depth++;
            getText().clear();
            marked = false;
            session = nextSession;
            depth = 0;
        }

        /** Commit what is composed, then hand GPUI the enter key. */
        private void enter() {
            depth++;
            try {
                BaseInputConnection.removeComposingSpans(getText());
            } finally {
                endEdit();
            }
            nativeIme(session, IME_ENTER, "", 0, 0);
        }

        private void endEdit() {
            if (--depth != 0) return;
            Editable text = getText();
            boolean composing = BaseInputConnection.getComposingSpanStart(text) >= 0;
            if (composing || marked || text.length() > 0) {
                nativeIme(session, composing ? 0 : 1, text.toString(),
                        Math.max(0, Selection.getSelectionStart(text)),
                        Math.max(0, Selection.getSelectionEnd(text)));
                marked = composing;
                if (!composing) {
                    depth++;
                    text.clear();
                    depth--;
                }
            }
        }

        @Override public boolean onKeyPreIme(int code, KeyEvent event) {
            if (code == KeyEvent.KEYCODE_BACK && event.getAction() == KeyEvent.ACTION_UP) {
                nativeIme(session, 4, "", 0, 0);
            }
            return super.onKeyPreIme(code, event);
        }

        @Override public InputConnection onCreateInputConnection(EditorInfo info) {
            InputConnection connection = super.onCreateInputConnection(info);
            if (connection == null) return null;
            final long connectionSession = session;
            /* Some IMEs commit a newline for their enter key instead of sending it. */
            return new InputConnectionWrapper(connection, false) {
                @Override public boolean beginBatchEdit() {
                    if (connectionSession != session) return false;
                    depth++;
                    return super.beginBatchEdit();
                }
                @Override public boolean endBatchEdit() {
                    if (connectionSession != session) return false;
                    boolean result = super.endBatchEdit();
                    if (depth > 0) endEdit();
                    return result;
                }
                @Override public boolean setComposingText(CharSequence text, int cursor) {
                    if (connectionSession != session) return false;
                    depth++;
                    try { return super.setComposingText(text, cursor); }
                    finally { endEdit(); }
                }
                @Override public boolean setComposingRegion(int start, int end) {
                    if (connectionSession != session) return false;
                    depth++;
                    try { return super.setComposingRegion(start, end); }
                    finally { endEdit(); }
                }
                @Override public boolean finishComposingText() {
                    if (connectionSession != session) return false;
                    depth++;
                    try { return super.finishComposingText(); }
                    finally { endEdit(); }
                }
                @Override public boolean commitText(CharSequence text, int cursor) {
                    if (connectionSession != session) return false;
                    if ("\n".contentEquals(text)) {
                        enter();
                        return true;
                    }
                    depth++;
                    try { return super.commitText(text, cursor); }
                    finally { endEdit(); }
                }
                @Override public boolean setSelection(int start, int end) {
                    if (connectionSession != session) return false;
                    depth++;
                    try { return super.setSelection(start, end); }
                    finally { endEdit(); }
                }
                @Override public boolean deleteSurroundingText(int before, int after) {
                    if (connectionSession != session) return false;
                    if (getText().length() == 0 && !marked) {
                        nativeIme(session, 2, "", before, after);
                        return true;
                    }
                    depth++;
                    try { return super.deleteSurroundingText(before, after); }
                    finally { endEdit(); }
                }
                @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) {
                    if (connectionSession != session) return false;
                    if (getText().length() == 0 && !marked) {
                        nativeIme(session, 3, "", before, after);
                        return true;
                    }
                    depth++;
                    try { return super.deleteSurroundingTextInCodePoints(before, after); }
                    finally { endEdit(); }
                }
                @Override public boolean sendKeyEvent(KeyEvent event) {
                    if (connectionSession != session) return false;
                    if (event.getKeyCode() == KeyEvent.KEYCODE_DEL) {
                        if (event.getAction() == KeyEvent.ACTION_DOWN) deleteSurroundingText(1, 0);
                        return true;
                    }
                    if (event.getKeyCode() == KeyEvent.KEYCODE_ENTER
                            || event.getKeyCode() == KeyEvent.KEYCODE_NUMPAD_ENTER) {
                        if (event.getAction() == KeyEvent.ACTION_DOWN) enter();
                        return true;
                    }
                    return super.sendKeyEvent(event);
                }
                @Override public boolean performEditorAction(int action) {
                    if (connectionSession != session) return false;
                    enter();
                    return true;
                }
            };
        }
    }

    private static native void nativeIme(long session, int kind, String text, int start, int end);

    private static native void nativeBack();

    private static native void nativeKeyboardHeight(float heightDp, boolean visible);
}
