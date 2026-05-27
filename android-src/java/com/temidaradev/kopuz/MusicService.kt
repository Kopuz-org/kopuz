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
        // Default to playing if the extra is missing (e.g. a sticky restart).
        val playing = intent?.getBooleanExtra(EXTRA_PLAYING, true) ?: true

        // Must call startForeground within 5s of startForegroundService regardless of
        // play state, or the system kills us with a ForegroundServiceDidNotStartInTime.
        try {
            startForeground(MediaSessionHelper.NOTIF_ID, notif)
        } catch (e: Exception) {
            e.printStackTrace()
        }

        if (playing) {
            acquireWakeLock()
        } else {
            // Paused: drop the CPU wake lock (battery) and detach foreground so the
            // notification becomes dismissible and the OS can reclaim the service —
            // but keep the notification visible via STOP_FOREGROUND_DETACH.
            releaseWakeLock()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
                stopForeground(STOP_FOREGROUND_DETACH)
            } else {
                @Suppress("DEPRECATION")
                stopForeground(false)
            }
        }

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
