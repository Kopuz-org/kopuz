package moe.kopuz.kopuz

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.webkit.CookieManager
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Full-screen in-app browser for provider sign-ins (YT Music, SoundCloud).
 * Desktop harvests cookies from a spawned system browser's profile; Android has
 * no such thing, so this WebView plays that role: its cookies land in the
 * app-global [CookieManager], where the Rust side polls for the auth cookies
 * and closes this activity once they appear. Back closes it, which the poller
 * reports as a cancelled sign-in.
 */
class LoginActivity : Activity() {
    private var web: WebView? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        instance = this
        val url = intent.getStringExtra(EXTRA_URL)
        if (url == null) {
            finish()
            return
        }
        val view = WebView(this)
        web = view
        view.settings.javaScriptEnabled = true
        view.settings.domStorageEnabled = true
        view.settings.userAgentString = MOBILE_UA
        val cookies = CookieManager.getInstance()
        cookies.setAcceptCookie(true)
        cookies.setAcceptThirdPartyCookies(view, true)
        view.webViewClient = WebViewClient()
        setContentView(view)
        view.loadUrl(url)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        web?.destroy()
        web = null
        super.onDestroy()
    }

    companion object {
        private const val EXTRA_URL = "kopuz_login_url"

        /**
         * Google refuses account sign-in when the UA reveals an embedded
         * WebView ("this browser or app may not be secure"), so the view
         * presents itself as plain mobile Chrome.
         */
        private const val MOBILE_UA =
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 " +
                "(KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36"

        @Volatile
        private var instance: LoginActivity? = null

        /**
         * Clears all WebView cookies first so a stale half-session can't
         * satisfy the poller before the user has actually signed in. Runs on
         * the main looper: JNI calls arrive on plain Rust threads, and
         * [CookieManager.removeAllCookies] refuses to run without a Looper.
         */
        @JvmStatic
        fun open(context: Context, url: String) {
            android.os.Handler(android.os.Looper.getMainLooper()).post {
                CookieManager.getInstance().removeAllCookies {
                    val intent = Intent(context, LoginActivity::class.java)
                        .putExtra(EXTRA_URL, url)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                    context.startActivity(intent)
                }
            }
        }

        @JvmStatic
        fun cookies(url: String): String? {
            val manager = CookieManager.getInstance()
            manager.flush()
            return manager.getCookie(url)
        }

        @JvmStatic
        fun isOpen(): Boolean = instance != null

        @JvmStatic
        fun close() {
            val act = instance ?: return
            act.runOnUiThread { act.finish() }
        }
    }
}
