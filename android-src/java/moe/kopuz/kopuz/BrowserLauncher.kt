package moe.kopuz.kopuz

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri

object BrowserLauncher {
    @JvmStatic
    fun open(context: Context, url: String, packageName: String): String? {
        val uri = Uri.parse(url)
        if (uri.scheme != "https" && uri.scheme != "http") {
            return "The sign-in address must use HTTP or HTTPS"
        }
        val intent = Intent(Intent.ACTION_VIEW, uri)
            .addCategory(Intent.CATEGORY_BROWSABLE)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            .setPackage(packageName)
        return try {
            context.startActivity(intent)
            null
        } catch (_: ActivityNotFoundException) {
            "The selected browser is not installed or cannot open the sign-in address"
        } catch (_: SecurityException) {
            "Android did not allow the selected browser to open"
        }
    }
}
