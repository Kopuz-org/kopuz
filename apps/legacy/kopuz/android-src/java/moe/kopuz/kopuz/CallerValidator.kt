package moe.kopuz.kopuz

import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.Process
import java.security.MessageDigest

/**
 * Who may browse the library and drive the session through
 * [KopuzBrowserService]. A package name alone proves nothing (any app can
 * claim one a phone does not have installed), so a known caller must also be
 * signed with that app's published release key. The keys are the ones
 * Google's reference media app (UAMP) pins in allowed_media_browser_callers.xml.
 *
 * Kopuz itself, the system, and anything signed with the platform key (the
 * media center on Android Automotive) are trusted as is.
 */
class CallerValidator(context: Context) {
    private val pm = context.packageManager
    private val platformKeys: Set<String> = signingKeys("android")
    private val verdicts = HashMap<Pair<String, Int>, Boolean>()

    @Synchronized
    fun isTrusted(packageName: String, uid: Int): Boolean =
        verdicts.getOrPut(packageName to uid) { check(packageName, uid) }

    private fun check(packageName: String, uid: Int): Boolean {
        if (uid == Process.myUid() || uid == Process.SYSTEM_UID) return true
        // The framework already ties the name to the uid; this keeps that
        // promise local instead of relying on it.
        if (pm.getPackagesForUid(uid)?.contains(packageName) != true) return false
        val keys = signingKeys(packageName)
        if (keys.isEmpty()) return false
        if (platformKeys.isNotEmpty() && keys.any { it in platformKeys }) return true
        val allowed = KNOWN_CALLERS[packageName] ?: return false
        return keys.any { it in allowed }
    }

    /** SHA-256 of each certificate the package is signed with, lower-case hex. */
    @SuppressLint("PackageManagerGetSignatures")
    private fun signingKeys(packageName: String): Set<String> {
        val certificates = try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                val info: PackageInfo =
                    pm.getPackageInfo(packageName, PackageManager.GET_SIGNING_CERTIFICATES)
                val signing = info.signingInfo ?: return emptySet()
                if (signing.hasMultipleSigners()) {
                    signing.apkContentsSigners
                } else {
                    signing.signingCertificateHistory
                }
            } else {
                @Suppress("DEPRECATION")
                pm.getPackageInfo(packageName, PackageManager.GET_SIGNATURES).signatures
            }
        } catch (_: PackageManager.NameNotFoundException) {
            null
        } ?: return emptySet()
        return certificates.map { sha256(it.toByteArray()) }.toSet()
    }

    companion object {
        private fun sha256(bytes: ByteArray): String =
            MessageDigest.getInstance("SHA-256").digest(bytes)
                .joinToString("") { "%02x".format(it) }

        private fun key(colons: String) = colons.replace(":", "").lowercase()

        /** Release signing keys, by package. */
        private val KNOWN_CALLERS: Map<String, Set<String>> = mapOf(
            // Android Auto, current and rotated release keys.
            "com.google.android.projection.gearhead" to setOf(
                key("fd:b0:0c:43:db:de:8b:51:cb:31:2a:a8:1d:3b:5f:a1:77:13:ad:b9:4b:28:f5:98:d7:7f:8e:b8:9d:ac:ee:df"),
                key("1c:a8:dc:c0:be:d3:cb:d8:72:d2:cb:79:12:00:c0:29:2c:a9:97:57:68:a8:2d:67:6b:8b:42:4f:b6:5b:52:95"),
            ),
            // The desktop head unit used to test Android Auto without a car.
            "com.google.android.autosimulator" to setOf(
                key("19:75:b2:f1:71:77:bc:89:a5:df:f3:1f:9e:64:a6:ca:e2:81:a5:3d:c1:d1:d5:9b:1d:14:7f:e1:c8:2a:fa:00"),
            ),
            // Google app, for "play ... on Kopuz".
            "com.google.android.googlequicksearchbox" to setOf(
                key("f0:fd:6c:5b:41:0f:25:cb:25:c3:b5:33:46:c8:97:2f:ae:30:f8:ee:74:11:df:91:04:80:ad:6b:2d:60:db:83"),
            ),
            // Google Assistant on Android Automotive.
            "com.google.android.carassistant" to setOf(
                key("74:B6:FB:F7:10:E8:D9:0D:44:D3:40:12:58:89:B4:23:06:A6:2C:43:79:D0:E5:A6:62:20:E3:A6:8A:BF:90:E2"),
            ),
        )
    }
}
