package com.temidaradev.kopuz

import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.os.PowerManager

class MusicService : Service() {

    private var wakeLock: PowerManager.WakeLock? = null

    companion object {
        private const val EXTRA_PLAYING = "kopuz_playing"

        /** Start/refresh the foreground service, carrying the current playing state. */
        fun update(context: Context, playing: Boolean) {
            try {
                val intent = Intent(context, MusicService::class.java)
                    .putExtra(EXTRA_PLAYING, playing)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    context.startForegroundService(intent)
                } else {
                    context.startService(intent)
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, MusicService::class.java))
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notif = MediaSessionHelper.pendingNotification ?: run {
            stopSelf()
            return START_NOT_STICKY
        }
        // Must call startForeground within 5s of startForegroundService regardless of
        // play state, or the system kills us with a ForegroundServiceDidNotStartInTime.
        try {
            startForeground(MediaSessionHelper.NOTIF_ID, notif)
        } catch (e: Exception) {
            e.printStackTrace()
        }

        // Held for the whole session, not just while decoding — the macOS side takes
        // an IOKit no-idle-sleep assertion for the same reason. With the screen off
        // and no lock, the CPU suspends: the loop that drains transport commands and
        // advances the queue stops getting timer ticks, so the app goes deaf until
        // something else happens to wake the device. Released on stopSession/onDestroy.
        acquireWakeLock()

        return START_STICKY
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "kopuz::MusicWakeLock").also {
            it.setReferenceCounted(false)
            it.acquire()
        }
    }

    private fun releaseWakeLock() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Keep the service alive when the user swipes the app from recents.
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        releaseWakeLock()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        super.onDestroy()
    }
}
