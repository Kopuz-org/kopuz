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
        requestNotificationPermission()
        requestBatteryOptimizationExemption()
    }

    // Draw under the status/navigation bars and make them transparent so the app's
    // dark background extends edge-to-edge instead of the system's gray bar. The web
    // UI already pads with env(safe-area-inset-*), so content stays clear of the bars.
    private fun enableEdgeToEdge() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        window.statusBarColor = Color.TRANSPARENT
        window.navigationBarColor = Color.TRANSPARENT
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

    private fun requestNotificationPermission() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED
            ) {
                ActivityCompat.requestPermissions(
                    this,
                    arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                    1001
                )
            }
        }
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
