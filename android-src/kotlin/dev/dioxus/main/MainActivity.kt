package dev.dioxus.main

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.PowerManager
import android.provider.Settings
import android.graphics.Color
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import androidx.core.view.WindowCompat
import com.temidaradev.kopuz.MediaReceiver
import com.temidaradev.kopuz.MediaSessionHelper

typealias BuildConfig = com.temidaradev.kopuz.BuildConfig

class MainActivity : WryActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        instance = this
        enableEdgeToEdge()
        MediaSessionHelper.init(this)
        if (!requestNotificationPermission()) {
            requestMediaPermission()
        }
        requestBatteryOptimizationExemption()
    }

    // Draw under the status/navigation bars and make them transparent so the app's
    // dark background extends edge-to-edge instead of the system's gray bar. The web
    // UI already pads with env(safe-area-inset-*), so content stays clear of the bars.
    private fun enableEdgeToEdge() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.TRANSPARENT
        // Stop the system painting a translucent gray contrast scrim behind the bars,
        // so the dark UI runs truly edge-to-edge and merges with the in-app header.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            window.isStatusBarContrastEnforced = false
            window.isNavigationBarContrastEnforced = false
        }
        WindowCompat.getInsetsController(window, window.decorView).apply {
            // Dark UI → light (white) status/nav icons.
            isAppearanceLightStatusBars = false
            isAppearanceLightNavigationBars = false
        }
    }

    // Forward hardware/gesture back to Rust, which pops the in-app router or, at the
    // root, backgrounds the app. Deliberately NOT calling super: letting the OS finish
    // the activity would tear down the native runtime and kill playback.
    @Deprecated("Routed to the in-app router instead of finishing the activity.")
    @Suppress("OVERRIDE_DEPRECATION", "DEPRECATION")
    override fun onBackPressed() {
        MediaReceiver.nativeOnAction("back")
    }

    private var webView: android.webkit.WebView? = null

    /**
     * Dioxus only polls its futures after the WebView acknowledges the previous
     * render, and that JS runs in the WebView's sandboxed renderer *process* — a
     * separate process our foreground service does not protect. By default the
     * WebView waives the renderer's priority whenever it is not visible, so about
     * a minute after backgrounding the cached-app freezer freezes the renderer:
     * no more edit acks, every Rust task stalls, notification taps queue up and
     * the track never auto-advances until the app is reopened. Keeping the
     * priority at IMPORTANT with waiving disabled binds the renderer to this
     * process's (service-protected) importance so it stays runnable.
     */
    override fun onWebViewCreate(webView: android.webkit.WebView) {
        this.webView = webView
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            webView.setRendererPriorityPolicy(
                android.webkit.WebView.RENDERER_PRIORITY_IMPORTANT,
                false
            )
        }
    }

    // WryActivity.onPause() suspends the WebView, which stops its JavaScript. Dioxus
    // only polls its futures once the WebView has acknowledged the previous render
    // (see poll_edits_flushed in dioxus-desktop), so a suspended WebView freezes
    // every task in the app: notification buttons queue up undelivered and a track
    // ending never advances the queue, all while audio keeps playing on the engine's
    // own threads. Resuming it right back keeps that loop turning in the background.
    override fun onPause() {
        super.onPause()
        webView?.onResume()
        webView?.resumeTimers()
        MediaReceiver.nativeOnAction("bg-enter")
    }

    override fun onResume() {
        super.onResume()
        MediaReceiver.nativeOnAction("bg-exit")
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    companion object {
        @Volatile
        private var instance: MainActivity? = null

        // Called from Rust (systemint::move_task_to_back) when back is pressed at the
        // root route. Marshals onto the UI thread because moveTaskToBack touches the
        // activity from a JNI/worker thread otherwise.
        @JvmStatic
        fun moveToBack() {
            val act = instance ?: return
            act.runOnUiThread { act.moveTaskToBack(true) }
        }
    }

    /**
     * Returns whether a permission dialog was actually launched. One dialog at
     * a time: firing the notification and media requests back to back lets the
     * second cancel the first on some Android builds, so onCreate requests
     * media immediately only when this launched nothing, and
     * onRequestPermissionsResult chains it after the 1001 result otherwise.
     */
    private fun requestNotificationPermission(): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                ActivityCompat.requestPermissions(
                    this,
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    1001
                )
                return true
            }
        }
        return false
    }

    // Without this the library scan finds nothing: the manifest entry alone does not
    // grant read access to shared storage, and the scanner has no way to ask for it
    // from the Rust side. Tiramisu split the media permissions out of READ_EXTERNAL_STORAGE.
    private fun requestMediaPermission() {
        val needed = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(Manifest.permission.READ_MEDIA_AUDIO)
        } else {
            arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE)
        }.filter {
            ContextCompat.checkSelfPermission(this, it) != PackageManager.PERMISSION_GRANTED
        }
        if (needed.isNotEmpty()) {
            ActivityCompat.requestPermissions(this, needed.toTypedArray(), 1002)
        }
    }

    override fun onRequestPermissionsResult(
        requestCode: Int,
        permissions: Array<out String>,
        grantResults: IntArray
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == 1001) {
            requestMediaPermission()
            return
        }
        if (requestCode != 1 && requestCode != 1002) return
        val granted = grantResults.isNotEmpty() &&
            grantResults.all { it == PackageManager.PERMISSION_GRANTED }
        MediaReceiver.nativeOnAction(if (granted) "media-granted" else "media-denied")
    }

    private fun requestBatteryOptimizationExemption() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            val pm = getSystemService(POWER_SERVICE) as PowerManager
            if (!pm.isIgnoringBatteryOptimizations(packageName)) {
                try {
                    val intent = Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS).apply {
                        data = Uri.parse("package:$packageName")
                    }
                    startActivity(intent)
                } catch (_: Exception) {}
            }
        }
    }
}
