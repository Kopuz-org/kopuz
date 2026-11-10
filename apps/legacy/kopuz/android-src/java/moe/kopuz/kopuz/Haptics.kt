package moe.kopuz.kopuz

import android.view.HapticFeedbackConstants
import android.webkit.JavascriptInterface
import dev.dioxus.main.MainActivity

/**
 * Lets the WebView ask for system haptic feedback. Going through the view keeps
 * it tied to the user's "touch feedback" setting and needs no VIBRATE permission,
 * which navigator.vibrate would.
 */
class Haptics(private val activity: MainActivity) {
    @JavascriptInterface
    fun perform(kind: String) {
        val constant = when (kind) {
            "long-press" -> HapticFeedbackConstants.LONG_PRESS
            "confirm" -> HapticFeedbackConstants.VIRTUAL_KEY
            else -> HapticFeedbackConstants.CLOCK_TICK
        }
        activity.runOnUiThread {
            activity.window?.decorView?.performHapticFeedback(constant)
        }
    }
}
