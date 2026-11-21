package moe.kopuz.kopuz

import android.media.MediaDescription
import android.media.browse.MediaBrowser.MediaItem
import android.os.Bundle
import android.os.Process
import android.service.media.MediaBrowserService

/**
 * What Android Auto (and Android Automotive's media center) talks to: it lists
 * the library and hands the car the session MediaSessionHelper already drives,
 * so the car's transport controls are the notification's controls.
 *
 * The rows come from Rust over JNI and the library lives in the app process, so
 * until the app has started once there is nothing to answer with; the car then
 * shows the "open Kopuz on your phone" error from [MediaSessionHelper.showNotReady].
 */
class KopuzBrowserService : MediaBrowserService() {

    override fun onCreate() {
        super.onCreate()
        MediaSessionHelper.init(this)
        MediaSessionHelper.token()?.let { sessionToken = it }
    }

    override fun onGetRoot(clientPackageName: String, clientUid: Int, rootHints: Bundle?): BrowserRoot? {
        if (!isTrusted(clientPackageName, clientUid)) return null
        val extras = Bundle().apply {
            putBoolean(SEARCH_SUPPORTED, true)
            putInt(CONTENT_STYLE_BROWSABLE, CONTENT_STYLE_LIST)
            putInt(CONTENT_STYLE_PLAYABLE, CONTENT_STYLE_LIST)
        }
        return BrowserRoot(ROOT, extras)
    }

    override fun onLoadChildren(parentId: String, result: Result<MutableList<MediaItem>>) {
        // The answer waits on the library, which must not block the main thread.
        result.detach()
        Thread {
            result.sendResult(load { nativeChildren(parentId) })
        }.apply { isDaemon = true }.start()
    }

    override fun onSearch(query: String, extras: Bundle?, result: Result<MutableList<MediaItem>>) {
        result.detach()
        Thread {
            result.sendResult(load { nativeSearch(query) })
        }.apply { isDaemon = true }.start()
    }

    private fun load(fetch: () -> String?): MutableList<MediaItem> {
        val encoded = try {
            fetch()
        } catch (_: UnsatisfiedLinkError) {
            // The native library loads with MainActivity; the car woke us first.
            null
        }
        if (encoded == null) {
            MediaSessionHelper.showNotReady()
            return mutableListOf()
        }
        MediaSessionHelper.clearNotReady()
        return decode(encoded)
    }

    private fun isTrusted(packageName: String, uid: Int): Boolean =
        uid == Process.myUid() ||
            uid == Process.SYSTEM_UID ||
            packageName in TRUSTED_CALLERS

    companion object {
        private const val ROOT = "root"

        private const val SEARCH_SUPPORTED = "android.media.browse.SEARCH_SUPPORTED"
        private const val CONTENT_STYLE_BROWSABLE =
            "android.media.browse.CONTENT_STYLE_BROWSABLE_HINT"
        private const val CONTENT_STYLE_PLAYABLE =
            "android.media.browse.CONTENT_STYLE_PLAYABLE_HINT"
        private const val CONTENT_STYLE_LIST = 1

        private const val FIELD_SEP = '\u001f'
        private const val RECORD_SEP = '\u001e'

        /** Android Auto, Automotive's media center, and the assistant's voice search. */
        private val TRUSTED_CALLERS = setOf(
            "com.google.android.projection.gearhead",
            "com.android.car.media",
            "com.google.android.carassistant",
            "com.google.android.googlequicksearchbox",
        )

        /** Rows as encoded by `android_auto.rs`: id, title, subtitle, flags. */
        private fun decode(encoded: String): MutableList<MediaItem> {
            if (encoded.isEmpty()) return mutableListOf()
            return encoded.split(RECORD_SEP).mapNotNull { record ->
                val fields = record.split(FIELD_SEP)
                if (fields.size < 4) return@mapNotNull null
                val description = MediaDescription.Builder()
                    .setMediaId(fields[0])
                    .setTitle(fields[1])
                    .apply { if (fields[2].isNotEmpty()) setSubtitle(fields[2]) }
                    .build()
                var flags = 0
                if ('b' in fields[3]) flags = flags or MediaItem.FLAG_BROWSABLE
                if ('p' in fields[3]) flags = flags or MediaItem.FLAG_PLAYABLE
                MediaItem(description, flags)
            }.toMutableList()
        }

        /** False when the app is not up to take it. */
        @JvmStatic
        fun play(mediaId: String): Boolean = try {
            nativePlay(mediaId)
        } catch (_: UnsatisfiedLinkError) {
            false
        }

        @JvmStatic
        fun playSearch(query: String): Boolean = try {
            nativePlaySearch(query)
        } catch (_: UnsatisfiedLinkError) {
            false
        }

        /** Encoded rows, or null before the app has registered its handler. */
        @JvmStatic
        external fun nativeChildren(parentId: String): String?

        @JvmStatic
        external fun nativeSearch(query: String): String?

        @JvmStatic
        external fun nativePlay(mediaId: String): Boolean

        @JvmStatic
        external fun nativePlaySearch(query: String): Boolean
    }
}
