package moe.kopuz.kopuz

import android.annotation.SuppressLint
import android.content.res.Configuration
import android.os.Build
import android.webkit.JavascriptInterface
import dev.dioxus.main.MainActivity
import org.json.JSONObject

/** Exposes Android's wallpaper palette to the app's WebView. */
class SystemColors(private val activity: MainActivity) {
    @JavascriptInterface
    fun setEnabled(enabled: Boolean) {
        activity.runOnUiThread { activity.setSystemColorsEnabled(enabled) }
    }

    @SuppressLint("DiscouragedApi")
    @JavascriptInterface
    fun palette(): String {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return "null"
        val resources = activity.resources
        val dark = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK ==
            Configuration.UI_MODE_NIGHT_YES
        val suffix = if (dark) "dark" else "light"
        fun color(name: String): Long? {
            val id = resources.getIdentifier(name, "color", "android")
            return if (id == 0) null else resources.getColor(id, activity.theme).toLong() and 0xffffffffL
        }
        val seed = color("system_accent1_500") ?: return "null"
        val neutral = color("system_neutral2_500") ?: return "null"
        val colors = JSONObject()
        fun role(name: String, fallback: String? = null) {
            val value = color("system_${name}_$suffix") ?: fallback?.let { color(it) }
            if (value != null) colors.put(name, value)
        }
        for ((index, name) in listOf("primary", "secondary", "tertiary").withIndex()) {
            val accent = "system_accent${index + 1}"
            role(name, "${accent}_${if (dark) 200 else 600}")
            role("on_$name", "${accent}_${if (dark) 800 else 0}")
            role("${name}_container", "${accent}_${if (dark) 700 else 100}")
            role("on_${name}_container", "${accent}_${if (dark) 100 else 900}")
        }
        role("on_surface", "system_neutral1_${if (dark) 100 else 900}")
        role("on_background", "system_neutral1_${if (dark) 100 else 900}")
        role("surface_variant", "system_neutral2_${if (dark) 700 else 100}")
        role("on_surface_variant", "system_neutral2_${if (dark) 200 else 700}")
        role("outline", "system_neutral2_${if (dark) 400 else 500}")
        role("outline_variant", "system_neutral2_${if (dark) 700 else 200}")
        for (name in listOf(
            "surface", "background", "surface_dim", "surface_bright",
            "surface_container_lowest", "surface_container_low", "surface_container",
            "surface_container_high", "surface_container_highest",
            "error", "on_error", "error_container", "on_error_container"
        )) role(name)
        return JSONObject()
            .put("dark", dark)
            .put("seed", seed)
            .put("neutral", neutral)
            .put("colors", colors)
            .toString()
    }
}
